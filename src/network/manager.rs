use std::collections::HashMap;
use tracing::{debug, info, warn};
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};
use zbus::Connection;

use crate::error::{NexusError, NexusResult};
use crate::network::dbus_proxies::*;
use crate::network::types::*;

/// High-level facade over NetworkManager's D-Bus API.
/// All network operations go through this struct.
pub struct NetworkManager {
    connection: Connection,
}

impl NetworkManager {
    /// Connect to the system D-Bus and create the NetworkManager facade
    pub async fn new() -> NexusResult<Self> {
        let connection = Connection::system().await?;
        Ok(Self { connection })
    }

    /// Check if NetworkManager is running
    pub async fn is_running(&self) -> bool {
        let proxy = match NetworkManagerProxy::new(&self.connection).await {
            Ok(p) => p,
            Err(_) => return false,
        };
        proxy.version().await.is_ok()
    }

    /// Get NM version string
    pub async fn version(&self) -> NexusResult<String> {
        let proxy = NetworkManagerProxy::new(&self.connection).await?;
        Ok(proxy.version().await?)
    }

    /// Get overall connectivity state
    pub async fn connectivity(&self) -> NexusResult<ConnectivityState> {
        let proxy = NetworkManagerProxy::new(&self.connection).await?;
        let val = proxy.connectivity().await?;
        Ok(ConnectivityState::from_nm(val))
    }

    /// Check if wireless is enabled
    pub async fn wireless_enabled(&self) -> NexusResult<bool> {
        let proxy = NetworkManagerProxy::new(&self.connection).await?;
        Ok(proxy.wireless_enabled().await?)
    }

    /// Toggle wireless on/off
    pub async fn set_wireless_enabled(&self, enabled: bool) -> NexusResult<()> {
        let proxy = NetworkManagerProxy::new(&self.connection).await?;
        proxy.set_wireless_enabled(enabled).await?;
        Ok(())
    }

    /// Check if networking is enabled
    pub async fn networking_enabled(&self) -> NexusResult<bool> {
        let proxy = NetworkManagerProxy::new(&self.connection).await?;
        Ok(proxy.networking_enabled().await?)
    }

    /// Get the hostname
    pub async fn hostname(&self) -> NexusResult<String> {
        let proxy = SettingsProxy::new(&self.connection).await?;
        Ok(proxy.hostname().await?)
    }

    // ── Device Operations ─────────────────────────────────────────────

    /// List all network devices with their info
    pub async fn list_devices(&self) -> NexusResult<Vec<DeviceInfo>> {
        let nm_proxy = NetworkManagerProxy::new(&self.connection).await?;
        let device_paths = nm_proxy.get_devices().await?;
        let mut devices = Vec::new();

        for path in device_paths {
            match self.get_device_info(&path).await {
                Ok(info) => devices.push(info),
                Err(e) => {
                    warn!("Failed to get device info for {}: {}", path, e);
                }
            }
        }

        // Sort: connected first, then by name
        devices.sort_by(|a, b| {
            b.state
                .is_connected()
                .cmp(&a.state.is_connected())
                .then_with(|| a.interface.cmp(&b.interface))
        });

        Ok(devices)
    }

