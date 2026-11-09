//! Plain-text rendering of the Ethernet ports for `nexus ethernet list`.

use std::fmt::Write as _;

use nexus_ipc::{EthernetList, EthernetPort, EthernetState};

/// One word for where a port stands.
pub fn state_word(state: EthernetState) -> &'static str {
    match state {
        EthernetState::Unplugged => "unplugged",
        EthernetState::Disconnected => "disconnected",
        EthernetState::Connecting => "connecting",
        EthernetState::Connected => "connected",
        EthernetState::Misconfigured => "misconfigured",
        _ => "unknown",
    }
}

fn link_line(p: &EthernetPort) -> String {
    let mut parts = Vec::new();
    if let Some(speed) = p.speed_text() {
        parts.push(speed);
    }
    for a in &p.addresses {
        parts.push(format!("{}/{}", a.address, a.prefix));
    }
    if let Some(g) = &p.gateway {
        parts.push(format!("gateway {g}"));
    }
    parts.join("  ")
}

pub fn table(list: &EthernetList) -> String {
    if list.ports.is_empty() {
        return "No Ethernet port found.\n".into();
    }
    let width = list
        .ports
        .iter()
        .map(|p| p.id.len())
        .max()
        .unwrap_or(4)
        .max(4);
    let mut out = format!("{:<width$}  {:<13}  LINK\n", "PORT", "STATE");
    for p in &list.ports {
        let _ = writeln!(
            out,
            "{:<width$}  {:<13}  {}",
            p.id,
            state_word(p.state),
            link_line(p)
        );
        if let Some(t) = &p.trouble {
            let _ = writeln!(out, "{:<width$}  {}", "", t.message);
        } else if p.state == EthernetState::Unplugged {
            let _ = writeln!(
                out,
                "{:<width$}  {}",
                "",
                crate::text::unplugged_sentence(&p.id)
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_ipc::{EthernetProblem, EthernetTrouble};

    #[test]
    fn unplugged_and_misconfigured_say_different_things() {
        let mut stuck = EthernetPort::bare("enp3s0", EthernetState::Misconfigured);
        stuck.trouble = Some(EthernetTrouble {
            kind: EthernetProblem::NoAddress,
            message: "no address".into(),
        });
        let list = EthernetList {
            ports: vec![
                EthernetPort::bare("enp1s0", EthernetState::Unplugged),
                stuck,
            ],
        };
        let out = table(&list);
        assert!(out.contains("unplugged"), "{out}");
        assert!(out.contains("No cable detected on enp1s0"), "{out}");
        assert!(out.contains("misconfigured"), "{out}");
        assert!(out.contains("no address"), "{out}");
    }
}
