// D-Bus proxy trait definitions for NetworkManager interfaces.
// These use zbus's #[proxy] macro to auto-generate typed async clients.

use std::collections::HashMap;
use zbus::proxy;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue};

// ── NetworkManager Main Interface ─────────────────────────────────────

#[proxy(
    interface = "org.freedesktop.NetworkManager",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/org/freedesktop/NetworkManager"
)]
pub trait NetworkManager {
    /// Get all network devices
    fn get_devices(&self) -> zbus::Result<Vec<OwnedObjectPath>>;

    /// Activate a connection
    fn activate_connection(
        &self,
        connection: &ObjectPath<'_>,
        device: &ObjectPath<'_>,
        specific_object: &ObjectPath<'_>,
    ) -> zbus::Result<OwnedObjectPath>;

    /// Add and activate a new connection
    fn add_and_activate_connection(
        &self,
        connection: HashMap<String, HashMap<String, OwnedValue>>,
        device: &ObjectPath<'_>,
        specific_object: &ObjectPath<'_>,
    ) -> zbus::Result<(OwnedObjectPath, OwnedObjectPath)>;

    /// Deactivate an active connection
    fn deactivate_connection(&self, active_connection: &ObjectPath<'_>) -> zbus::Result<()>;

    /// Enable or disable networking
    fn enable(&self, enable: bool) -> zbus::Result<()>;

    /// Check connectivity
    fn check_connectivity(&self) -> zbus::Result<u32>;

    /// NetworkManager version
    #[zbus(property)]
    fn version(&self) -> zbus::Result<String>;

    /// Overall NM state
    #[zbus(property)]
    fn state(&self) -> zbus::Result<u32>;

    /// Connectivity state
    #[zbus(property)]
    fn connectivity(&self) -> zbus::Result<u32>;

    /// Whether wireless is enabled
    #[zbus(property)]
    fn wireless_enabled(&self) -> zbus::Result<bool>;

    /// Set wireless enabled/disabled
    #[zbus(property)]
    fn set_wireless_enabled(&self, enabled: bool) -> zbus::Result<()>;

    /// Whether networking is enabled
    #[zbus(property)]
    fn networking_enabled(&self) -> zbus::Result<bool>;

    /// Currently active connections
    #[zbus(property)]
    fn active_connections(&self) -> zbus::Result<Vec<OwnedObjectPath>>;

    /// The primary connection
    #[zbus(property)]
    fn primary_connection(&self) -> zbus::Result<OwnedObjectPath>;
}

// ── Device Interface ──────────────────────────────────────────────────

#[proxy(
    interface = "org.freedesktop.NetworkManager.Device",
    default_service = "org.freedesktop.NetworkManager"
)]
pub trait Device {
    /// Disconnect this device
    fn disconnect(&self) -> zbus::Result<()>;

    /// Re-apply connection settings
    fn reapply(
        &self,
        connection: HashMap<String, HashMap<String, OwnedValue>>,
        version_id: u64,
        flags: u32,
    ) -> zbus::Result<()>;

    /// Device interface name (e.g., "wlan0")
    #[zbus(property)]
    fn interface(&self) -> zbus::Result<String>;

    /// Device type
    #[zbus(property)]
    fn device_type(&self) -> zbus::Result<u32>;

    /// Current device state
    #[zbus(property)]
    fn state(&self) -> zbus::Result<u32>;

    /// Hardware address
    #[zbus(property)]
    fn hw_address(&self) -> zbus::Result<String>;

    /// IP4Config object path
    #[zbus(property)]
    fn ip4_config(&self) -> zbus::Result<OwnedObjectPath>;

    /// IP6Config object path
    #[zbus(property)]
    fn ip6_config(&self) -> zbus::Result<OwnedObjectPath>;

    /// Active connection path
    #[zbus(property)]
    fn active_connection(&self) -> zbus::Result<OwnedObjectPath>;