    /// Get detailed info for a single device
    pub async fn get_device_info(&self, path: &OwnedObjectPath) -> NexusResult<DeviceInfo> {
        let dev_proxy = DeviceProxy::builder(&self.connection)
            .path(path.clone())?
            .build()
            .await?;

        let interface = dev_proxy.interface().await.unwrap_or_default();
        let device_type = DeviceType::from_nm(dev_proxy.device_type().await.unwrap_or(0));
        let state = DeviceState::from_nm(dev_proxy.state().await.unwrap_or(0));
        let hw_address = dev_proxy.hw_address().await.unwrap_or_default();
        let driver = dev_proxy.driver().await.unwrap_or_default();
        let mtu = dev_proxy.mtu().await.unwrap_or(0);
        let autoconnect = dev_proxy.autoconnect().await.unwrap_or(false);

        // Get active connection path
        let active_connection_path = dev_proxy.active_connection().await.ok().and_then(|p| {
            if p.as_str() == "/" {
                None
            } else {
                Some(p)
            }
        });

        // Get connection name from active connection
        let connection_name = if let Some(ref ac_path) = active_connection_path {
            self.get_active_connection_name(ac_path).await.ok()
        } else {
            None
        };

        // Get IP info
        let (ip4_address, ip4_gateway, ip4_dns, ip4_subnet) =
            self.get_device_ip4_info(&dev_proxy).await;

        let ip6_address = self.get_device_ip6_info(&dev_proxy).await;

        // Get speed for wired devices
        let speed = if device_type == DeviceType::Ethernet {
            self.get_wired_speed(path).await.unwrap_or(0)
        } else if device_type == DeviceType::WiFi {
            self.get_wireless_bitrate(path).await.unwrap_or(0) / 1000 // kbit/s -> Mbit/s
        } else {
            0
        };

        Ok(DeviceInfo {
            path: path.clone(),
            interface,
            device_type,
            state,
            hw_address,
            ip4_address,
            ip4_gateway,
            ip4_dns,
            ip4_subnet,
            ip6_address,
            mtu,
            speed,
            driver,
            active_connection_path,
            connection_name,
            autoconnect,
        })
    }

    async fn get_device_ip4_info(
        &self,
        dev_proxy: &DeviceProxy<'_>,
    ) -> (Option<String>, Option<String>, Vec<String>, Option<String>) {
        let ip4_path = match dev_proxy.ip4_config().await {
            Ok(p) if p.as_str() != "/" => p,
            _ => return (None, None, Vec::new(), None),
        };

        let ip4_proxy = match IP4ConfigProxy::builder(&self.connection)
            .path(ip4_path)
            .ok()
            .map(|b| b.build())
        {
            Some(fut) => match fut.await {
                Ok(p) => p,
                Err(_) => return (None, None, Vec::new(), None),
            },
            None => return (None, None, Vec::new(), None),
        };

        let address = ip4_proxy.address_data().await.ok().and_then(|addrs| {
            addrs
                .first()
                .and_then(|a| a.get("address").and_then(|v| ov_to_string(v)))
        });

        let subnet = ip4_proxy.address_data().await.ok().and_then(|addrs| {
            addrs.first().and_then(|a| {
                a.get("prefix")
                    .and_then(|v| ov_to_u32(v).map(|p| format!("/{}", p)))
            })
        });

        let gateway =
            ip4_proxy
                .gateway()
                .await
                .ok()
                .and_then(|g| if g.is_empty() { None } else { Some(g) });

        let dns = ip4_proxy
            .nameserver_data()
            .await
            .ok()
            .map(|servers| {
                servers
                    .iter()
                    .filter_map(|s| s.get("address").and_then(|v| ov_to_string(v)))
                    .collect()
            })
            .unwrap_or_default();

        (address, gateway, dns, subnet)
    }

    async fn get_device_ip6_info(&self, dev_proxy: &DeviceProxy<'_>) -> Option<String> {
        let ip6_path = match dev_proxy.ip6_config().await {
            Ok(p) if p.as_str() != "/" => p,
            _ => return None,
        };

        let ip6_proxy = IP6ConfigProxy::builder(&self.connection)
            .path(ip6_path)
            .ok()?
            .build()
            .await
            .ok()?;

        ip6_proxy.address_data().await.ok().and_then(|addrs| {
            addrs
                .first()
                .and_then(|a| a.get("address").and_then(|v| ov_to_string(v)))
        })
    }

    async fn get_wired_speed(&self, path: &OwnedObjectPath) -> NexusResult<u32> {
        let proxy = WiredProxy::builder(&self.connection)
            .path(path.clone())?
            .build()
            .await?;
        Ok(proxy.speed().await?)
    }

    async fn get_wireless_bitrate(&self, path: &OwnedObjectPath) -> NexusResult<u32> {
        let proxy = WirelessProxy::builder(&self.connection)
            .path(path.clone())?
            .build()
            .await?;
        Ok(proxy.bitrate().await?)
    }

    /// Disconnect a device
    pub async fn disconnect_device(&self, device_path: &OwnedObjectPath) -> NexusResult<()> {
        let proxy = DeviceProxy::builder(&self.connection)
            .path(device_path.clone())?
            .build()
            .await?;
        proxy.disconnect().await?;
        info!("Disconnected device: {}", device_path);
        Ok(())
    }

