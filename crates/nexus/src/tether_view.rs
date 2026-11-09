//! Plain-text rendering of the Tethering list for `nexus tether list`.

use std::fmt::Write as _;

use nexus_ipc::{Bluetooth, Tethers};

/// A phone's name is the phone's to choose: show control characters as
/// U+FFFD so a hostile name cannot drive the terminal.
pub fn printable(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_control() { '\u{fffd}' } else { c })
        .collect()
}

/// One line saying whether Bluetooth can carry a Link.
pub fn bluetooth_line(b: Bluetooth) -> &'static str {
    match b {
        Bluetooth::On => "Bluetooth is on.",
        Bluetooth::Off => "Bluetooth is off.",
        Bluetooth::Absent => "No Bluetooth adapter found, or the Bluetooth service is not running.",
    }
}

pub fn table(t: &Tethers) -> String {
    let mut out = format!("{}\n", bluetooth_line(t.bluetooth));
    if t.phones.is_empty() {
        out.push_str("No paired phone offers network access.\n");
        return out;
    }
    let width = t
        .phones
        .iter()
        .map(|p| printable(&p.name).chars().count())
        .max()
        .unwrap_or(5)
        .max(5);
    let _ = writeln!(out, "{:<width$}  {:<17}  STATE", "PHONE", "ADDRESS");
    for p in &t.phones {
        let state = if p.connected {
            "connected"
        } else if p.connecting {
            "connecting"
        } else if p.saved {
            "saved"
        } else {
            "paired"
        };
        let _ = writeln!(out, "{:<width$}  {:<17}  {state}", printable(&p.name), p.id);
    }
    out
}
