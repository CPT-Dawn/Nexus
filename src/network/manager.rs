use std::collections::HashMap;
use std::time::Duration;

use eyre::{Context, Result, bail};
use tracing::{debug, info};
use zbus::Connection;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

use super::NetworkBackend;
use super::types::*;

/// NetworkManager D-Bus backend
pub struct NmBackend {
    conn: Connection,
    wifi_device_path: OwnedObjectPath,
    interface: String,
}

impl NmBackend {
    /// Create a new NM backend, connecting to the system D-Bus.
    /// Auto-detects a WiFi device unless `interface` is specified.
    pub async fn new(interface: Option<&str>) -> Result<Self> {
        let conn = Connection::system()
            .await
            .wrap_err("Failed to connect to system D-Bus. Is D-Bus running?")?;

        // Verify NetworkManager is available by reading its Version property
        let nm_version: Result<String> = Self::get_property(
            &conn,
            "/org/freedesktop/NetworkManager",
            "org.freedesktop.NetworkManager",
            "Version",
        )
        .await;
        match &nm_version {
            Ok(v) => info!("NetworkManager version: {}", v),
            Err(_) => {
                bail!(
                    "NetworkManager is not running or not reachable via D-Bus.\n\
                     Start it with: sudo systemctl start NetworkManager\n\
                     Enable it with: sudo systemctl enable NetworkManager\n\
                     Install it with: sudo pacman -S networkmanager"
                );
            }
        }

        // Find WiFi device
        let (device_path, iface_name) = Self::find_wifi_device(&conn, interface).await?;

        info!("Using WiFi interface: {} ({})", iface_name, device_path);

        Ok(Self {
            conn,
            wifi_device_path: device_path,
            interface: iface_name,
        })
    }

    /// Get the D-Bus connection (for signal subscriptions)
    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    /// Get the WiFi device path
    pub fn device_path(&self) -> OwnedObjectPath {
        self.wifi_device_path.clone()
    }

    /// Call a method on the NetworkManager D-Bus interface
    async fn call_nm_method<
        B: serde::Serialize + zbus::zvariant::Type,
        R: serde::de::DeserializeOwned + zbus::zvariant::Type,
    >(
        conn: &Connection,
        path: &str,
        interface: &str,
        method: &str,
        body: &B,
    ) -> Result<R> {
        let msg = conn
            .call_method(
                Some("org.freedesktop.NetworkManager"),
                path,
                Some(interface),
                method,
                body,
            )
            .await
            .wrap_err_with(|| format!("D-Bus call failed: {interface}.{method}"))?;
        let result: R = msg.body().deserialize()?;
        Ok(result)
    }

    /// Get a property from a D-Bus object
    async fn get_property<R: TryFrom<OwnedValue>>(
        conn: &Connection,
        path: &str,
        interface: &str,
        property: &str,
    ) -> Result<R>
    where
        R::Error: std::fmt::Display,
    {
        let val: OwnedValue = Self::call_nm_method(
            conn,
            path,
            "org.freedesktop.DBus.Properties",
            "Get",
            &(interface, property),
        )
        .await?;

        R::try_from(val).map_err(|e| eyre::eyre!("Property conversion failed for {property}: {e}"))
    }

    /// Find a WiFi-capable network device
    async fn find_wifi_device(
        conn: &Connection,
        preferred_interface: Option<&str>,
    ) -> Result<(OwnedObjectPath, String)> {
        let devices: Vec<OwnedObjectPath> = Self::call_nm_method(
            conn,
            "/org/freedesktop/NetworkManager",
            "org.freedesktop.NetworkManager",
            "GetDevices",
            &(),
        )
        .await
        .wrap_err("Failed to list network devices")?;

        for device_path in &devices {
            let path_str = device_path.as_str();

            // Get device type: 2 = WiFi
            let dev_type: u32 = match Self::get_property(
                conn,
                path_str,
                "org.freedesktop.NetworkManager.Device",
                "DeviceType",
            )
            .await
            {
                Ok(t) => t,
                Err(_) => continue,
            };

            if dev_type != 2 {
                continue;
            }

            // Get interface name
            let iface: String = Self::get_property(
                conn,
                path_str,
                "org.freedesktop.NetworkManager.Device",
                "Interface",
            )
            .await
            .unwrap_or_default();

            // If user specified an interface, only match that one
            if let Some(preferred) = preferred_interface
                && iface != preferred
            {
                continue;
            }

            return Ok((device_path.clone(), iface));
        }

        if let Some(iface) = preferred_interface {
            bail!(
                "WiFi interface '{}' not found. Check with: nmcli device",
                iface
            );
        }
        bail!(
            "No WiFi adapter detected.\n\
             Check your hardware with: ip link\n\
             If using a USB adapter, ensure drivers are loaded."
        );
    }

