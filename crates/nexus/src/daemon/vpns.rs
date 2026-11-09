//! Turning changes in the VPN list into activity lines and drop alerts.

use std::collections::HashSet;
use std::sync::Mutex;

use nexus_ipc::{ActivityKind, VpnList, VpnState};

/// What happened to the VPNs between two reads.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Changes {
    /// Activity lines, oldest first.
    pub activity: Vec<(ActivityKind, String)>,
    /// Names of VPNs that went down although nobody asked.
    pub dropped: Vec<String>,
}

/// VPNs a Front end asked to take down, so their going down is no surprise.
#[derive(Debug, Default)]
pub struct Asked(Mutex<HashSet<String>>);

impl Asked {
    pub fn down(&self, id: &str) {
        self.0.lock().expect("asked").insert(id.to_owned());
    }

    pub fn forget(&self, id: &str) {
        self.0.lock().expect("asked").remove(id);
    }

    /// Compares two lists. A VPN that was up and still has a Profile but is
    /// not up any more is a drop unless it was asked to go down. A Profile
    /// that disappeared was deleted on purpose: no alert.
    pub fn observe(&self, before: &VpnList, after: &VpnList) -> Changes {
        let mut asked = self.0.lock().expect("asked");
        let mut changes = Changes::default();
        for now in &after.vpns {
            let was = before.vpns.iter().find(|v| v.id == now.id).map(|v| v.state);
            match (was, now.state) {
                (Some(VpnState::Up), VpnState::Up) => {}
                (_, VpnState::Up) => changes
                    .activity
                    .push((ActivityKind::VpnUp, format!("VPN {} is up", now.name))),
                (Some(VpnState::Up), VpnState::Down) => {
                    if asked.remove(&now.id) {
                        changes
                            .activity
                            .push((ActivityKind::VpnDown, format!("VPN {} is down", now.name)));
                    } else {
                        changes
                            .activity
                            .push((ActivityKind::VpnDown, format!("VPN {} dropped", now.name)));
                        changes.dropped.push(now.name.clone());
                    }
                }
                (Some(VpnState::Activating), VpnState::Down) => {
                    asked.remove(&now.id);
                }
                _ => {}
            }
        }
        changes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_ipc::{Vpn, VpnKind};

    fn list(state: VpnState) -> VpnList {
        VpnList {
            vpns: vec![Vpn {
                id: "a".into(),
                name: "Work".into(),
                kind: VpnKind::Wireguard,
                state,
                addresses: vec![],
                endpoint: None,
            }],
        }
    }

    #[test]
    fn an_unasked_fall_is_a_drop() {
        let asked = Asked::default();
        let c = asked.observe(&list(VpnState::Up), &list(VpnState::Down));
        assert_eq!(c.dropped, ["Work"]);
        assert_eq!(c.activity[0].1, "VPN Work dropped");
    }

    #[test]
    fn an_asked_fall_is_quiet_once() {
        let asked = Asked::default();
        asked.down("a");
        let c = asked.observe(&list(VpnState::Up), &list(VpnState::Down));
        assert!(c.dropped.is_empty());
        assert_eq!(c.activity[0].1, "VPN Work is down");
        // The next fall is a surprise again.
        let c = asked.observe(&list(VpnState::Up), &list(VpnState::Down));
        assert_eq!(c.dropped, ["Work"]);
    }

    #[test]
    fn a_deleted_profile_is_not_a_drop() {
        let c = Asked::default().observe(&list(VpnState::Up), &VpnList::default());
        assert_eq!(c, Changes::default());
    }

    #[test]
    fn coming_up_is_logged() {
        let c = Asked::default().observe(&list(VpnState::Down), &list(VpnState::Up));
        assert_eq!(c.activity, [(ActivityKind::VpnUp, "VPN Work is up".into())]);
    }
}
