//! The NetworkManager proxies and helpers every part of the Backend shares.

use std::collections::HashMap;

use zbus::{OwnedObjectPath, OwnedVariant};

/// `org.freedesktop.NetworkManager.Device`: what any kind of Device says.
#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Device",
    default_service = "org.freedesktop.NetworkManager"
)]
pub(super) trait Device {
    #[zbus(property)]
    fn interface(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn device_type(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn state(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn state_reason(&self) -> zbus::Result<(u32, u32)>;
    #[zbus(property)]
    fn active_connection(&self) -> zbus::Result<OwnedObjectPath>;
    #[zbus(property, name = "Ip4Config")]
    fn ip4_config(&self) -> zbus::Result<OwnedObjectPath>;
    #[zbus(property, name = "Ip6Config")]
    fn ip6_config(&self) -> zbus::Result<OwnedObjectPath>;
    fn disconnect(&self) -> zbus::Result<()>;
    #[zbus(signal, name = "StateChanged")]
    fn device_state_changed(&self, new_state: u32, old_state: u32, reason: u32)
    -> zbus::Result<()>;
}

/// `org.freedesktop.NetworkManager.Settings.Connection`: one saved Profile.
#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Settings.Connection",
    default_service = "org.freedesktop.NetworkManager"
)]
pub(super) trait SettingsConnection {
    #[zbus(property)]
    fn unsaved(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn version_id(&self) -> zbus::Result<u64>;
    fn get_settings(&self) -> zbus::Result<HashMap<String, HashMap<String, OwnedVariant>>>;
    fn get_secrets(
        &self,
        setting: &str,
    ) -> zbus::Result<HashMap<String, HashMap<String, OwnedVariant>>>;
    fn update(&self, settings: HashMap<String, HashMap<String, OwnedVariant>>) -> zbus::Result<()>;
    fn update2(
        &self,
        settings: HashMap<String, HashMap<String, OwnedVariant>>,
        flags: u32,
        args: HashMap<String, OwnedVariant>,
    ) -> zbus::Result<HashMap<String, OwnedVariant>>;
    fn save(&self) -> zbus::Result<()>;
    fn delete(&self) -> zbus::Result<()>;
}

/// A plain D-Bus value (no file descriptors) ready to put in a settings map.
pub(super) fn variant<'a>(v: impl Into<zbus::Variant<'a>>) -> OwnedVariant {
    OwnedVariant::try_from(v.into()).expect("plain values have no file descriptors")
}