    /// Get a list of saved connection profile SSIDs
    async fn get_saved_ssids(&self) -> Result<Vec<String>> {
        let conn_paths: Vec<OwnedObjectPath> = Self::call_nm_method(
            &self.conn,
            "/org/freedesktop/NetworkManager/Settings",
            "org.freedesktop.NetworkManager.Settings",
            "ListConnections",
            &(),
        )
        .await
        .unwrap_or_default();

        let mut ssids = Vec::new();

        for conn_path in &conn_paths {
            let settings: HashMap<String, HashMap<String, OwnedValue>> = match Self::call_nm_method(
                &self.conn,
                conn_path.as_str(),
                "org.freedesktop.NetworkManager.Settings.Connection",
                "GetSettings",
                &(),
            )
            .await
            {
                Ok(s) => s,
                Err(_) => continue,
            };

            // Check if it's a WiFi connection
            if let Some(conn_section) = settings.get("connection") {
                let conn_type: Option<String> = conn_section
                    .get("type")
                    .and_then(|v| String::try_from(v.clone()).ok());

                if conn_type.as_deref() != Some("802-11-wireless") {
                    continue;
                }
            } else {
                continue;
            }

            // Get the SSID
            if let Some(wireless) = settings.get("802-11-wireless")
                && let Some(ssid_val) = wireless.get("ssid")
                && let Ok(ssid_bytes) = <Vec<u8>>::try_from(ssid_val.clone())
            {
                let ssid = String::from_utf8_lossy(&ssid_bytes).to_string();
                if !ssid.is_empty() {
                    ssids.push(ssid);
                }
            }
        }

        Ok(ssids)
    }

    /// Parse an access point D-Bus object into a WiFiNetwork
    async fn parse_access_point(
        &self,
        ap_path: &str,
        saved_ssids: &[String],
        active_ssid: Option<&str>,
    ) -> Option<WiFiNetwork> {
        let ssid_bytes: Vec<u8> = Self::get_property(
            &self.conn,
            ap_path,
            "org.freedesktop.NetworkManager.AccessPoint",
            "Ssid",
        )
        .await
        .unwrap_or_default();

        let ssid = String::from_utf8_lossy(&ssid_bytes).to_string();
        // Skip hidden/empty SSIDs in the normal list
        if ssid.is_empty() {
            return None;
        }

        let bssid: String = Self::get_property(
            &self.conn,
            ap_path,
            "org.freedesktop.NetworkManager.AccessPoint",
            "HwAddress",
        )
        .await
        .unwrap_or_default();

        let strength: u8 = Self::get_property(
            &self.conn,
            ap_path,
            "org.freedesktop.NetworkManager.AccessPoint",
            "Strength",
        )
        .await
        .unwrap_or(0);

        let frequency: u32 = Self::get_property(
            &self.conn,
            ap_path,
            "org.freedesktop.NetworkManager.AccessPoint",
            "Frequency",
        )
        .await
        .unwrap_or(0);

        let flags: u32 = Self::get_property(
            &self.conn,
            ap_path,
            "org.freedesktop.NetworkManager.AccessPoint",
            "Flags",
        )
        .await
        .unwrap_or(0);

        let wpa_flags: u32 = Self::get_property(
            &self.conn,
            ap_path,
            "org.freedesktop.NetworkManager.AccessPoint",
            "WpaFlags",
        )
        .await
        .unwrap_or(0);

        let rsn_flags: u32 = Self::get_property(
            &self.conn,
            ap_path,
            "org.freedesktop.NetworkManager.AccessPoint",
            "RsnFlags",
        )
        .await
        .unwrap_or(0);

        let security = SecurityType::from_flags(flags, wpa_flags, rsn_flags);
        let is_saved = saved_ssids.contains(&ssid);
        let is_active = active_ssid.is_some_and(|a| a == ssid);

        Some(WiFiNetwork {
            ssid,
            bssid,
            signal_strength: strength,
            frequency,
            security,
            is_saved,
            is_active,
            ap_path: ap_path.to_string(),
            seen_ticks: 0,
            display_signal: strength as f32,
        })
    }