    // ── Active Connection Operations ──────────────────────────────────

    /// Get all active connections
    pub async fn list_active_connections(&self) -> NexusResult<Vec<ActiveConnection>> {
        let nm_proxy = NetworkManagerProxy::new(&self.connection).await?;
        let paths = nm_proxy.active_connections().await?;
        let mut connections = Vec::new();

        for path in paths {
            match self.get_active_connection_info(&path).await {
                Ok(info) => connections.push(info),
                Err(e) => {
                    warn!("Failed to get active connection {}: {}", path, e);
                }
            }
        }

        Ok(connections)
    }

    async fn get_active_connection_info(
        &self,
        path: &OwnedObjectPath,
    ) -> NexusResult<ActiveConnection> {
        let proxy = ActiveConnectionProxy::builder(&self.connection)
            .path(path.clone())?
            .build()
            .await?;

        Ok(ActiveConnection {
            path: path.clone(),
            id: proxy.id().await.unwrap_or_default(),
            uuid: proxy.uuid().await.unwrap_or_default(),
            conn_type: proxy.connection_type().await.unwrap_or_default(),
            state: ActiveConnectionState::from_nm(proxy.state().await.unwrap_or(0)),
            devices: proxy.devices().await.unwrap_or_default(),
        })
    }

    async fn get_active_connection_name(&self, path: &OwnedObjectPath) -> NexusResult<String> {
        let proxy = ActiveConnectionProxy::builder(&self.connection)
            .path(path.clone())?
            .build()
            .await?;
        Ok(proxy.id().await?)
    }

    /// Deactivate an active connection
    pub async fn deactivate_connection(
        &self,
        active_conn_path: &OwnedObjectPath,
    ) -> NexusResult<()> {
        let nm_proxy = NetworkManagerProxy::new(&self.connection).await?;
        nm_proxy
            .deactivate_connection(&active_conn_path.as_ref())
            .await?;
        info!("Deactivated connection: {}", active_conn_path);
        Ok(())
    }

    // ── Saved Connection Operations ───────────────────────────────────

    /// List all saved connection profiles
    pub async fn list_saved_connections(&self) -> NexusResult<Vec<ConnectionProfile>> {
        let settings_proxy = SettingsProxy::new(&self.connection).await?;
        let paths = settings_proxy.list_connections().await?;
        let mut connections = Vec::new();

        for path in paths {
            match self.get_connection_profile(&path).await {
                Ok(profile) => connections.push(profile),
                Err(e) => {
                    warn!("Failed to get connection profile {}: {}", path, e);
                }
            }
        }

        // Sort by last used (most recent first)
        connections.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

        Ok(connections)
    }

    async fn get_connection_profile(
        &self,
        path: &OwnedObjectPath,
    ) -> NexusResult<ConnectionProfile> {
        let proxy = ConnectionSettingsProxy::builder(&self.connection)
            .path(path.clone())?
            .build()
            .await?;

        let settings = proxy.get_settings().await?;

        let connection_settings = settings
            .get("connection")
            .ok_or_else(|| NexusError::Connection("Missing 'connection' section".into()))?;

        let id = extract_string(connection_settings, "id").unwrap_or_default();
        let uuid = extract_string(connection_settings, "uuid").unwrap_or_default();
        let conn_type = extract_string(connection_settings, "type").unwrap_or_default();
        let interface = extract_string(connection_settings, "interface-name");
        let autoconnect = extract_bool(connection_settings, "autoconnect").unwrap_or(true);
        let timestamp = extract_u64(connection_settings, "timestamp").unwrap_or(0);

        Ok(ConnectionProfile {
            path: path.clone(),
            id,
            uuid,
            conn_type,
            interface,
            autoconnect,
            timestamp,
        })
    }

    /// Activate a saved connection on a specific device
    pub async fn activate_connection(
        &self,
        conn_path: &OwnedObjectPath,
        device_path: &OwnedObjectPath,
    ) -> NexusResult<OwnedObjectPath> {
        let nm_proxy = NetworkManagerProxy::new(&self.connection).await?;
        let root: OwnedObjectPath = ObjectPath::try_from("/").unwrap().into();
        let active = nm_proxy
            .activate_connection(&conn_path.as_ref(), &device_path.as_ref(), &root.as_ref())
            .await?;
        info!("Activated connection {} on {}", conn_path, device_path);
        Ok(active)
    }

