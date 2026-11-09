//! Trust: the marks the user puts on Networks, and the policy that runs when
//! an Untrusted Network is joined.
//!
//! A mark is the user's word and is kept in `$XDG_STATE_HOME/nexus/trust.json`
//! (the Daemon's own file, not the config, which people edit and comment).
//! A Network nobody marked is Untrusted when it is open, Trusted otherwise.

use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::sync::Mutex;

use std::sync::Arc;

use nexus_ipc::{ActivityKind, Connectivity, Event, Security, Status, Trust, WifiList, WifiNetwork};

use super::{Live, notify};
use crate::backend::Backend;
use crate::config::TrustConfig;

/// `$XDG_STATE_HOME/nexus/trust.json`, else `~/.local/state/nexus/trust.json`.
pub fn default_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|v| !v.is_empty())
                .map(|h| PathBuf::from(h).join(".local/state"))
        })?;
    Some(base.join("nexus").join("trust.json"))
}

/// What the user marked, by `WifiNetwork.id`.
#[derive(Debug, Default)]
pub struct Marks {
    path: Option<PathBuf>,
    marks: Mutex<HashMap<String, Trust>>,
}

impl Marks {
    /// Reads the file; a missing or broken one means no marks.
    pub fn load(path: Option<PathBuf>) -> Self {
        let marks = path
            .as_deref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        Self {
            path,
            marks: Mutex::new(marks),
        }
    }

    /// The default for a Network nobody marked.
    pub fn default_for(security: Security) -> Trust {
        if security == Security::Open {
            Trust::Untrusted
        } else {
            Trust::Trusted
        }
    }

    pub fn trust_of(&self, network: &WifiNetwork) -> Trust {
        self.marks
            .lock()
            .expect("marks")
            .get(&network.id)
            .copied()
            .unwrap_or_else(|| Self::default_for(network.security))
    }

    /// Fills in `trust` on every Network in the list.
    pub fn apply(&self, list: &mut WifiList) {
        for n in &mut list.networks {
            n.trust = self.trust_of(n);
        }
    }

    /// Remembers a mark and writes the file.
    pub fn set(&self, id: &str, trust: Trust) -> io::Result<()> {
        let mut marks = self.marks.lock().expect("marks");
        marks.insert(id.to_owned(), trust);
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_string_pretty(&*marks).expect("marks serialize");
        // Written whole beside the file and renamed over it, so a crash never
        // leaves half a file.
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, path)
    }
}

/// The Untrusted Network the machine is joined to, if any.
pub fn untrusted_join(list: &WifiList) -> Option<&WifiNetwork> {
    list.networks
        .iter()
        .find(|n| n.joined && n.trust == Trust::Untrusted)
}

/// Whether the Link can carry the policy's work: it is past any Portal login.
/// With the connectivity check switched off nobody can tell, so a Link is
/// taken at its word.
pub fn link_ready(status: &Status, check_enabled: bool) -> bool {
    if status.link.is_none() {
        return false;
    }
    match status.connectivity {
        Connectivity::Full | Connectivity::Limited => true,
        Connectivity::Portal => false,
        _ => !check_enabled,
    }
}

/// Does what the config asks for on a freshly joined Untrusted Network, then
/// says so (activity line, notification).
pub async fn run_policy<B: Backend>(
    backend: Arc<B>,
    live: Arc<Live>,
    notifier: notify::Notifier,
    id: String,
    ssid: String,
    config: TrustConfig,
) {
    let mut done = Vec::new();
    if !config.auto_vpn.is_empty() {
        done.push(bring_up(&*backend, &config.auto_vpn).await);
    }
    if config.metered {
        done.push(match backend.wifi_metered(id, true).await {
            Ok(()) => "Marked the network metered.".to_owned(),
            Err(e) => format!("Could not mark the network metered: {e}"),
        });
    }
    let body = done.join(" ");
    let mut chars = body.chars();
    let lower: String = chars
        .next()
        .map(|c| c.to_lowercase().chain(chars).collect())
        .unwrap_or_default();
    if let Some(entry) = live
        .activity
        .record(ActivityKind::Policy, format!("{ssid} is untrusted: {lower}"))
    {
        let _ = live.events.send(Event::Activity { entry });
    }
    notifier.policy(&ssid, &body);
}

