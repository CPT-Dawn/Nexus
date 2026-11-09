//! The Daemon: owns the socket and the Backend, answers Front ends.

mod socket;

use std::process::ExitCode;
use std::sync::Arc;

use nexus_ipc::{ErrorCode, Event, Op, Request, Response};
use tokio::sync::broadcast;
use tracing_subscriber::EnvFilter;

use crate::backend::Backend;
use crate::backend::nm::NetworkManager;
use crate::config::{self, Settings};

/// `nexus daemon`.
pub fn run() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with_writer(std::io::stderr)
        .init();
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            tracing::error!("cannot start the async runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    match rt.block_on(serve(NetworkManager::new())) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!("{e}");
            ExitCode::FAILURE
        }
    }
}

async fn serve<B: Backend>(backend: B) -> std::io::Result<()> {
    let backend = Arc::new(backend);
    let (events, _) = broadcast::channel::<Event>(64);
    let settings = Arc::new(Settings::load(config::default_path(), events.clone()));
    tokio::spawn(config::watch(settings.clone()));
    let listener = socket::listen()?;
    tracing::info!("listening");
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                if !socket::same_user(&stream) {
                    tracing::warn!("refused a connection from another user");
                    continue; // dropped: silence is the answer
                }
                let backend = backend.clone();
                let settings = settings.clone();
                let events = events.clone();
                tokio::spawn(async move {
                    if let Err(e) = socket::serve_connection(stream, events, |req| answer(&*backend, &settings, req)).await {
                        tracing::debug!("connection ended: {e}");
                    }
                });
            }
            _ = tokio::signal::ctrl_c() => break,
            _ = term.recv() => break,
        }
    }
    tracing::info!("shutting down");
    Ok(())
}

/// One request in, one response out.
async fn answer<B: Backend>(backend: &B, settings: &Settings, req: Request) -> Response {
    match req.op {
        Op::Status => match backend.status().await {
            Ok(status) => Response::ok(
                req.id,
                serde_json::to_value(status).expect("status serializes"),
            ),
            Err(e) => Response::err(Some(req.id), ErrorCode::Internal, e.to_string()),
        },
        Op::ConfigGet { key } => reply(req.id, settings.get(key.as_deref())),
        Op::ConfigSet { key, value } => reply(req.id, settings.set(&key, &value)),
        // `Subscribe` is handled by the connection itself.
        // `Op` is non_exhaustive: a newer client op this build cannot serve.
        _ => Response::err(Some(req.id), ErrorCode::UnknownOp, "op not supported"),
    }
}

fn reply(id: u64, result: Result<serde_json::Value, nexus_ipc::Failure>) -> Response {
    match result {
        Ok(v) => Response::ok(id, v),
        Err(f) => Response::err(Some(id), f.code, f.message),
    }
}
