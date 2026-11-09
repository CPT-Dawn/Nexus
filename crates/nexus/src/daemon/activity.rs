//! The activity log: what just happened to the network, in plain English.
//! Entries come from comparing consecutive statuses.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use nexus_ipc::{ActivityEntry, ActivityKind, Connectivity, Link, LinkKind, Status};

/// The log keeps this many entries; the strip shows far fewer.
const KEEP: usize = 20;

#[derive(Debug, Default)]
pub struct Log {
    entries: Mutex<VecDeque<ActivityEntry>>,
}

impl Log {
    /// Newest first.
    pub fn entries(&self) -> Vec<ActivityEntry> {
        self.entries.lock().expect("log").iter().cloned().collect()
    }

    /// Compares two statuses, records what changed and returns the new
    /// entries, oldest first.
    pub fn observe(&self, before: &Status, after: &Status) -> Vec<ActivityEntry> {
        let at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let new: Vec<ActivityEntry> = diff(before, after)
            .into_iter()
            .map(|(kind, text)| ActivityEntry { at, kind, text })
            .collect();
        self.push(&new);
        new
    }

    /// Records a line that does not come from a status change (a VPN going
    /// up or down) and returns the entry, unless the newest line already
    /// says the same (a VPN that is also the primary Link is seen twice).
    pub fn record(&self, kind: ActivityKind, text: String) -> Option<ActivityEntry> {
        if self
            .entries
            .lock()
            .expect("log")
            .front()
            .is_some_and(|e| e.text == text)
        {
            return None;
        }
        let at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let entry = ActivityEntry { at, kind, text };
        self.push(std::slice::from_ref(&entry));
        Some(entry)
    }

    fn push(&self, new: &[ActivityEntry]) {
        let mut log = self.entries.lock().expect("log");
        for entry in new {
            log.push_front(entry.clone());
        }
        log.truncate(KEEP);
    }
}

fn is_vpn(link: &Link) -> bool {
    matches!(link.kind, LinkKind::Vpn | LinkKind::Wireguard)
}

fn kind_name(link: &Link) -> &'static str {
    match link.kind {
        LinkKind::Wifi => "Wi-Fi",
        LinkKind::Ethernet => "Ethernet",
        _ => "network",
    }
}

fn joined(link: &Link) -> (ActivityKind, String) {
    if is_vpn(link) {
        (ActivityKind::VpnUp, format!("VPN {} is up", link.name))
    } else {
        (
            ActivityKind::Joined,
            format!("Joined {} ({})", link.name, kind_name(link)),
        )
    }
}

fn lost(link: &Link) -> (ActivityKind, String) {
    if is_vpn(link) {
        (
            ActivityKind::VpnDown,
            format!("VPN {} went down", link.name),
        )
    } else {
        (ActivityKind::Lost, format!("Lost {}", link.name))
    }
}

fn diff(before: &Status, after: &Status) -> Vec<(ActivityKind, String)> {
    let mut out = Vec::new();
    match (&before.link, &after.link) {
        (None, Some(l)) => out.push(joined(l)),
        // A VPN that stops being the primary Link is reported from the VPN
        // list, which also knows whether anyone asked for it.
        (Some(l), None) if is_vpn(l) => {}
        (Some(l), None) => out.push(lost(l)),
        (Some(a), Some(b)) if a != b => out.push(joined(b)),
        _ => {}
    }
    if let Some(l) = &after.link {
        let (was, is) = (before.connectivity, after.connectivity);
        if is == Connectivity::Portal && was != Connectivity::Portal {
            out.push((
                ActivityKind::Portal,
                format!("Sign-in page detected on {}", l.name),
            ));
        } else if was == Connectivity::Portal && is == Connectivity::Full {
            out.push((
                ActivityKind::Online,
                format!("Signed in to {}, you're online", l.name),
            ));
        }
    }
    out
}
