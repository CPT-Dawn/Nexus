//! Plain-text rendering of the Wi-Fi list for `nexus wifi`.

use std::fmt::Write as _;

use nexus_ipc::{ApList, ScanOutcome, WifiList};

/// An SSID is untrusted bytes: show control characters as U+FFFD so a
/// hostile name cannot drive the terminal.
fn printable(ssid: &str) -> String {
    ssid.chars()
        .map(|c| if c.is_control() { '\u{fffd}' } else { c })
        .collect()
}

pub fn table(list: &WifiList) -> String {
    let mut out = String::new();
    if list.devices.is_empty() {
        return "No Wi-Fi device found.\n".into();
    }
    if list.networks.is_empty() {
        out.push_str("No Wi-Fi networks in range.\n");
    } else {
        let width = list
            .networks
            .iter()
            .map(|n| printable(&n.ssid).chars().count())
            .max()
            .unwrap_or(4)
            .max(4);
        // With several cards, say which one hears each Network.
        let many = list.devices.len() > 1;
        let card = |device: &str| {
            list.devices
                .iter()
                .find(|d| d.id == device)
                .map_or("", |d| d.name.as_str())
        };
        let _ = writeln!(
            out,
            "{:<width$}  {:>6}  {:<10}  {:<7}  {}STATE",
            "SSID",
            "SIGNAL",
            "SECURITY",
            "BAND",
            if many { "DEVICE  " } else { "" },
        );
        for n in &list.networks {
            let _ = writeln!(
                out,
                "{:<width$}  {:>5}%  {:<10}  {:<7}  {}{}",
                printable(&n.ssid),
                n.signal,
                n.security.label(),
                n.band.label(),
                if many {
                    format!("{:<8}", card(&n.device))
                } else {
                    String::new()
                },
                if n.joined {
                    "connected"
                } else if n.profile.is_some() {
                    "saved"
                } else {
                    ""
                },
            );
        }
    }
    if !list.saved.is_empty() {
        out.push_str("\nSaved, out of range:\n");
        for s in &list.saved {
            let auto = if s.autoconnect {
                ""
            } else {
                "  (no autoconnect)"
            };
            let _ = writeln!(out, "  {}{auto}", printable(&s.ssid));
        }
    }
    out
}

/// The access points behind a Network, one per line.
pub fn aps_table(list: &ApList) -> String {
    let mut out = format!("{}\n", printable(&list.ssid));
    let _ = writeln!(
        out,
        "{:<17}  {:>4}  {:>9}  {:<7}  {:>6}  {:>9}  {:<8}  STATE",
        "BSSID", "CHAN", "FREQ", "BAND", "SIGNAL", "RATE", "DEVICE"
    );
    for ap in &list.aps {
        let rate = if ap.rate_mbps == 0 {
            "?".to_owned()
        } else {
            format!("{} Mbit/s", ap.rate_mbps)
        };
        let _ = writeln!(
            out,
            "{:<17}  {:>4}  {:>5} MHz  {:<7}  {:>5}%  {:>9}  {:<8}  {}",
            ap.bssid,
            ap.channel,
            ap.frequency_mhz,
            ap.band.label(),
            ap.signal,
            rate,
            ap.device,
            if ap.joined { "connected" } else { "" },
        );
    }
    out
}

pub fn scan_line(o: &ScanOutcome) -> String {
    let age = o
        .last_scan_ms_ago
        .map(|ms| format!(" (last scan {}s ago)", ms / 1000))
        .unwrap_or_default();
    match (o.requested, o.skipped) {
        (0, 0) => "No Wi-Fi device found.".into(),
        (0, _) => format!("Recent results reused{age}."),
        (n, 0) => format!("Scanning on {n} device(s)."),
        (n, s) => format!("Scanning on {n} device(s); {s} reused recent results{age}."),
    }
}
