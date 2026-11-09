//! The Doctor: independent checks that explain, in plain English, why the
//! machine is or is not online and what to do about it.
//!
//! `gather` asks the Backend and the host ([`host`]); [`assess`] turns the
//! answers into findings. Nothing here contacts a server: names are looked up
//! only for the host of the configured connectivity check, and the gateway is
//! on the local network.

mod assess;
mod fix;
mod host;
mod render;

use std::path::Path;
use std::sync::Mutex;

use nexus_ipc::{Connectivity, DoctorReport, Event, Finding, State, Status};
use tokio::sync::broadcast;

use crate::backend::Backend;
use crate::config::Settings;
use assess::Inputs;

pub use fix::run_fix;
pub use render::render;

const RFKILL_DIR: &str = "/sys/class/rfkill";
const ROUTE_TABLE: &str = "/proc/net/route";
const OS_RELEASE: &str = "/etc/os-release";

/// The last report, so Front ends hear about a *change* once.
#[derive(Debug)]
pub struct Memory {
    last: Mutex<Option<Vec<Finding>>>,
    events: broadcast::Sender<Event>,
}

impl Memory {
    pub fn new(events: broadcast::Sender<Event>) -> Self {
        Self {
            last: Mutex::new(None),
            events,
        }
    }

    fn remember(&self, findings: &[Finding]) {
        let mut last = self.last.lock().expect("doctor memory");
        if last.as_deref() != Some(findings) {
            *last = Some(findings.to_vec());
            let _ = self.events.send(Event::DoctorFindings {
                findings: findings.to_vec(),
            });
        }
    }
}

/// Runs every check and reports.
pub async fn run<B: Backend>(backend: &B, settings: &Settings, memory: &Memory) -> DoctorReport {
    let findings = check(backend, settings).await;
    memory.remember(&findings);
    DoctorReport { findings }
}

/// Runs every check without telling anyone.
async fn check<B: Backend>(backend: &B, settings: &Settings) -> Vec<Finding> {
    let status = backend
        .status()
        .await
        .unwrap_or_else(|_| Status::unavailable());
    let nm = if status.state == State::Unavailable {
        None
    } else {
        backend.net_facts().await.ok()
    };
    let (config_on, url) = connectivity_config(settings);
    let check_off = !config_on || nm.as_ref().is_some_and(|n| !n.connectivity_check);
    // A Portal answers names its own way, and without a Link there is nothing
    // to probe: the finding for that explains more than a failed lookup would.
    let probe = matches!(
        status.state,
        State::Unavailable | State::Unknown | State::Local | State::Site | State::Online
    ) && status.connectivity != Connectivity::Portal;
    let gateway = match gateway_setting(settings).as_str() {
        "off" => None,
        "auto" => match &nm {
            Some(nm) => nm.gateway4.clone(),
            None => std::fs::read_to_string(ROUTE_TABLE)
                .ok()
                .and_then(|t| host::default_gateway(&t)),
        },
        pinned => Some(pinned.to_owned()),
    };
    let target = (probe && !check_off)
        .then(|| host::url_host(&url).map(str::to_owned))
        .flatten();
    let (units, dns, gateway) = tokio::join!(
        host::units(),
        async {
            match target {
                Some(h) => {
                    let result = host::resolve(&h).await;
                    Some((h, result))
                }
                None => None,
            }
        },
        async {
            match gateway.filter(|_| probe) {
                Some(g) => {
                    let result = host::gateway_answers(&g).await;
                    Some((g, result))
                }
                None => None,
            }
        }
    );
    let radio = if nm.is_none() {
        host::wifi_radio(Path::new(RFKILL_DIR))
    } else {
        None
    };
    assess::assess(&Inputs {
        status,
        nm,
        units,
        radio,
        check_off,
        dns,
        gateway,
        distro: distro(),
    })
}

/// `doctor.gateway` of the live config.
fn gateway_setting(settings: &Settings) -> String {
    settings
        .get(Some("doctor.gateway"))
        .ok()
        .and_then(|v| v["value"].as_str().map(str::to_owned))
        .unwrap_or_else(|| "auto".into())
}

/// `(enabled, url)` of the connectivity check in the live config.
fn connectivity_config(settings: &Settings) -> (bool, String) {
    let value = settings
        .get(Some("connectivity"))
        .map(|v| v["value"].clone())
        .unwrap_or_default();
    (
        value["enabled"].as_bool().unwrap_or(true),
        value["url"].as_str().unwrap_or_default().to_owned(),
    )
}

/// The lower-case `ID` and `ID_LIKE` words of the running distribution.
fn distro() -> Vec<String> {
    let text = std::fs::read_to_string(OS_RELEASE).unwrap_or_default();
    text.lines()
        .filter_map(|l| l.strip_prefix("ID_LIKE=").or_else(|| l.strip_prefix("ID=")))
        .flat_map(|v| {
            v.trim_matches('"')
                .split_whitespace()
                .map(str::to_lowercase)
                .collect::<Vec<_>>()
        })
        .collect()
}
