//! The Daemon: owns the socket and the Backend, answers Front ends.

mod activity;
mod notify;
mod portal;
mod profile;
mod socket;
mod traffic;
mod vpns;

use std::process::ExitCode;
use std::sync::Arc;

use nexus_ipc::{
    ErrorCode, EthernetList, Event, Hotspot, Op, Portal, Request, Response, Status, Tethers,
    VpnList, WifiList,
};

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
    let opener: portal::SharedOpener = Arc::new(portal::SystemOpener);
    let live = Arc::new(Live::new(events.clone(), settings.clone(), opener.clone()));
    // Listen first, so nothing between the baseline and the first wait is lost.
    backend.arm().await;
    // The baseline is read before the socket exists, so anything a client
    // can see happening is a change from it (and gets an activity entry).
    let baseline = tokio::time::timeout(BASELINE_WAIT, async {
        (
            backend.wifi_list().await.ok(),
            backend.status().await.ok(),
            backend.tethers().await.ok(),
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
        opener,
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
                    if let Err(e) = socket::serve_connection(stream, subscribe, |req| dispatch(&backend, &settings, &broker, &live, &notifier, req)).await {
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
    tethers: Option<Tethers>,
    ethernet: Option<EthernetList>,
    hotspot: Option<Hotspot>,
    portal: Option<Portal>,
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
    sync_tethers(backend, events, seen, push).await;
    sync_ethernet(backend, events, seen, push).await;
    sync_hotspot(backend, events, seen, push).await;
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
            let portal = portal::of_status(&status, &live.settings);
            if push == Push::All || portal != seen.portal {
                let _ = events.send(Event::PortalChanged {
                    portal: portal.clone(),
                });
            }
            seen.portal = portal;
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

/// Pushes `tethers_changed` when Bluetooth or a phone's Link differs from
/// what subscribers were last told.
async fn sync_tethers<B: Backend>(
    backend: &B,
    events: &broadcast::Sender<Event>,
    seen: &mut Seen,
    push: Push,
) {
    let tethers = match backend.tethers().await {
        Ok(t) => t,
        Err(BackendError::Unavailable) => Tethers::default(),
        Err(e) => {
            tracing::debug!("tethers unavailable: {e}");
            return;
        }
    };
    if push == Push::All || (push == Push::Changes && seen.tethers.as_ref() != Some(&tethers)) {
        let _ = events.send(Event::TethersChanged {
            tethers: tethers.clone(),
        });
    }
    seen.tethers = Some(tethers);
}

/// Pushes `ethernet_changed` when a port's cable, Link or trouble differs
/// from what subscribers were last told. A Backend that is gone reads as no
/// ports, like the Wi-Fi list.
async fn sync_ethernet<B: Backend>(
    backend: &B,
    events: &broadcast::Sender<Event>,
    seen: &mut Seen,
    push: Push,
) {
    let list = match backend.ethernet_list().await {
        Ok(l) => l,
        Err(BackendError::Unavailable) => EthernetList::default(),
        Err(e) => {
            tracing::debug!("ethernet unavailable: {e}");
            return;
        }
    };
    if push == Push::All || (push == Push::Changes && seen.ethernet.as_ref() != Some(&list)) {
        let _ = events.send(Event::EthernetChanged {
            ethernet: list.clone(),
        });
    }
    seen.ethernet = Some(list);
}

/// Whether two pictures say the same about the running Hotspot. Whether the
/// machine *can* host one is left out on purpose: it changes when a Wi-Fi card
/// is plugged in or the radio is switched, which no Front end needs pushed
/// (the TUI asks again when its Hotspot tab opens).
fn same_hotspot(a: &Hotspot, b: &Hotspot) -> bool {
    (a.on, a.starting, &a.ssid, a.devices) == (b.on, b.starting, &b.ssid, b.devices)
}

/// Pushes `hotspot_changed` when the Hotspot or its device count differs from
/// what subscribers were last told.
async fn sync_hotspot<B: Backend>(
    backend: &B,
    events: &broadcast::Sender<Event>,
    seen: &mut Seen,
    push: Push,
) {
    let hotspot = match backend.hotspot().await {
        Ok(h) => h,
        Err(BackendError::Unavailable) => Hotspot::default(),
        Err(e) => {
            tracing::debug!("hotspot unavailable: {e}");
            return;
        }
    };
    let differs = seen
        .hotspot
        .as_ref()
        .is_none_or(|before| !same_hotspot(before, &hotspot));
    if push == Push::All || (push == Push::Changes && differs) {
        let _ = events.send(Event::HotspotChanged {
            hotspot: hotspot.clone(),
        });
    }
    seen.hotspot = Some(hotspot);
}

/// Resolves after `every` while a Hotspot runs, never otherwise: the devices
/// that join it raise no signal, so they are counted on a timer, but only
/// while there is something to count.
async fn hotspot_tick(on: bool, every: std::time::Duration) {
    if on {
        tokio::time::sleep(every).await;
    } else {
        std::future::pending::<()>().await;
    }
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
    settings: Arc<Settings>,
    opener: portal::SharedOpener,
    profile: Arc<profile::Gate>,
}

impl Live {
    fn new(
        events: broadcast::Sender<Event>,
        settings: Arc<Settings>,
        opener: portal::SharedOpener,
    ) -> Self {
        Self {
            settings,
            opener,
            traffic: traffic::Sampler::default(),
            activity: activity::Log::default(),
            profile: Arc::new(profile::Gate::new(events.clone())),
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
    (wifi, status, tethers, vpns): (
        Option<WifiList>,
        Option<Status>,
        Option<Tethers>,
        Option<VpnList>,
    ),
) {
    let portal = status
        .as_ref()
        .and_then(|s| portal::of_status(s, &live.settings));
    // Read here rather than in the baseline: it is not worth a startup wait,
    // and a Backend that is down simply has no first picture.
    let hotspot = backend.hotspot().await.ok();
    let mut seen = Seen {
        status,
        wifi,
        tethers,
        // What a Front end reads on connecting; only changes from it are pushed.
        ethernet: backend.ethernet_list().await.ok(),
        hotspot,
        portal,
        vpns,
    };
    loop {
        let on = seen.hotspot.as_ref().is_some_and(|h| h.on);
        let woke = tokio::select! {
            () = backend.changed() => Some(Push::Changes),
            () = backend.resumed() => {
                tracing::info!("woke from sleep: refreshing");
                Some(Push::All)
            }
            () = hotspot_tick(on, live.settings.hotspot_poll()) => None,
        };
        let Some(push) = woke else {
            sync_hotspot(&*backend, &events, &mut seen, Push::Changes).await;
            continue;
        };
        sync(&*backend, &events, &live, &notifier, &mut seen, push).await;
        if push == Push::All
            && let Err(e) = backend.wifi_scan().await
        {
            tracing::debug!("no scan after resume: {e}");
        }
    }
}

/// Routes a request: Profile settings ops need the shared Backend handle
/// (their timers outlive the request), everything else is [`answer`].
async fn dispatch<B: Backend>(
    backend: &Arc<B>,
    settings: &Settings,
    broker: &Broker,
    live: &Live,
    notifier: &notify::Notifier,
    req: Request,
) -> Response {
    if profile::handles(&req.op) {
        profile::answer(backend, &live.profile, req).await
    } else {
        answer(&**backend, settings, broker, live, notifier, req).await
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
        Op::Portal => {
            let portal = match backend.status().await {
                Ok(status) => portal::of_status(&status, settings),
                Err(e) => return backend_reply::<()>(req.id, Err(e)),
            };
            Response::ok(
                req.id,
                serde_json::to_value(nexus_ipc::PortalState { portal })
                    .expect("wire types serialize"),
            )
        }
        Op::PortalOpen => open_portal(backend, settings, live, req.id).await,
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
        Op::WifiAps { network } => backend_reply(req.id, backend.wifi_aps(network).await),
        Op::WifiJoin {
            network,
            password,
            device,
            hidden,
        } => {
            let target = crate::backend::JoinTarget { device, hidden };
            let result = backend.wifi_join(network, password, target).await;
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
        Op::VpnImport {
            name,
            config,
            plugin,
        } => backend_reply(req.id, backend.vpn_import(name, config, plugin).await),
        Op::VpnCreate(tunnel) => backend_reply(req.id, backend.vpn_create(tunnel).await),
        Op::VpnKey { vpn } => backend_reply(req.id, backend.vpn_key(vpn).await),
        Op::VpnReveal { vpn } => backend_reply(req.id, backend.vpn_reveal(vpn).await),
        Op::WifiJoinEap { network, eap } => {
            let result = backend.wifi_join_eap(network, eap).await;
            if let Err(BackendError::Refused(f)) = &result
                && f.code == ErrorCode::WrongPassword
            {
                notifier.wrong_password(&f.message);
            }
            backend_reply(req.id, result)
        }
        Op::EthernetList => backend_reply(req.id, backend.ethernet_list().await),
        Op::EthernetConnect { port } => backend_reply(req.id, backend.ethernet_connect(port).await),
        Op::EthernetJoinEap { port, eap } => {
            backend_reply(req.id, backend.ethernet_join_eap(port, eap).await)
        }
        Op::EthernetDisconnect { port } => {
            backend_reply(req.id, backend.ethernet_disconnect(port).await)
        }
        Op::Tethers => backend_reply(req.id, backend.tethers().await),
        Op::TetherConnect { phone } => backend_reply(req.id, backend.tether_connect(phone).await),
        Op::TetherDisconnect => backend_reply(req.id, backend.tether_disconnect().await),
        Op::Hotspot => backend_reply(req.id, backend.hotspot().await),
        Op::HotspotStart { ssid, password } => {
            backend_reply(req.id, backend.hotspot_start(ssid, password).await)
        }
        Op::HotspotStop => backend_reply(req.id, backend.hotspot_stop().await),
        Op::HotspotReveal => backend_reply(req.id, backend.hotspot_reveal().await),
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

/// `portal_open`: the login page of the Portal the Link is behind, in the browser.
async fn open_portal<B: Backend>(
    backend: &B,
    settings: &Settings,
    live: &Live,
    id: u64,
) -> Response {
    let status = match backend.status().await {
        Ok(status) => status,
        Err(e) => return backend_reply::<()>(id, Err(e)),
    };
    let Some(portal) = portal::of_status(&status, settings) else {
        return Response::err(
            Some(id),
            ErrorCode::NoPortal,
            "No network is asking for a login right now.",
        );
    };
    match live.opener.open(&portal.url) {
        Ok(()) => Response::ok(
            id,
            serde_json::to_value(nexus_ipc::PortalOpened { url: portal.url })
                .expect("wire types serialize"),
        ),
        Err(message) => Response::err(Some(id), ErrorCode::OpenFailed, message),
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
