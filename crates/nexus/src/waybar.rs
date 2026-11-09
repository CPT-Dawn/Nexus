//! `nexus status --waybar`: the machine's state as Waybar's custom-module
//! JSON (`text`, `tooltip`, `class`, `alt`). Pure: the CLI feeds it what the
//! Daemon said and prints what it returns.

use nexus_ipc::{
    Connectivity, EthernetList, EthernetState, Hotspot, LinkKind, State, Status, Vpn, VpnList,
    VpnState,
};
use serde::Serialize;

/// One line of Waybar output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Output {
    /// The bar text: the Link's name, or a word for the state.
    pub text: String,
    /// Several lines: the sentence `nexus status` prints, then the VPNs
    /// that are up or coming up and the Hotspot.
    pub tooltip: String,
    /// For styling: the state, the Link kind, `vpn`, `hotspot`.
    pub class: Vec<String>,
    /// For `format-icons`: the one word that says which icon to show.
    pub alt: String,
}

/// The glyph that goes in front of `text` when `waybar.icons` is on: one plain
/// Unicode symbol per `alt` word (no icon font needed).
fn glyph(alt: &str) -> &'static str {
    match alt {
        "online" => "\u{25cf}",
        "vpn" => "\u{25c6}",
        "connecting" | "disconnecting" => "\u{25cc}",
        "limited" | "portal" => "\u{25d0}",
        "offline" | "asleep" => "\u{25cb}",
        _ => "\u{2715}",
    }
}

/// `out` with its glyph in front of the text, for `waybar.icons = true`.
pub fn with_glyph(mut out: Output) -> Output {
    out.text = format!("{} {}", glyph(&out.alt), out.text);
    out
}

/// What the module shows for the Daemon being out of reach.
pub fn unavailable() -> Output {
    render(&Status::unavailable(), None, None, None)
}

/// Builds the module from the Daemon's status and, when known, its VPNs and
/// Hotspot.
pub fn render(
    status: &Status,
    vpns: Option<&VpnList>,
    hotspot: Option<&Hotspot>,
    ethernet: Option<&EthernetList>,
) -> Output {
    let live: Vec<&Vpn> = vpns
        .into_iter()
        .flat_map(|l| &l.vpns)
        .filter(|v| v.state != VpnState::Down)
        .collect();
    let vpn_up = live.iter().any(|v| v.state == VpnState::Up);
    let hosting = hotspot.filter(|h| h.on);

    let (state, word) = state_word(status);
    let mut text = match (&status.link, status.state) {
        (Some(link), State::Online | State::Local | State::Site | State::Connecting) => {
            link.name.clone()
        }
        _ => word.to_string(),
    };
    if vpn_up {
        text.push_str(" + VPN");
    }
    if hosting.is_some() {
        text.push_str(" + Hotspot");
    }

    let mut class = vec![state.to_string()];
    if let Some(link) = &status.link {
        class.push(
            match link.kind {
                LinkKind::Wifi => "wifi",
                LinkKind::Ethernet => "ethernet",
                LinkKind::Vpn | LinkKind::Wireguard => "vpn-link",
                _ => "other",
            }
            .into(),
        );
    }
    if vpn_up {
        class.push("vpn".into());
    }
    if hosting.is_some() {
        class.push("hotspot".into());
    }
    // A cable that is in but does not work is the one Ethernet fact the Link
    // cannot show: there is no primary Link to name it.
    let stuck: Vec<_> = ethernet
        .into_iter()
        .flat_map(|e| &e.ports)
        .filter(|p| p.state == EthernetState::Misconfigured)
        .collect();
    if !stuck.is_empty() {
        class.push("ethernet-problem".into());
    }

    let mut tooltip = vec![capitalise(&status.human())];
    for vpn in &live {
        let how = if vpn.state == VpnState::Up {
            "up"
        } else {
            "connecting"
        };
        tooltip.push(format!("VPN {}: {how}", vpn.name));
    }
    if let Some(h) = hosting {
        let name = h.ssid.as_deref().unwrap_or("Hotspot");
        tooltip.push(match (h.starting, h.devices) {
            (true, _) => format!("Hotspot {name}: starting"),
            (false, Some(1)) => format!("Hotspot {name}: 1 device"),
            (false, Some(n)) => format!("Hotspot {name}: {n} devices"),
            (false, None) => format!("Hotspot {name}: on"),
        });
    }

    for port in stuck {
        let why = port
            .trouble
            .as_ref()
            .map_or("not working", |t| t.message.as_str());
        tooltip.push(format!("Ethernet {}: {why}", port.id));
    }

    let alt = if vpn_up && state == "online" {
        "vpn"
    } else {
        state
    };
    Output {
        text: escape(&text),
        tooltip: escape(&tooltip.join("\n")),
        class,
        alt: alt.into(),
    }
}

/// The state's class/alt name and the bar text when there is no Link to name.
fn state_word(status: &Status) -> (&'static str, &'static str) {
    match status.state {
        State::Unavailable => ("unavailable", "No NetworkManager"),
        State::Asleep => ("asleep", "Off"),
        State::Offline => ("offline", "Offline"),
        State::Disconnecting => ("disconnecting", "Disconnecting"),
        State::Connecting => ("connecting", "Connecting"),
        State::Local | State::Site => ("limited", "No internet"),
        State::Online => match status.connectivity {
            Connectivity::Portal => ("portal", "Login required"),
            Connectivity::Limited => ("limited", "No internet"),
            _ => ("online", "Online"),
        },
        _ => ("unknown", "Unknown"),
    }
}