    /// Find the connection profile path for a given SSID
    async fn find_connection_for_ssid(&self, ssid: &str) -> Result<Option<OwnedObjectPath>> {
        let conn_paths: Vec<OwnedObjectPath> = Self::call_nm_method(
            &self.conn,
            "/org/freedesktop/NetworkManager/Settings",
            "org.freedesktop.NetworkManager.Settings",
            "ListConnections",
            &(),
        )
        .await
        .unwrap_or_default();

        for conn_path in &conn_paths {
            let settings: HashMap<String, HashMap<String, OwnedValue>> = match Self::call_nm_method(
                &self.conn,
                conn_path.as_str(),
                "org.freedesktop.NetworkManager.Settings.Connection",
                "GetSettings",
                &(),
            )
            .await
            {
                Ok(s) => s,
                Err(_) => continue,
            };

            if let Some(wireless) = settings.get("802-11-wireless")
                && let Some(ssid_val) = wireless.get("ssid")
                && let Ok(ssid_bytes) = <Vec<u8>>::try_from(ssid_val.clone())
            {
                let profile_ssid = String::from_utf8_lossy(&ssid_bytes);
                if profile_ssid == ssid {
                    return Ok(Some(conn_path.clone()));
                }
            }
        }

        Ok(None)
    }

    /// Build connection settings for a new WiFi connection
    fn build_connection_settings<'a>(
        ssid: &'a str,
        password: Option<&'a str>,
        hidden: bool,
    ) -> HashMap<String, HashMap<String, Value<'a>>> {
        let mut settings: HashMap<String, HashMap<String, Value<'a>>> = HashMap::new();

        // connection section
        let mut conn = HashMap::new();
        conn.insert("type".to_string(), Value::from("802-11-wireless"));
        conn.insert("id".to_string(), Value::from(ssid));
        settings.insert("connection".to_string(), conn);

        // 802-11-wireless section
        let mut wireless = HashMap::new();
        wireless.insert("ssid".to_string(), Value::from(ssid.as_bytes().to_vec()));
        if hidden {
            wireless.insert("hidden".to_string(), Value::from(true));
        }
        settings.insert("802-11-wireless".to_string(), wireless);

        // 802-11-wireless-security section (if password provided)
        if let Some(pwd) = password {
            let mut wireless_sec = HashMap::new();
            wireless_sec.insert("key-mgmt".to_string(), Value::from("wpa-psk"));
            wireless_sec.insert("psk".to_string(), Value::from(pwd));
            settings.insert("802-11-wireless-security".to_string(), wireless_sec);

            // Update wireless section to reference security
            if let Some(ws) = settings.get_mut("802-11-wireless") {
                ws.insert(
                    "security".to_string(),
                    Value::from("802-11-wireless-security"),
                );
            }
        }

        settings
    }

    /// Get the SSID of the currently active WiFi connection
    async fn get_active_ssid(&self) -> Option<String> {
        let active_conn: OwnedObjectPath = Self::get_property(
            &self.conn,
            self.wifi_device_path.as_str(),
            "org.freedesktop.NetworkManager.Device",
            "ActiveConnection",
        )
        .await
        .ok()?;

        if active_conn.as_str() == "/" {
            return None;
        }

        // Get the connection settings path
        let conn_path: OwnedObjectPath = Self::get_property(
            &self.conn,
            active_conn.as_str(),
            "org.freedesktop.NetworkManager.Connection.Active",
            "Connection",
        )
        .await
        .ok()?;

        let settings: HashMap<String, HashMap<String, OwnedValue>> = Self::call_nm_method(
            &self.conn,
            conn_path.as_str(),
            "org.freedesktop.NetworkManager.Settings.Connection",
            "GetSettings",
            &(),
        )
        .await
        .ok()?;

        let wireless = settings.get("802-11-wireless")?;
        let ssid_val = wireless.get("ssid")?;
        let ssid_bytes = <Vec<u8>>::try_from(ssid_val.clone()).ok()?;
        Some(String::from_utf8_lossy(&ssid_bytes).to_string())
    }
}