    /// Whether the device is managed by NM
    #[zbus(property)]
    fn managed(&self) -> zbus::Result<bool>;

    /// Set managed state
    #[zbus(property)]
    fn set_managed(&self, managed: bool) -> zbus::Result<()>;

    /// Whether autoconnect is enabled
    #[zbus(property)]
    fn autoconnect(&self) -> zbus::Result<bool>;

    /// Set autoconnect
    #[zbus(property)]
    fn set_autoconnect(&self, autoconnect: bool) -> zbus::Result<()>;

    /// Device driver
    #[zbus(property)]
    fn driver(&self) -> zbus::Result<String>;

    /// MTU
    #[zbus(property)]
    fn mtu(&self) -> zbus::Result<u32>;
}

// ── Wireless Device Interface ─────────────────────────────────────────

#[proxy(
    interface = "org.freedesktop.NetworkManager.Device.Wireless",
    default_service = "org.freedesktop.NetworkManager"
)]
pub trait Wireless {
    /// Request a WiFi scan
    fn request_scan(&self, options: HashMap<String, OwnedValue>) -> zbus::Result<()>;

    /// Get all visible access points
    fn get_all_access_points(&self) -> zbus::Result<Vec<OwnedObjectPath>>;

    /// Active access point
    #[zbus(property)]
    fn active_access_point(&self) -> zbus::Result<OwnedObjectPath>;

    /// Bitrate in kbit/s
    #[zbus(property)]
    fn bitrate(&self) -> zbus::Result<u32>;

    /// Last scan time (unix epoch in milliseconds)
    #[zbus(property)]
    fn last_scan(&self) -> zbus::Result<i64>;

    /// Signal: access point added
    #[zbus(signal)]
    fn access_point_added(&self, access_point: OwnedObjectPath) -> zbus::Result<()>;

    /// Signal: access point removed
    #[zbus(signal)]
    fn access_point_removed(&self, access_point: OwnedObjectPath) -> zbus::Result<()>;
}

// ── Access Point Interface ────────────────────────────────────────────

#[proxy(
    interface = "org.freedesktop.NetworkManager.AccessPoint",
    default_service = "org.freedesktop.NetworkManager"
)]
pub trait AccessPoint {
    /// SSID as bytes
    #[zbus(property)]
    fn ssid(&self) -> zbus::Result<Vec<u8>>;

    /// BSSID (MAC address string)
    #[zbus(property)]
    fn hw_address(&self) -> zbus::Result<String>;

    /// Frequency in MHz
    #[zbus(property)]
    fn frequency(&self) -> zbus::Result<u32>;

    /// Signal strength 0-100
    #[zbus(property)]
    fn strength(&self) -> zbus::Result<u8>;

    /// AP flags (privacy etc.)
    #[zbus(property)]
    fn flags(&self) -> zbus::Result<u32>;

    /// WPA flags
    #[zbus(property)]
    fn wpa_flags(&self) -> zbus::Result<u32>;

    /// RSN (WPA2/WPA3) flags
    #[zbus(property)]
    fn rsn_flags(&self) -> zbus::Result<u32>;

    /// AP mode
    #[zbus(property)]
    fn mode(&self) -> zbus::Result<u32>;
}

// ── IP4Config Interface ───────────────────────────────────────────────

#[proxy(
    interface = "org.freedesktop.NetworkManager.IP4Config",
    default_service = "org.freedesktop.NetworkManager"
)]
pub trait IP4Config {
    /// Address data: array of dicts with "address" (string) and "prefix" (u32)
    #[zbus(property)]
    fn address_data(&self) -> zbus::Result<Vec<HashMap<String, OwnedValue>>>;

    /// Gateway
    #[zbus(property)]
    fn gateway(&self) -> zbus::Result<String>;

    /// DNS server data
    #[zbus(property)]
    fn nameserver_data(&self) -> zbus::Result<Vec<HashMap<String, OwnedValue>>>;