fn capitalise(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// Waybar renders `text` and `tooltip` as Pango markup, and a Network's name
/// is whatever its owner typed.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_ipc::{BackendInfo, Link, Radios, VpnKind};

    fn status(state: State, connectivity: Connectivity, link: Option<(&str, LinkKind)>) -> Status {
        Status {
            state,
            connectivity,
            link: link.map(|(name, kind)| Link {
                name: name.into(),
                kind,
            }),
            backend: BackendInfo {
                name: "NetworkManager".into(),
                version: None,
            },
            radios: Radios::default(),
        }
    }

    fn vpn(name: &str, state: VpnState) -> Vpn {
        Vpn {
            id: name.into(),
            name: name.into(),
            kind: VpnKind::Wireguard,
            state,
            addresses: vec![],
            endpoint: None,
        }
    }

    #[test]
    fn online_shows_the_link_name() {
        let s = status(
            State::Online,
            Connectivity::Full,
            Some(("Home", LinkKind::Wifi)),
        );
        let out = render(&s, None, None, None);
        assert_eq!(out.text, "Home");
        assert_eq!(out.alt, "online");
        assert_eq!(out.class, ["online", "wifi"]);
        assert_eq!(out.tooltip, "Online via Home");
    }

    #[test]
    fn a_portal_is_its_own_state() {
        let s = status(
            State::Online,
            Connectivity::Portal,
            Some(("Cafe", LinkKind::Wifi)),
        );
        let out = render(&s, None, None, None);
        assert_eq!(out.alt, "portal");
        assert_eq!(out.class[0], "portal");
    }

    #[test]
    fn offline_and_unavailable_have_words() {
        let off = render(
            &status(State::Offline, Connectivity::None, None),
            None,
            None,
            None,
        );
        assert_eq!(
            (off.text.as_str(), off.alt.as_str()),
            ("Offline", "offline")
        );
        let gone = unavailable();
        assert_eq!(gone.alt, "unavailable");
        assert_eq!(gone.text, "No NetworkManager");
    }

    #[test]
    fn a_vpn_that_is_up_changes_alt_class_and_tooltip() {
        let s = status(
            State::Online,
            Connectivity::Full,
            Some(("enp3s0", LinkKind::Ethernet)),
        );
        let vpns = VpnList {
            vpns: vec![vpn("Work", VpnState::Up), vpn("Spare", VpnState::Down)],
        };
        let out = render(&s, Some(&vpns), None, None);
        assert_eq!(out.text, "enp3s0 + VPN");
        assert_eq!(out.alt, "vpn");
        assert_eq!(out.class, ["online", "ethernet", "vpn"]);
        assert_eq!(out.tooltip, "Online via enp3s0\nVPN Work: up");
    }

    #[test]
    fn a_plugged_port_that_does_not_work_is_flagged() {
        use nexus_ipc::{EthernetPort, EthernetProblem, EthernetTrouble};
        let mut stuck = EthernetPort::bare("enp3s0", EthernetState::Misconfigured);
        stuck.trouble = Some(EthernetTrouble {
            kind: EthernetProblem::SignIn,
            message: "enp3s0 wants an 802.1X sign-in.".into(),
        });
        let fine = EthernetPort::bare("enp4s0", EthernetState::Unplugged);
        let list = EthernetList {
            ports: vec![stuck, fine],
        };
        let s = status(State::Offline, Connectivity::None, None);
        let out = render(&s, None, None, Some(&list));
        assert!(out.class.contains(&"ethernet-problem".to_string()));
        assert!(
            out.tooltip
                .contains("Ethernet enp3s0: enp3s0 wants an 802.1X sign-in.")
        );
        assert!(
            !out.tooltip.contains("enp4s0"),
            "an unplugged port is not news"
        );
        let calm = render(&s, None, None, Some(&EthernetList::default()));
        assert!(!calm.class.contains(&"ethernet-problem".to_string()));
    }

    #[test]
    fn a_hotspot_is_listed_with_its_devices() {
        let s = status(
            State::Online,
            Connectivity::Full,
            Some(("Home", LinkKind::Wifi)),
        );
        let h = Hotspot {
            available: true,
            why: None,
            on: true,
            starting: false,
            ssid: Some("Garage".into()),
            devices: Some(2),
        };
        let out = render(&s, None, Some(&h), None);
        assert_eq!(out.text, "Home + Hotspot");
        assert!(out.class.contains(&"hotspot".to_string()));
        assert!(out.tooltip.ends_with("Hotspot Garage: 2 devices"));
    }

    #[test]
    fn names_cannot_inject_markup() {
        let s = status(
            State::Online,
            Connectivity::Full,
            Some(("<b>&Evil", LinkKind::Wifi)),
        );
        let out = render(&s, None, None, None);
        assert_eq!(out.text, "&lt;b&gt;&amp;Evil");
        assert!(!out.tooltip.contains('<'));
    }
}