    /// Delete a saved connection
    pub async fn delete_connection(&self, conn_path: &OwnedObjectPath) -> NexusResult<()> {
        let proxy = ConnectionSettingsProxy::builder(&self.connection)
            .path(conn_path.clone())?
            .build()
            .await?;
        proxy.delete().await?;
        info!("Deleted connection: {}", conn_path);
        Ok(())
    }

    // ── WiFi Operations ───────────────────────────────────────────────

    /// Request a WiFi scan on a wireless device
    pub async fn request_wifi_scan(&self, device_path: &OwnedObjectPath) -> NexusResult<()> {
        let proxy = WirelessProxy::builder(&self.connection)
            .path(device_path.clone())?
            .build()
            .await?;
        proxy.request_scan(HashMap::new()).await?;
        debug!("WiFi scan requested on {}", device_path);
        Ok(())
    }

    /// Get all visible WiFi access points for a wireless device
    pub async fn list_access_points(
        &self,
        device_path: &OwnedObjectPath,
    ) -> NexusResult<Vec<WifiAccessPoint>> {
        let proxy = WirelessProxy::builder(&self.connection)
            .path(device_path.clone())?
            .build()
            .await?;

        let active_ap = proxy.active_access_point().await.ok().and_then(|p| {
            if p.as_str() == "/" {
                None
            } else {
                Some(p)
            }
        });

        let ap_paths = proxy.get_all_access_points().await?;

        // Get saved SSIDs for marking saved networks
        let saved = self.list_saved_connections().await.unwrap_or_default();
        let saved_ssids: std::collections::HashSet<String> = saved
            .iter()
            .filter(|c| c.conn_type == "802-11-wireless")
            .map(|c| c.id.clone())
            .collect();

        let mut access_points = Vec::new();
        for ap_path in &ap_paths {
            match self
                .get_access_point_info(ap_path, &active_ap, &saved_ssids)
                .await
            {
                Ok(ap) if !ap.ssid.is_empty() => access_points.push(ap),
                Ok(_) => {} // Skip hidden networks
                Err(e) => {
                    debug!("Failed to get AP info for {}: {}", ap_path, e);
                }
            }
        }

        // Sort by signal strength (best first), active first
        access_points.sort_by(|a, b| {
            b.is_active
                .cmp(&a.is_active)
                .then_with(|| b.strength.cmp(&a.strength))
        });

        // Deduplicate by SSID (keep strongest signal for each)
        let mut seen = std::collections::HashSet::new();
        access_points.retain(|ap| seen.insert(ap.ssid.clone()));

        Ok(access_points)
    }

    async fn get_access_point_info(
        &self,
        path: &OwnedObjectPath,
        active_ap: &Option<OwnedObjectPath>,
        saved_ssids: &std::collections::HashSet<String>,
    ) -> NexusResult<WifiAccessPoint> {
        let proxy = AccessPointProxy::builder(&self.connection)
            .path(path.clone())?
            .build()
            .await?;

        let ssid_bytes = proxy.ssid().await?;
        let ssid = String::from_utf8_lossy(&ssid_bytes).to_string();
        let bssid = proxy.hw_address().await.unwrap_or_default();
        let frequency = proxy.frequency().await.unwrap_or(0);
        let strength = proxy.strength().await.unwrap_or(0);
        let flags = proxy.flags().await.unwrap_or(0);
        let wpa_flags = proxy.wpa_flags().await.unwrap_or(0);
        let rsn_flags = proxy.rsn_flags().await.unwrap_or(0);

        let is_active = active_ap.as_ref().map(|ap| ap == path).unwrap_or(false);
        let is_saved = saved_ssids.contains(&ssid);

        Ok(WifiAccessPoint {
            path: path.clone(),
            ssid: ssid.clone(),
            bssid,
            frequency,
            channel: WifiAccessPoint::freq_to_channel(frequency),
            strength,
            security: WifiSecurity::from_nm_flags(flags, wpa_flags, rsn_flags),
            is_saved,
            is_active,
        })
    }

