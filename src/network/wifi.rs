use crate::error::NexusResult;
use crate::network::dbus_proxies::*;
use crate::network::manager::{ov_to_bytes, ov_to_string};
use zbus::zvariant::OwnedObjectPath;
use zbus::Connection;

/// WiFi-specific helper operations
pub struct WifiManager<'a> {
    connection: &'a Connection,
}

impl<'a> WifiManager<'a> {
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// Find the first WiFi device path
    pub async fn find_wifi_device(&self) -> NexusResult<Option<OwnedObjectPath>> {
        let nm_proxy = NetworkManagerProxy::new(self.connection).await?;
        let devices = nm_proxy.get_devices().await?;

        for path in devices {
            let dev_proxy = DeviceProxy::builder(self.connection)
                .path(path.clone())?
                .build()
                .await?;

            if dev_proxy.device_type().await.unwrap_or(0) == 2 {
                // WiFi
                return Ok(Some(path));
            }
        }

        Ok(None)
    }

    /// Check if the given SSID has a saved connection profile
    pub async fn find_saved_connection_for_ssid(
        &self,
        ssid: &str,
    ) -> NexusResult<Option<OwnedObjectPath>> {
        let settings_proxy = SettingsProxy::new(self.connection).await?;
        let connections = settings_proxy.list_connections().await?;

        for conn_path in connections {
            let conn_proxy = ConnectionSettingsProxy::builder(self.connection)
                .path(conn_path.clone())?
                .build()
                .await?;

            if let Ok(settings) = conn_proxy.get_settings().await {
                // Check if it's a WiFi connection
                if let Some(conn) = settings.get("connection") {
                    let conn_type = conn.get("type").and_then(ov_to_string);
                    if conn_type.as_deref() != Some("802-11-wireless") {
                        continue;
                    }
                }

                // Check SSID match
                if let Some(wifi) = settings.get("802-11-wireless") {
                    if let Some(ssid_val) = wifi.get("ssid") {
                        if let Some(bytes) = ov_to_bytes(ssid_val) {
                            let saved_ssid = String::from_utf8_lossy(&bytes);
                            if saved_ssid == ssid {
                                return Ok(Some(conn_path));
                            }
                        }
                    }
                }
            }
        }

        Ok(None)
    }
}
