//! The Daemon: owns the socket and the Backend, answers Front ends.

mod socket;

use std::process::ExitCode;
use std::sync::Arc;

use nexus_ipc::{ErrorCode, Event, Op, Request, Response};

use crate::backend::BackendError;
use tokio::sync::broadcast;
use tracing_subscriber::EnvFilter;

use crate::backend::Backend;
use crate::backend::nm::NetworkManager;
use crate::config::{self, Settings};
use crate::secrets::Broker;

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
    match rt.block_on(serve()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!("{e}");
            ExitCode::FAILURE
        }
    }
}

async fn serve() -> std::io::Result<()> {
    let (events, _) = broadcast::channel::<Event>(64);
    let settings = Arc::new(Settings::load(config::default_path(), events.clone()));
    let broker = Arc::new(Broker::new(events.clone(), settings.clone()));
    let backend = Arc::new(NetworkManager::new(broker.clone()));
    tokio::spawn(config::watch(settings.clone()));
    tokio::spawn(watch_wifi(backend.clone(), events.clone()));
    tokio::spawn({
        let backend = backend.clone();
        async move { backend.secrets().await }
    });
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
                let broker = broker.clone();
                tokio::spawn(async move {
                    let subscribe = || broker.subscribe();
                    if let Err(e) = socket::serve_connection(stream, subscribe, |req| answer(&*backend, &settings, &broker, req)).await {
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

/// Tells subscribers when the Wi-Fi list changes. The Backend coalesces each
/// burst of changes into one tick; a tick that changed nothing visible is not
/// worth an event.
async fn watch_wifi<B: Backend>(backend: Arc<B>, events: broadcast::Sender<Event>) {
    let mut last = backend.wifi_list().await.ok();
    loop {
        backend.changed().await;
        match backend.wifi_list().await {
            Ok(list) if last.as_ref() != Some(&list) => {
                last = Some(list.clone());
                let _ = events.send(Event::NetworksChanged { wifi: list });
            }
            Ok(_) => {}
            Err(e) => tracing::debug!("wifi list unavailable: {e}"),
        }
    }
}

/// One request in, one response out.
async fn answer<B: Backend>(
    backend: &B,
    settings: &Settings,
    broker: &Broker,
    req: Request,
) -> Response {
    match req.op {
        Op::Status => match backend.status().await {
            Ok(status) => Response::ok(
                req.id,
                serde_json::to_value(status).expect("status serializes"),
            ),
            Err(e) => backend_reply::<()>(req.id, Err(e)),
        },
        Op::WifiList => backend_reply(req.id, backend.wifi_list().await),
        Op::WifiScan => backend_reply(req.id, backend.wifi_scan().await),
        Op::WifiJoin { network, password } => {
            backend_reply(req.id, backend.wifi_join(network, password).await)
        }
        Op::WifiDisconnect => backend_reply(req.id, backend.wifi_disconnect().await),
        Op::WifiForget { network } => backend_reply(req.id, backend.wifi_forget(network).await),
        Op::WifiAutoconnect { network, enabled } => {
            backend_reply(req.id, backend.wifi_autoconnect(network, enabled).await)
        }
        Op::SecretAnswer { prompt, secret } => reply(
            req.id,
            broker
                .answer(prompt, secret)
                .map(|()| serde_json::json!({})),
        ),
        Op::SecretCancel { prompt } => reply(
            req.id,
            broker.cancel(prompt).map(|()| serde_json::json!({})),
        ),
        Op::ConfigGet { key } => reply(req.id, settings.get(key.as_deref())),
        Op::ConfigSet { key, value } => reply(req.id, settings.set(&key, &value)),
        // `Subscribe` is handled by the connection itself.
        // `Op` is non_exhaustive: a newer client op this build cannot serve.
        _ => Response::err(Some(req.id), ErrorCode::UnknownOp, "op not supported"),
    }
}

fn backend_reply<T: serde::Serialize>(id: u64, result: Result<T, BackendError>) -> Response {
    match result {
        Ok(v) => Response::ok(id, serde_json::to_value(v).expect("wire types serialize")),
        Err(BackendError::Unavailable) => Response::err(
            Some(id),
            ErrorCode::BackendUnavailable,
            BackendError::Unavailable.to_string(),
        ),
        Err(BackendError::Refused(f)) => Response::err(Some(id), f.code, f.message),
        Err(e @ BackendError::Failed(_)) => {
            Response::err(Some(id), ErrorCode::Internal, e.to_string())
        }
    }
}

fn reply(id: u64, result: Result<serde_json::Value, nexus_ipc::Failure>) -> Response {
    match result {
        Ok(v) => Response::ok(id, v),
        Err(f) => Response::err(Some(id), f.code, f.message),
    }
}