    /// Connect to a WiFi network (creates and activates a new connection)
    pub async fn connect_wifi(
        &self,
        device_path: &OwnedObjectPath,
        ap_path: &OwnedObjectPath,
        ssid: &str,
        password: Option<&str>,
        security: WifiSecurity,
    ) -> NexusResult<OwnedObjectPath> {
        let nm_proxy = NetworkManagerProxy::new(&self.connection).await?;

        let mut connection: HashMap<String, HashMap<String, OwnedValue>> = HashMap::new();

        // Connection settings
        let mut conn_settings: HashMap<String, OwnedValue> = HashMap::new();
        conn_settings.insert("id".into(), Value::from(ssid).try_into().unwrap());
        conn_settings.insert(
            "type".into(),
            Value::from("802-11-wireless").try_into().unwrap(),
        );
        conn_settings.insert("autoconnect".into(), Value::from(true).try_into().unwrap());
        connection.insert("connection".into(), conn_settings);

        // WiFi settings
        let mut wifi_settings: HashMap<String, OwnedValue> = HashMap::new();
        wifi_settings.insert(
            "ssid".into(),
            Value::from(ssid.as_bytes().to_vec()).try_into().unwrap(),
        );
        wifi_settings.insert(
            "mode".into(),
            Value::from("infrastructure").try_into().unwrap(),
        );
        connection.insert("802-11-wireless".into(), wifi_settings);

        // Security settings
        if security != WifiSecurity::Open {
            let mut sec_settings: HashMap<String, OwnedValue> = HashMap::new();
            match security {
                WifiSecurity::WPA | WifiSecurity::WPA2 | WifiSecurity::WPA3 => {
                    sec_settings.insert(
                        "key-mgmt".into(),
                        Value::from("wpa-psk").try_into().unwrap(),
                    );
                    if let Some(pw) = password {
                        sec_settings.insert("psk".into(), Value::from(pw).try_into().unwrap());
                    }
                }
                WifiSecurity::WEP => {
                    sec_settings.insert("key-mgmt".into(), Value::from("none").try_into().unwrap());
                    if let Some(pw) = password {
                        sec_settings.insert("wep-key0".into(), Value::from(pw).try_into().unwrap());
                    }
                }
                _ => {
                    sec_settings.insert(
                        "key-mgmt".into(),
                        Value::from("wpa-psk").try_into().unwrap(),
                    );
                    if let Some(pw) = password {
                        sec_settings.insert("psk".into(), Value::from(pw).try_into().unwrap());
                    }
                }
            }
            connection.insert("802-11-wireless-security".into(), sec_settings);

            // Also link the security to the wifi section
            connection.get_mut("802-11-wireless").unwrap().insert(
                "security".into(),
                Value::from("802-11-wireless-security").try_into().unwrap(),
            );
        }

        // IPv4 settings (auto/DHCP)
        let mut ipv4_settings: HashMap<String, OwnedValue> = HashMap::new();
        ipv4_settings.insert("method".into(), Value::from("auto").try_into().unwrap());
        connection.insert("ipv4".into(), ipv4_settings);

        // IPv6 settings (auto)
        let mut ipv6_settings: HashMap<String, OwnedValue> = HashMap::new();
        ipv6_settings.insert("method".into(), Value::from("auto").try_into().unwrap());
        connection.insert("ipv6".into(), ipv6_settings);

        let (_settings_path, active_path) = nm_proxy
            .add_and_activate_connection(connection, &device_path.as_ref(), &ap_path.as_ref())
            .await?;

        info!(
            "Connected to WiFi '{}', active connection: {}",
            ssid, active_path
        );
        Ok(active_path)
    }

    // ── DNS Info ──────────────────────────────────────────────────────