    /// DNS search domains
    #[zbus(property)]
    fn searches(&self) -> zbus::Result<Vec<String>>;
}

// ── IP6Config Interface ───────────────────────────────────────────────

#[proxy(
    interface = "org.freedesktop.NetworkManager.IP6Config",
    default_service = "org.freedesktop.NetworkManager"
)]
pub trait IP6Config {
    /// Address data
    #[zbus(property)]
    fn address_data(&self) -> zbus::Result<Vec<HashMap<String, OwnedValue>>>;

    /// Gateway
    #[zbus(property)]
    fn gateway(&self) -> zbus::Result<String>;
}

// ── Active Connection Interface ───────────────────────────────────────

#[proxy(
    interface = "org.freedesktop.NetworkManager.Connection.Active",
    default_service = "org.freedesktop.NetworkManager"
)]
pub trait ActiveConnection {
    /// Human-readable connection ID
    #[zbus(property)]
    fn id(&self) -> zbus::Result<String>;

    /// Connection UUID
    #[zbus(property)]
    fn uuid(&self) -> zbus::Result<String>;

    /// Connection type
    #[zbus(property, name = "Type")]
    fn connection_type(&self) -> zbus::Result<String>;

    /// State of the active connection
    #[zbus(property)]
    fn state(&self) -> zbus::Result<u32>;

    /// Devices using this connection
    #[zbus(property)]
    fn devices(&self) -> zbus::Result<Vec<OwnedObjectPath>>;

    /// The settings connection path
    #[zbus(property)]
    fn connection(&self) -> zbus::Result<OwnedObjectPath>;

    /// IP4Config path
    #[zbus(property)]
    fn ip4_config(&self) -> zbus::Result<OwnedObjectPath>;
}

// ── Settings Interface ────────────────────────────────────────────────

#[proxy(
    interface = "org.freedesktop.NetworkManager.Settings",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/org/freedesktop/NetworkManager/Settings"
)]
pub trait Settings {
    /// List all saved connection profiles
    fn list_connections(&self) -> zbus::Result<Vec<OwnedObjectPath>>;

    /// Add a new connection
    fn add_connection(
        &self,
        connection: HashMap<String, HashMap<String, OwnedValue>>,
    ) -> zbus::Result<OwnedObjectPath>;

    /// Get hostname
    #[zbus(property)]
    fn hostname(&self) -> zbus::Result<String>;

    /// Signal: new connection added
    #[zbus(signal)]
    fn new_connection(&self, connection: OwnedObjectPath) -> zbus::Result<()>;
}

// ── Connection Settings Interface ─────────────────────────────────────

#[proxy(
    interface = "org.freedesktop.NetworkManager.Settings.Connection",
    default_service = "org.freedesktop.NetworkManager"
)]
pub trait ConnectionSettings {
    /// Get all settings for this connection
    fn get_settings(&self) -> zbus::Result<HashMap<String, HashMap<String, OwnedValue>>>;

    /// Update this connection's settings
    fn update(&self, properties: HashMap<String, HashMap<String, OwnedValue>>) -> zbus::Result<()>;

    /// Delete this connection
    fn delete(&self) -> zbus::Result<()>;

    /// Get secrets for this connection
    fn get_secrets(
        &self,
        setting_name: &str,
    ) -> zbus::Result<HashMap<String, HashMap<String, OwnedValue>>>;
}

// ── Wired Device Interface ────────────────────────────────────────────

#[proxy(
    interface = "org.freedesktop.NetworkManager.Device.Wired",
    default_service = "org.freedesktop.NetworkManager"
)]
pub trait Wired {
    /// Speed in Mbit/s
    #[zbus(property)]
    fn speed(&self) -> zbus::Result<u32>;

    /// Whether carrier (cable) is detected
    #[zbus(property)]
    fn carrier(&self) -> zbus::Result<bool>;
}
