//! TUI sentences: everything the terminal Front end says in its own voice
//! (toasts, empty states, headlines). Part of the one text catalog; kept in
//! its own file so the Daemon's sentences in `text.rs` stay a separate read.

use nexus_ipc::{Connectivity, LinkKind, Radios, State, Status};

/// The one-word headline for the Overview hero.
pub fn headline(s: &Status) -> &'static str {
    match (s.state, s.connectivity) {
        (State::Unavailable, _) => "Unavailable",
        (State::Connecting, _) => "Connecting",
        (State::Disconnecting, _) => "Disconnecting",
        (State::Online, Connectivity::Portal) => "Portal",
        (State::Online, _) => "Online",
        (State::Local | State::Site, _) => "No internet",
        (State::Asleep, _) => "Off",
        (State::Offline, _) => "Offline",
        _ => "Unknown",
    }
}

// ---- radios ---------------------------------------------------------------------

/// What the Wi-Fi list says in place of Networks while the radio is off.
pub fn wifi_off(r: &Radios) -> String {
    if r.wifi.hardware_blocked {
        super::HARD_BLOCKED.to_owned()
    } else if r.airplane {
        "Airplane mode is on, so Wi-Fi is off.\nPress A to turn it off.".to_owned()
    } else {
        "Wi-Fi is off.\nPress w to turn it on.".to_owned()
    }
}

/// The Overview card of a Wi-Fi radio that is off.
pub fn wifi_card_off(r: &Radios) -> &'static str {
    if r.wifi.hardware_blocked {
        "Blocked by a switch"
    } else if r.airplane {
        "Airplane mode"
    } else {
        "Off"
    }
}

pub fn wifi_power(on: bool) -> &'static str {
    if on { "Wi-Fi is on." } else { "Wi-Fi is off." }
}

pub fn airplane(on: bool) -> &'static str {
    if on {
        "Airplane mode is on. Wi-Fi and Bluetooth are off."
    } else {
        "Airplane mode is off."
    }
}

pub const WIFI_ALREADY_ON: &str = "Wi-Fi is already on.";
pub const WIFI_ALREADY_OFF: &str = "Wi-Fi is already off.";
pub const AIRPLANE_ALREADY_ON: &str = "Airplane mode is already on.";
pub const AIRPLANE_ALREADY_OFF: &str = "Airplane mode is already off.";

// ---- Settings -------------------------------------------------------------------

pub fn switched(label: &str, on: bool) -> String {
    format!("{label} turned {}.", if on { "on" } else { "off" })
}

pub fn reset(label: &str) -> String {
    format!("{label} is back to its default.")
}

// ---- Overview hero ------------------------------------------------------------

fn via(s: &Status) -> String {
    match &s.link {
        Some(l) => {
            let kind = match l.kind {
                LinkKind::Wifi => "Wi-Fi",
                LinkKind::Ethernet => "Ethernet",
                LinkKind::Vpn | LinkKind::Wireguard => "VPN",
                _ => "a network",
            };
            format!(" via {kind} ({})", l.name)
        }
        None => String::new(),
    }
}

/// The sentence under the headline. `trouble` says the Doctor found something
/// wrong: then the sentence must not claim the machine is healthy.
pub fn sentence(s: &Status, trouble: bool) -> String {
    match (s.state, s.connectivity) {
        (State::Unavailable, _) => "NetworkManager is not running.".into(),
        (State::Asleep, _) => "Networking is switched off.".into(),
        (State::Offline, _) => "You're offline. No network is connected.".into(),
        (State::Connecting, _) => "Connecting to a network...".into(),
        (State::Disconnecting, _) => "Disconnecting...".into(),
        (State::Online, Connectivity::Portal) => {
            format!(
                "You're connected{}, but a sign-in page is required. Press o to open it.",
                via(s)
            )
        }
        (State::Online, Connectivity::Limited) => {
            format!(
                "You're connected{}, but the internet is unreachable.",
                via(s)
            )
        }
        (State::Online, _) if trouble => {
            format!("You're online{}. The Doctor has found a problem.", via(s))
        }
        (State::Online, _) => format!("You're online{}. Everything looks healthy.", via(s)),
        (State::Local | State::Site, _) => {
            format!("You're connected{}, but there is no internet.", via(s))
        }
        _ => "The network state is unknown.".into(),
    }
}

// ---- the `/` filter ---------------------------------------------------------------

/// The list title's tail while a filter is typed or in force.
pub fn filter_title(filter: &str, typing: bool) -> String {
    format!(" · /{filter}{}", if typing { "_" } else { "" })
}

/// A list whose filter hides every row.
pub fn no_match(what: &str, filter: &str) -> String {
    format!("No {what} match “{filter}”.\nEsc clears the filter.")
}
