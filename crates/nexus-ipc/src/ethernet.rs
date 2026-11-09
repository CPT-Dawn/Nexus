//! Wire types of the Ethernet tab: ports, their Link details, and why a port
//! that has a cable in it still does not work.

use serde::{Deserialize, Serialize};

use crate::AddressEntry;

/// Where an Ethernet port stands. An unplugged cable and a plugged one that
/// the network does not talk to are different states with different cures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum EthernetState {
    /// No cable, or nothing at the other end of it.
    Unplugged,
    /// A cable is in and nothing uses it (no Profile joined, or it was dropped).
    Disconnected,
    Connecting,
    Connected,
    /// A cable is in, but joining did not work: no address, or a sign-in the
    /// port wants and does not get. `problem` says which.
    Misconfigured,
    #[serde(other)]
    Unknown,
}

/// Why a plugged-in port is [`EthernetState::Misconfigured`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum EthernetProblem {
    /// The network never handed out an address (DHCP timed out), or the
    /// address settings cannot work.
    NoAddress,
    /// The port wants an 802.1X sign-in that was not given, or was refused.
    SignIn,
    /// Something else; the message says what NetworkManager reported.
    Other,
    #[serde(other)]
    Unknown,
}

/// The Profile a port uses (or would use).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EthernetProfile {
    pub uuid: String,
    pub name: String,
    /// The Profile signs in with 802.1X.
    #[serde(default)]
    pub enterprise: bool,
}

/// What went wrong on a port, in words for the person.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EthernetTrouble {
    pub kind: EthernetProblem,
    pub message: String,
}

/// One Ethernet port and, while it is up, its Link.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EthernetPort {
    /// The interface name (`enp3s0`): what the other ops take as `port`.
    pub id: String,
    pub state: EthernetState,
    /// The hardware address, upper case.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mac: Option<String>,
    /// Negotiated speed in Mbit/s; absent when there is no link.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_mbps: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<EthernetProfile>,
    /// IPv4 and IPv6 addresses, while the Link is up.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub addresses: Vec<AddressEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateway: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dns: Vec<String>,
    /// Set exactly when `state` is `misconfigured`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trouble: Option<EthernetTrouble>,
}

impl EthernetPort {
    /// A port with nothing known about it but its name and state.
    pub fn bare(id: &str, state: EthernetState) -> Self {
        Self {
            id: id.to_owned(),
            state,
            mac: None,
            speed_mbps: None,
            profile: None,
            addresses: Vec::new(),
            gateway: None,
            dns: Vec::new(),
            trouble: None,
        }
    }

    /// The speed as a person says it: `100 Mbps`, `1 Gbps`, `2.5 Gbps`.
    pub fn speed_text(&self) -> Option<String> {
        self.speed_mbps.filter(|s| *s > 0).map(speed_text)
    }
}

/// What to tell a person whose port has no cable: shared by the Daemon's
/// refusals and the Front ends' port details, so every surface says the same.
pub fn unplugged_message(port: &str) -> String {
    format!(
        "No cable detected on {port}. Plug one in, and check that the other end is \
         connected to a switch or router that is on."
    )
}

/// `1000` is `1 Gbps`, `2500` is `2.5 Gbps`, `100` is `100 Mbps`.
pub fn speed_text(mbps: u32) -> String {
    if mbps >= 1000 && mbps.is_multiple_of(1000) {
        format!("{} Gbps", mbps / 1000)
    } else if mbps >= 1000 {
        format!("{} Gbps", f64::from(mbps) / 1000.0)
    } else {
        format!("{mbps} Mbps")
    }
}

/// The `ethernet_list` result and the `ethernet_changed` payload.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EthernetList {
    pub ports: Vec<EthernetPort>,
}

/// The `ethernet_connect` and `ethernet_join_eap` result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EthernetJoined {
    pub port: String,
    pub profile: String,
}

/// The `ethernet_disconnect` result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EthernetDropped {
    pub port: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speeds_read_the_way_people_say_them() {
        assert_eq!(speed_text(10), "10 Mbps");
        assert_eq!(speed_text(100), "100 Mbps");
        assert_eq!(speed_text(1000), "1 Gbps");
        assert_eq!(speed_text(2500), "2.5 Gbps");
        assert_eq!(speed_text(10_000), "10 Gbps");
    }

    #[test]
    fn a_port_from_a_newer_daemon_still_reads() {
        let port: EthernetPort =
            serde_json::from_str(r#"{"id":"eth0","state":"warp","future":1}"#).unwrap();
        assert_eq!(port.state, EthernetState::Unknown);
        assert!(port.addresses.is_empty());
    }
}