impl NetworkBackend for NmBackend {
    async fn scan(&self) -> Result<Vec<WiFiNetwork>> {
        debug!("Requesting WiFi scan on {}", self.interface);

        // Request a scan (may fail silently if one is already in progress)
        let scan_result: Result<()> = Self::call_nm_method(
            &self.conn,
            self.wifi_device_path.as_str(),
            "org.freedesktop.NetworkManager.Device.Wireless",
            "RequestScan",
            &HashMap::<String, OwnedValue>::new(),
        )
        .await;

        if let Err(e) = &scan_result {
            debug!("Scan request note: {}", e);
        }

        // Wait for scan to complete
        tokio::time::sleep(Duration::from_secs(2)).await;

        // Get APs
        let ap_paths: Vec<OwnedObjectPath> = Self::call_nm_method(
            &self.conn,
            self.wifi_device_path.as_str(),
            "org.freedesktop.NetworkManager.Device.Wireless",
            "GetAllAccessPoints",
            &(),
        )
        .await
        .wrap_err("Failed to get access points")?;

        let saved = self.get_saved_ssids().await.unwrap_or_default();
        let active_ssid = self.get_active_ssid().await;

        let mut networks = Vec::new();
        let mut seen_ssids = std::collections::HashSet::new();

        for ap_path in &ap_paths {
            if let Some(net) = self
                .parse_access_point(ap_path.as_str(), &saved, active_ssid.as_deref())
                .await
            {
                // Deduplicate by SSID — keep the strongest signal
                if let Some(existing) = networks
                    .iter_mut()
                    .find(|n: &&mut WiFiNetwork| n.ssid == net.ssid)
                {
                    if net.signal_strength > existing.signal_strength {
                        *existing = net;
                    }
                } else {
                    seen_ssids.insert(net.ssid.clone());
                    networks.push(net);
                }
            }
        }

        // Sort: active first, then by signal strength descending
        networks.sort_by(|a, b| {
            b.is_active
                .cmp(&a.is_active)
                .then(b.signal_strength.cmp(&a.signal_strength))
        });

        info!("Scan complete: {} networks found", networks.len());
        Ok(networks)
    }

    async fn connect(&self, ssid: &str, password: Option<&str>) -> Result<()> {
        info!("Connecting to network: {}", ssid);

        // Check if we have a saved connection
        if let Some(conn_path) = self.find_connection_for_ssid(ssid).await? {
            debug!("Using saved connection profile for {}", ssid);
            let _: OwnedObjectPath = Self::call_nm_method(
                &self.conn,
                "/org/freedesktop/NetworkManager",
                "org.freedesktop.NetworkManager",
                "ActivateConnection",
                &(
                    &conn_path,
                    &self.wifi_device_path,
                    ObjectPath::try_from("/").unwrap(),
                ),
            )
            .await
            .wrap_err_with(|| format!("Failed to activate saved connection for '{ssid}'"))?;
        } else {
            debug!("Creating new connection for {}", ssid);
            let settings = Self::build_connection_settings(ssid, password, false);
            let (_conn_path, _active_conn): (OwnedObjectPath, OwnedObjectPath) =
                Self::call_nm_method(
                    &self.conn,
                    "/org/freedesktop/NetworkManager",
                    "org.freedesktop.NetworkManager",
                    "AddAndActivateConnection",
                    &(
                        settings,
                        &self.wifi_device_path,
                        ObjectPath::try_from("/").unwrap(),
                    ),
                )
                .await
                .wrap_err_with(|| format!("Failed to connect to '{ssid}'"))?;
        }

        Ok(())
    }

    async fn disconnect(&self) -> Result<()> {
        info!("Disconnecting WiFi");

        let active_conn: OwnedObjectPath = Self::get_property(
            &self.conn,
            self.wifi_device_path.as_str(),
            "org.freedesktop.NetworkManager.Device",
            "ActiveConnection",
        )
        .await
        .wrap_err("No active connection to disconnect")?;

        if active_conn.as_str() == "/" {
            bail!("No active WiFi connection");
        }

        let _: () = Self::call_nm_method(
            &self.conn,
            "/org/freedesktop/NetworkManager",
            "org.freedesktop.NetworkManager",
            "DeactivateConnection",
            &(&active_conn,),
        )
        .await
        .wrap_err("Failed to disconnect")?;

        Ok(())
    }

    async fn forget_network(&self, ssid: &str) -> Result<()> {
        info!("Forgetting network: {}", ssid);

        let conn_path = self
            .find_connection_for_ssid(ssid)
            .await?
            .ok_or_else(|| eyre::eyre!("No saved profile found for '{}'", ssid))?;

        let _: () = Self::call_nm_method(
            &self.conn,
            conn_path.as_str(),
            "org.freedesktop.NetworkManager.Settings.Connection",
            "Delete",
            &(),
        )
        .await
        .wrap_err_with(|| format!("Failed to delete connection profile for '{ssid}'"))?;

        Ok(())
    }

