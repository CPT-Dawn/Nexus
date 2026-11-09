//! The Wi-Fi interfaces of the fake NetworkManager: `Device.Wireless` and `AccessPoint`.

#![allow(clippy::wildcard_imports)]

use super::*;

pub(super) struct Wireless(pub(super) Shared, pub(super) usize);

#[interface(name = "org.freedesktop.NetworkManager.Device.Wireless")]
impl Wireless {
    #[zbus(property)]
    fn access_points(&self) -> Vec<OwnedObjectPath> {
        self.all()
    }

    #[zbus(property)]
    fn wireless_capabilities(&self) -> u32 {
        hotspot::capabilities(self.0.lock().unwrap().devices[self.1].ap_capable)
    }

    #[zbus(property)]
    fn last_scan(&self) -> i64 {
        self.0.lock().unwrap().devices[self.1].last_scan
    }

    #[zbus(property)]
    fn active_access_point(&self) -> OwnedObjectPath {
        let m = self.0.lock().unwrap();
        match m.devices[self.1].active_ap {
            Some(id) => opath(&ap_path(id)),
            None => opath("/"),
        }
    }

    fn get_all_access_points(&self) -> Vec<OwnedObjectPath> {
        self.all()
    }

    async fn request_scan(
        &self,
        _options: HashMap<String, OwnedVariant>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<(), DeviceError> {
        {
            let mut m = self.0.lock().unwrap();
            let last = m.devices[self.1].last_scan;
            if m.refuse_scans || (last >= 0 && boottime_ms() - last < SCAN_RATE_LIMIT_MS) {
                return Err(DeviceError::NotAllowed(
                    "Scanning not allowed immediately following previous scan".into(),
                ));
            }
            m.scan_requests += 1;
            m.devices[self.1].last_scan = boottime_ms();
        }
        self.last_scan_changed(&emitter).await?;
        Ok(())
    }

    #[zbus(signal)]
    pub(super) async fn access_point_added(
        emitter: &SignalEmitter<'_>,
        access_point: OwnedObjectPath,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    pub(super) async fn access_point_removed(
        emitter: &SignalEmitter<'_>,
        access_point: OwnedObjectPath,
    ) -> zbus::Result<()>;
}

impl Wireless {
    fn all(&self) -> Vec<OwnedObjectPath> {
        let m = self.0.lock().unwrap();
        m.devices[self.1]
            .aps
            .iter()
            .map(|id| opath(&ap_path(*id)))
            .collect()
    }
}

pub(super) struct AccessPoint(pub(super) Shared, pub(super) u32);

impl AccessPoint {
    fn ap(&self) -> Ap {
        self.0.lock().unwrap().aps[&self.1].clone()
    }
}

#[interface(name = "org.freedesktop.NetworkManager.AccessPoint")]
impl AccessPoint {
    #[zbus(property)]
    fn ssid(&self) -> Vec<u8> {
        let ap = self.ap();
        if ap.hidden { Vec::new() } else { ap.ssid }
    }

    #[zbus(property)]
    fn max_bitrate(&self) -> u32 {
        self.ap().bitrate
    }

    #[zbus(property)]
    fn strength(&self) -> u8 {
        self.ap().strength
    }

    #[zbus(property)]
    fn frequency(&self) -> u32 {
        self.ap().frequency
    }

    #[zbus(property)]
    fn flags(&self) -> u32 {
        self.ap().flags
    }

    #[zbus(property)]
    fn wpa_flags(&self) -> u32 {
        self.ap().wpa_flags
    }

    #[zbus(property)]
    fn rsn_flags(&self) -> u32 {
        self.ap().rsn_flags
    }

    #[zbus(property)]
    fn hw_address(&self) -> String {
        self.ap().bssid
    }
}