    /// Get DNS configuration from active connections
    pub async fn get_dns_info(&self) -> NexusResult<DnsInfo> {
        let devices = self.list_devices().await?;
        let mut servers = Vec::new();
        let mut search_domains = Vec::new();

        for device in &devices {
            if device.state.is_connected() {
                servers.extend(device.ip4_dns.clone());
            }
        }

        // Try to get search domains from IP4Config
        for device in &devices {
            if !device.state.is_connected() {
                continue;
            }
            let dev_proxy = match DeviceProxy::builder(&self.connection)
                .path(device.path.clone())
                .ok()
                .map(|b| b.build())
            {
                Some(fut) => match fut.await {
                    Ok(p) => p,
                    Err(_) => continue,
                },
                None => continue,
            };

            if let Ok(ip4_path) = dev_proxy.ip4_config().await {
                if ip4_path.as_str() != "/" {
                    if let Ok(builder) = IP4ConfigProxy::builder(&self.connection).path(ip4_path) {
                        if let Ok(proxy) = builder.build().await {
                            if let Ok(domains) = proxy.searches().await {
                                search_domains.extend(domains);
                            }
                        }
                    }
                }
            }
        }

        servers.dedup();
        search_domains.dedup();

        Ok(DnsInfo {
            servers,
            search_domains,
        })
    }

    // ── Full State Snapshot ───────────────────────────────────────────

    /// Build a complete snapshot of network state
    pub async fn snapshot(&self) -> NetworkState {
        let nm_running = self.is_running().await;
        if !nm_running {
            return NetworkState {
                nm_running: false,
                ..Default::default()
            };
        }

        let connectivity = self
            .connectivity()
            .await
            .unwrap_or(ConnectivityState::Unknown);
        let wireless_enabled = self.wireless_enabled().await.unwrap_or(false);
        let networking_enabled = self.networking_enabled().await.unwrap_or(false);
        let devices = self.list_devices().await.unwrap_or_default();
        let active_connections = self.list_active_connections().await.unwrap_or_default();
        let saved_connections = self.list_saved_connections().await.unwrap_or_default();
        let dns = self.get_dns_info().await.unwrap_or(DnsInfo {
            servers: Vec::new(),
            search_domains: Vec::new(),
        });
        let hostname = self.hostname().await.unwrap_or_default();
        let nm_version = self.version().await.unwrap_or_default();

        // Get WiFi APs from all wireless devices
        let mut wifi_access_points = Vec::new();
        for device in &devices {
            if device.device_type == DeviceType::WiFi {
                if let Ok(aps) = self.list_access_points(&device.path).await {
                    wifi_access_points.extend(aps);
                }
            }
        }

        NetworkState {
            connectivity,
            wireless_enabled,
            networking_enabled,
            devices,
            active_connections,
            wifi_access_points,
            saved_connections,
            stats: std::collections::HashMap::new(), // Stats filled by poller
            dns,
            hostname,
            nm_version,
            nm_running,
        }
    }
}

// ── Helper functions for extracting values from NM settings dicts ─────

fn extract_string(settings: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    settings.get(key).and_then(|v| ov_to_string(v))
}

fn extract_bool(settings: &HashMap<String, OwnedValue>, key: &str) -> Option<bool> {
    settings.get(key).and_then(|v| ov_to_bool(v))
}

fn extract_u64(settings: &HashMap<String, OwnedValue>, key: &str) -> Option<u64> {
    settings.get(key).and_then(|v| ov_to_u64(v))
}

// ── Safe OwnedValue extraction via pattern matching ───────────────────
// OwnedValue derefs to Value<'static>, so &**v gives &Value to match on.

pub fn ov_to_string(v: &OwnedValue) -> Option<String> {
    match &**v {
        Value::Str(s) => Some(s.to_string()),
        _ => None,
    }
}

fn ov_to_bool(v: &OwnedValue) -> Option<bool> {
    match &**v {
        Value::Bool(b) => Some(*b),
        _ => None,
    }
}

fn ov_to_u32(v: &OwnedValue) -> Option<u32> {
    match &**v {
        Value::U32(n) => Some(*n),
        _ => None,
    }
}

fn ov_to_u64(v: &OwnedValue) -> Option<u64> {
    match &**v {
        Value::U64(n) => Some(*n),
        Value::U32(n) => Some(*n as u64),
        _ => None,
    }
}

pub fn ov_to_bytes(v: &OwnedValue) -> Option<Vec<u8>> {
    match &**v {
        Value::Array(arr) => {
            let mut bytes = Vec::new();
            for item in arr.iter() {
                match item {
                    Value::U8(b) => bytes.push(*b),
                    _ => return None,
                }
            }
            Some(bytes)
        }
        _ => None,
    }
}
