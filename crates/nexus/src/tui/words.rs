//! Plain-English wording for Daemon state: the sentence comes first, data second.

use nexus_ipc::{Connectivity, LinkKind, State, Status};

/// The one-word headline for the hero.
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

/// The sentence under the headline.
pub fn sentence(s: &Status) -> String {
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
        (State::Online, _) => format!("You're online{}. Everything looks healthy.", via(s)),
        (State::Local | State::Site, _) => {
            format!("You're connected{}, but there is no internet.", via(s))
        }
        _ => "The network state is unknown.".into(),
    }
}
