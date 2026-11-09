//! The `profile_list` and `device_list` results: what is saved and what
//! hardware there is, across Wi-Fi and Ethernet, in one answer. The Daemon
//! builds both from the Wi-Fi and Ethernet lists, so a Front end that wants an
//! inventory (a script, a status bar) needs one call, not two plus a merge.

use serde::{Deserialize, Serialize};

use crate::{EthernetList, WifiList};

/// What a saved Profile connects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ProfileKind {
    Wifi,
    Ethernet,
    #[serde(other)]
    Unknown,
}

/// One saved Profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileEntry {
    pub kind: ProfileKind,
    /// The Profile's UUID: what `profile_get` takes as `profile`.
    pub uuid: String,
    /// The SSID, or the Ethernet Profile's name.
    pub name: String,
    /// A Wi-Fi Profile's `WifiNetwork.id`: what `profile_get` takes as `network`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network: Option<String>,
    /// Whether it joins by itself; absent when the Backend does not say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub autoconnect: Option<bool>,
}

/// The `profile_list` result.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileList {
    pub profiles: Vec<ProfileEntry>,
}

impl ProfileList {
    /// Every saved Wi-Fi Profile (nearby or not) and every Ethernet Profile.
    pub fn of(wifi: &WifiList, ethernet: &EthernetList) -> Self {
        let mut profiles = Vec::new();
        for n in &wifi.networks {
            if let Some(uuid) = &n.profile {
                profiles.push(ProfileEntry {
                    kind: ProfileKind::Wifi,
                    uuid: uuid.clone(),
                    name: n.ssid.clone(),
                    network: Some(n.id.clone()),
                    autoconnect: n.autoconnect,
                });
            }
        }
        for s in &wifi.saved {
            profiles.push(ProfileEntry {
                kind: ProfileKind::Wifi,
                uuid: s.uuid.clone(),
                name: s.ssid.clone(),
                network: Some(s.id.clone()),
                autoconnect: Some(s.autoconnect),
            });
        }
        for p in ethernet.ports.iter().filter_map(|p| p.profile.as_ref()) {
            profiles.push(ProfileEntry {
                kind: ProfileKind::Ethernet,
                uuid: p.uuid.clone(),
                name: p.name.clone(),
                network: None,
                autoconnect: None,
            });
        }
        Self { profiles }
    }
}

/// What kind of hardware a Device is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum DeviceKind {
    Wifi,
    Ethernet,
    #[serde(other)]
    Unknown,
}

/// One Device (a Wi-Fi card or an Ethernet port).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceEntry {
    pub kind: DeviceKind,
    /// What the other ops take: a Wi-Fi Device's `WifiDevice.id`, an Ethernet
    /// port's interface name.
    pub id: String,
    /// The interface name (`wlan0`, `enp3s0`).
    pub name: String,
}

/// The `device_list` result.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceList {
    pub devices: Vec<DeviceEntry>,
}

impl DeviceList {
    /// Wi-Fi Devices first, then Ethernet ports.
    pub fn of(wifi: &WifiList, ethernet: &EthernetList) -> Self {
        let wifi = wifi.devices.iter().map(|d| DeviceEntry {
            kind: DeviceKind::Wifi,
            id: d.id.clone(),
            name: d.name.clone(),
        });
        let wired = ethernet.ports.iter().map(|p| DeviceEntry {
            kind: DeviceKind::Ethernet,
            id: p.id.clone(),
            name: p.id.clone(),
        });
        Self {
            devices: wifi.chain(wired).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EthernetPort, EthernetProfile, EthernetState, SavedWifi, WifiDevice};

    fn lists() -> (WifiList, EthernetList) {
        let wifi = WifiList {
            devices: vec![WifiDevice {
                id: "/dev/0".into(),
                name: "wlan0".into(),
            }],
            networks: Vec::new(),
            saved: vec![SavedWifi {
                uuid: "u1".into(),
                id: "6162".into(),
                ssid: "ab".into(),
                autoconnect: true,
            }],
        };
        let mut port = EthernetPort::bare("enp3s0", EthernetState::Connected);
        port.profile = Some(EthernetProfile {
            uuid: "u2".into(),
            name: "Wired".into(),
            enterprise: false,
        });
        (wifi, EthernetList { ports: vec![port] })
    }

    #[test]
    fn profiles_come_from_both_lists() {
        let (wifi, ethernet) = lists();
        let list = ProfileList::of(&wifi, &ethernet);
        let kinds: Vec<_> = list
            .profiles
            .iter()
            .map(|p| (p.kind, p.uuid.as_str()))
            .collect();
        assert_eq!(
            kinds,
            [(ProfileKind::Wifi, "u1"), (ProfileKind::Ethernet, "u2")]
        );
        assert_eq!(list.profiles[0].network.as_deref(), Some("6162"));
    }

    #[test]
    fn devices_list_wifi_then_wired() {
        let (wifi, ethernet) = lists();
        let list = DeviceList::of(&wifi, &ethernet);
        let names: Vec<_> = list.devices.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["wlan0", "enp3s0"]);
    }

    #[test]
    fn a_kind_from_a_newer_daemon_decodes_as_unknown() {
        let d: DeviceEntry =
            serde_json::from_str(r#"{"kind":"modem","id":"x","name":"wwan0"}"#).unwrap();
        assert_eq!(d.kind, DeviceKind::Unknown);
        let p: ProfileEntry =
            serde_json::from_str(r#"{"kind":"vpn","uuid":"u","name":"n"}"#).unwrap();
        assert_eq!(p.kind, ProfileKind::Unknown);
    }
}
