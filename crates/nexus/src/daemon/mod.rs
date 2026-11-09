//! The Daemon: owns the socket and the Backend, answers Front ends.

mod activity;
mod notify;
mod socket;
mod traffic;
mod vpns;

use std::process::ExitCode;
use std::sync::Arc;

use nexus_ipc::{ErrorCode, Event, Op, Request, Response, Status, VpnList, WifiList};

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
    let live = Arc::new(Live::new(events.clone()));
    // Listen first, so nothing between the baseline and the first wait is lost.
    backend.arm().await;
    // The baseline is read before the socket exists, so anything a client
    // can see happening is a change from it (and gets an activity entry).
    let baseline = tokio::time::timeout(BASELINE_WAIT, async {
        (
            backend.wifi_list().await.ok(),
            backend.status().await.ok(),
            backend.vpn_list().await.ok(),
        )
    })
    .await
    .unwrap_or_default();
    tokio::spawn({
        let backend = backend.clone();
        async move { backend.secrets().await }
    });
    let frontends = notify::Frontends::default();
    let notifier = notify::Notifier::spawn(
        settings.clone(),
        &broker,
        frontends.clone(),
        Arc::new(notify::SystemTerminal),
    );
    tokio::spawn(watch_backend(
        backend.clone(),
        events.clone(),
        live.clone(),
        notifier.clone(),
        baseline,
    ));
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
                let live = live.clone();
                let broker = broker.clone();
                let frontends = frontends.clone();
                let notifier = notifier.clone();
                tokio::spawn(async move {
                    let subscribe = || {
                        let (rx, backlog) = broker.subscribe();
                        (rx, backlog, frontends.join())
                    };
                    if let Err(e) = socket::serve_connection(stream, subscribe, |req| answer(&*backend, &settings, &broker, &live, &notifier, req)).await {
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

/// What subscribers were last told.
#[derive(Default)]
struct Seen {
    status: Option<Status>,
    wifi: Option<WifiList>,
    vpns: Option<VpnList>,
}

/// How [`sync`] treats what it reads.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Push {
    /// Tell subscribers only what differs from before.
    Changes,
    /// Tell subscribers everything (after a resume, when nothing can be trusted).
    All,
}

/// Re-reads the Backend and pushes `status_changed` and `networks_changed`.
/// A Backend that is gone reads as an unavailable status and an empty list, so
/// Front ends learn about the outage and again about the return.
async fn sync<B: Backend>(
    backend: &B,
    events: &broadcast::Sender<Event>,
    live: &Live,
    notifier: &notify::Notifier,
    seen: &mut Seen,
    push: Push,
) {
    match backend.status().await {
        Ok(status) => {
            // The log entry goes out first, so a Front end that shows the new
            // status already has the line that explains it.
            if let Some(before) = &seen.status {
                for entry in live.activity.observe(before, &status) {
                    let _ = events.send(Event::Activity { entry });
                }
            }
            if push == Push::All || (push == Push::Changes && seen.status.as_ref() != Some(&status))
            {
                let _ = events.send(Event::StatusChanged {
                    status: status.clone(),
                });
            }
            seen.status = Some(status);
        }
        Err(e) => tracing::debug!("status unavailable: {e}"),
    }
    let list = match backend.wifi_list().await {
        Ok(list) => list,
        Err(BackendError::Unavailable) => WifiList::default(),
        Err(e) => {
            tracing::debug!("wifi list unavailable: {e}");
            return;
        }
    };
    if push == Push::All || (push == Push::Changes && seen.wifi.as_ref() != Some(&list)) {
        let _ = events.send(Event::NetworksChanged { wifi: list.clone() });
    }
    seen.wifi = Some(list);
    sync_vpns(backend, events, live, notifier, seen, push).await;
}

/// Pushes `vpns_changed`, logs VPNs coming and going, and sounds the alarm
/// for one that dropped without being asked to. A Backend that is gone
/// leaves the last list standing: its tunnels are judged when it returns.
async fn sync_vpns<B: Backend>(
    backend: &B,
    events: &broadcast::Sender<Event>,
    live: &Live,
    notifier: &notify::Notifier,
    seen: &mut Seen,
    push: Push,
) {
    let Ok(list) = backend.vpn_list().await else {
        return;
    };
    if let Some(before) = &seen.vpns {
        let changes = live.vpns.observe(before, &list);
        for (kind, text) in changes.activity {
            if let Some(entry) = live.activity.record(kind, text) {
                let _ = events.send(Event::Activity { entry });
            }
        }
        for name in changes.dropped {
            notifier.vpn_dropped(&name);
        }
    }
    if push == Push::All || (push == Push::Changes && seen.vpns.as_ref() != Some(&list)) {
        let _ = events.send(Event::VpnsChanged { vpns: list.clone() });
    }
    seen.vpns = Some(list);
}

/// How long startup waits for the Backend's first answers before listening.
const BASELINE_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

/// What the Daemon remembers between requests.
#[derive(Debug)]
struct Live {
    traffic: traffic::Sampler,
    activity: activity::Log,
    doctor: crate::doctor::Memory,
    vpns: vpns::Asked,
}

impl Live {
    fn new(events: broadcast::Sender<Event>) -> Self {
        Self {
            traffic: traffic::Sampler::default(),
            activity: activity::Log::default(),
            doctor: crate::doctor::Memory::new(events),
            vpns: vpns::Asked::default(),
        }
    }
}

/// Keeps subscribers current. The Backend coalesces each burst of changes into
/// one tick, and only one task may wait for ticks, so the Wi-Fi list, the
/// status and the activity log are all fed from here. A tick that changed
/// nothing visible is not worth an event. After the machine wakes from sleep
/// everything is re-read and re-sent, and a fresh scan is asked for because
/// the access points seen before sleeping are stale.
async fn watch_backend<B: Backend>(
    backend: Arc<B>,
    events: broadcast::Sender<Event>,
    live: Arc<Live>,
    notifier: notify::Notifier,
    (wifi, status, vpns): (Option<WifiList>, Option<Status>, Option<VpnList>),
) {
    let mut seen = Seen { status, wifi, vpns };
    loop {
        let push = tokio::select! {
            () = backend.changed() => Push::Changes,
            () = backend.resumed() => {
                tracing::info!("woke from sleep: refreshing");
                Push::All
            }
        };
        sync(&*backend, &events, &live, &notifier, &mut seen, push).await;
        if push == Push::All
            && let Err(e) = backend.wifi_scan().await
        {
            tracing::debug!("no scan after resume: {e}");
        }
    }
}

/// One request in, one response out.
async fn answer<B: Backend>(
    backend: &B,
    settings: &Settings,
    broker: &Broker,
    live: &Live,
    notifier: &notify::Notifier,
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
        Op::Traffic => match backend.counters().await {
            Ok(counters) => Response::ok(
                req.id,
                serde_json::to_value(live.traffic.rates(counters).await)
                    .expect("wire types serialize"),
            ),
            Err(e) => backend_reply::<()>(req.id, Err(e)),
        },
        Op::Activity => Response::ok(
            req.id,
            serde_json::to_value(nexus_ipc::ActivityLog {
                entries: live.activity.entries(),
            })
            .expect("wire types serialize"),
        ),
        Op::Doctor => Response::ok(
            req.id,
            serde_json::to_value(crate::doctor::run(backend, settings, &live.doctor).await)
                .expect("wire types serialize"),
        ),
        Op::DoctorFix { finding, confirm } => {
            match crate::doctor::run_fix(backend, settings, &finding, confirm).await {
                Ok(done) => Response::ok(
                    req.id,
                    serde_json::to_value(done).expect("wire types serialize"),
                ),
                Err(f) => Response::err(Some(req.id), f.code, f.message),
            }
        }
        Op::WifiList => backend_reply(req.id, backend.wifi_list().await),
        Op::WifiScan => backend_reply(req.id, backend.wifi_scan().await),
        Op::WifiJoin { network, password } => {
            let result = backend.wifi_join(network, password).await;
            if let Err(BackendError::Refused(f)) = &result
                && f.code == ErrorCode::WrongPassword
            {
                notifier.wrong_password(&f.message);
            }
            backend_reply(req.id, result)
        }
        Op::VpnList => backend_reply(req.id, backend.vpn_list().await),
        Op::VpnUp { vpn } => backend_reply(req.id, backend.vpn_up(vpn).await),
        Op::VpnDown { vpn } => {
            live.vpns.down(&vpn);
            let result = backend.vpn_down(vpn.clone()).await;
            if result.is_err() {
                live.vpns.forget(&vpn);
            }
            backend_reply(req.id, result)
        }
        Op::VpnImport { name, config } => {
            backend_reply(req.id, backend.vpn_import(name, config).await)
        }
        Op::VpnCreate(tunnel) => backend_reply(req.id, backend.vpn_create(tunnel).await),
        Op::VpnKey { vpn } => backend_reply(req.id, backend.vpn_key(vpn).await),
        Op::VpnReveal { vpn } => backend_reply(req.id, backend.vpn_reveal(vpn).await),
        Op::WifiPower { on } => backend_reply(req.id, backend.wifi_power(on).await),
        Op::Airplane { on } => backend_reply(req.id, backend.airplane(on).await),
        Op::WifiDisconnect => backend_reply(req.id, backend.wifi_disconnect().await),
        Op::WifiForget { network } => backend_reply(req.id, backend.wifi_forget(network).await),
        Op::WifiAutoconnect { network, enabled } => {
            backend_reply(req.id, backend.wifi_autoconnect(network, enabled).await)
        }
        Op::WifiReveal { network } => backend_reply(req.id, backend.wifi_reveal(network).await),
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