    async fn current_connection(&self) -> Result<Option<ConnectionInfo>> {
        let active_conn_path: OwnedObjectPath = match Self::get_property(
            &self.conn,
            self.wifi_device_path.as_str(),
            "org.freedesktop.NetworkManager.Device",
            "ActiveConnection",
        )
        .await
        {
            Ok(p) => p,
            Err(_) => return Ok(None),
        };

        if active_conn_path.as_str() == "/" {
            return Ok(None);
        }

        let ssid = self.get_active_ssid().await.unwrap_or_default();

        // Get IP4 config
        let ip4_path: OwnedObjectPath = Self::get_property(
            &self.conn,
            self.wifi_device_path.as_str(),
            "org.freedesktop.NetworkManager.Device",
            "Ip4Config",
        )
        .await
        .unwrap_or_else(|_| OwnedObjectPath::try_from("/").unwrap());

        let ip4 = if ip4_path.as_str() != "/" {
            // Get address data
            let addr_data: Vec<HashMap<String, OwnedValue>> = Self::get_property(
                &self.conn,
                ip4_path.as_str(),
                "org.freedesktop.NetworkManager.IP4Config",
                "AddressData",
            )
            .await
            .unwrap_or_default();

            addr_data
                .first()
                .and_then(|a| a.get("address"))
                .and_then(|v| String::try_from(v.clone()).ok())
        } else {
            None
        };

        let gateway: Option<String> = if ip4_path.as_str() != "/" {
            Self::get_property(
                &self.conn,
                ip4_path.as_str(),
                "org.freedesktop.NetworkManager.IP4Config",
                "Gateway",
            )
            .await
            .ok()
        } else {
            None
        };

        // Get HW address
        let mac: String = Self::get_property(
            &self.conn,
            self.wifi_device_path.as_str(),
            "org.freedesktop.NetworkManager.Device",
            "HwAddress",
        )
        .await
        .unwrap_or_default();

        // Get active AP for signal & frequency
        let active_ap: OwnedObjectPath = Self::get_property(
            &self.conn,
            self.wifi_device_path.as_str(),
            "org.freedesktop.NetworkManager.Device.Wireless",
            "ActiveAccessPoint",
        )
        .await
        .unwrap_or_else(|_| OwnedObjectPath::try_from("/").unwrap());

        let (signal, frequency, bssid) = if active_ap.as_str() != "/" {
            let sig: u8 = Self::get_property(
                &self.conn,
                active_ap.as_str(),
                "org.freedesktop.NetworkManager.AccessPoint",
                "Strength",
            )
            .await
            .unwrap_or(0);

            let freq: u32 = Self::get_property(
                &self.conn,
                active_ap.as_str(),
                "org.freedesktop.NetworkManager.AccessPoint",
                "Frequency",
            )
            .await
            .unwrap_or(0);

            let bss: String = Self::get_property(
                &self.conn,
                active_ap.as_str(),
                "org.freedesktop.NetworkManager.AccessPoint",
                "HwAddress",
            )
            .await
            .unwrap_or_default();

            (sig, freq, bss)
        } else {
            (0, 0, String::new())
        };

        // Get bitrate
        let speed: u32 = Self::get_property(
            &self.conn,
            self.wifi_device_path.as_str(),
            "org.freedesktop.NetworkManager.Device.Wireless",
            "Bitrate",
        )
        .await
        .unwrap_or(0)
            / 1000; // Convert from kbit/s to Mbit/s

        Ok(Some(ConnectionInfo {
            ssid,
            bssid,
            ip4,
            ip6: None,
            gateway,
            dns: Vec::new(),
            mac,
            speed,
            frequency,
            signal,
            interface: self.interface.clone(),
        }))
    }

    async fn connect_hidden(&self, ssid: &str, password: Option<&str>) -> Result<()> {
        info!("Connecting to hidden network: {}", ssid);

        let settings = Self::build_connection_settings(ssid, password, true);
        let (_conn_path, _active_conn): (OwnedObjectPath, OwnedObjectPath) = Self::call_nm_method(
            &self.conn,
            "/org/freedesktop/NetworkManager",
            "org.freedesktop.NetworkManager",
            "AddAndActivateConnection",
            &(
                settings,
                &self.wifi_device_path,
                ObjectPath::try_from("/").unwrap(),
            ),
        )
        .await
        .wrap_err_with(|| format!("Failed to connect to hidden network '{ssid}'"))?;

        Ok(())
    }

    fn interface_name(&self) -> &str {
        &self.interface
    }
}