/// Brings the configured VPN (a name or a Profile UUID) up, unless it is.
async fn bring_up<B: Backend>(backend: &B, wanted: &str) -> String {
    let list = match backend.vpn_list().await {
        Ok(list) => list,
        Err(e) => return format!("Could not bring up the VPN {wanted}: {e}"),
    };
    let Some(vpn) = list
        .vpns
        .iter()
        .find(|v| v.id == wanted || v.name == wanted)
    else {
        return format!("Could not bring up the VPN {wanted}: no VPN has that name.");
    };
    if vpn.state == nexus_ipc::VpnState::Up {
        return format!("The VPN {} was already up.", vpn.name);
    }
    match backend.vpn_up(vpn.id.clone()).await {
        Ok(_) => format!("Brought up the VPN {}.", vpn.name),
        Err(e) => format!("Could not bring up the VPN {}: {e}", vpn.name),
    }
}

/// Remembers which join the policy already handled, so it runs once per join.
#[derive(Debug, Default)]
pub struct Policy {
    handled: Option<String>,
}

impl Policy {
    /// A Daemon that starts while joined does not treat that as a join.
    pub fn starting(current: Option<&WifiNetwork>) -> Self {
        Self {
            handled: current.map(|n| n.id.clone()),
        }
    }

    /// Looks at the Untrusted Network joined now (if any) and whether the
    /// Link is ready: past any Portal login. Returns the Network the policy
    /// must run for, once per join.
    pub fn step<'a>(
        &mut self,
        current: Option<&'a WifiNetwork>,
        ready: bool,
    ) -> Option<&'a WifiNetwork> {
        let Some(net) = current else {
            self.handled = None;
            return None;
        };
        if self.handled.as_deref() == Some(&net.id) || !ready {
            return None;
        }
        self.handled = Some(net.id.clone());
        Some(net)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_ipc::Band;

    fn net(id: &str, security: Security, joined: bool) -> WifiNetwork {
        WifiNetwork {
            id: id.into(),
            ssid: id.into(),
            signal: 50,
            security,
            band: Band::Ghz5,
            access_points: 1,
            joined,
            device: "wlan0".into(),
            profile: None,
            autoconnect: None,
            trust: Trust::default(),
        }
    }

    #[test]
    fn unmarked_open_networks_are_untrusted_and_the_rest_trusted() {
        let marks = Marks::default();
        assert_eq!(
            marks.trust_of(&net("a", Security::Open, false)),
            Trust::Untrusted
        );
        assert_eq!(
            marks.trust_of(&net("b", Security::Wpa2, false)),
            Trust::Trusted
        );
        assert_eq!(
            marks.trust_of(&net("c", Security::Owe, false)),
            Trust::Trusted
        );
    }

    #[test]
    fn a_mark_beats_the_default_and_survives_a_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("trust.json");
        let marks = Marks::load(Some(path.clone()));
        marks.set("a", Trust::Trusted).unwrap();
        let again = Marks::load(Some(path));
        assert_eq!(
            again.trust_of(&net("a", Security::Open, false)),
            Trust::Trusted
        );
    }

    #[test]
    fn a_broken_file_means_no_marks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("trust.json");
        std::fs::write(&path, "{ nope").unwrap();
        let marks = Marks::load(Some(path));
        assert_eq!(
            marks.trust_of(&net("a", Security::Open, false)),
            Trust::Untrusted
        );
    }

    #[test]
    fn the_policy_runs_once_per_join_and_only_when_ready() {
        let a = net("a", Security::Open, true);
        let mut policy = Policy::default();
        assert!(policy.step(Some(&a), false).is_none(), "behind a Portal");
        assert!(policy.step(Some(&a), true).is_some(), "Portal cleared");
        assert!(policy.step(Some(&a), true).is_none(), "once");
        assert!(policy.step(None, true).is_none(), "left");
        assert!(policy.step(Some(&a), true).is_some(), "joined again");
    }

    #[test]
    fn a_daemon_that_starts_joined_does_not_count_it_as_a_join() {
        let a = net("a", Security::Open, true);
        let mut policy = Policy::starting(Some(&a));
        assert!(policy.step(Some(&a), true).is_none());
    }
}
