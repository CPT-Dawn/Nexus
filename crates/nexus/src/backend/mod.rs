//! The domain-level Backend interface (ADR-0001). The Daemon depends on this
//! trait, never on NetworkManager types, so another Backend can be added later.

mod agent;
pub mod nm;
mod tether;
mod wifi;

use std::future::Future;

use nexus_ipc::{
    AutoconnectSet, Disconnected, Forgotten, Joined, Radios, ScanOutcome, Secret, Status, Tethered,
    Tethers, Untethered, WifiList,
};

use crate::text::Failure;

/// What a Backend can do, so Front ends can hide what is not there.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[allow(dead_code)] // read by Front-end-facing ops in later tickets
pub struct Capabilities {
    /// Reports whether the internet is actually reachable.
    pub connectivity_check: bool,
    /// Can tell a Portal from a plain outage.
    pub portal_detection: bool,
}

/// Byte counters of the Device carrying the primary Link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Counters {
    /// Identifies the Device, so a change of Link restarts the baseline.
    pub device: String,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}

/// What the Doctor asks of the Backend, read fresh on each call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetFacts {
    /// The Backend's own version, when it says.
    pub version: Option<String>,
    /// The software radio switch for Wi-Fi.
    pub wireless_enabled: bool,
    /// The hardware radio switch for Wi-Fi.
    pub wireless_hw_enabled: bool,
    /// Whether the Backend probes internet reachability.
    pub connectivity_check: bool,
    /// The primary Link's IPv4 gateway.
    pub gateway4: Option<String>,
    /// Whether the primary Link has an IPv6 address.
    pub ipv6: bool,
}

/// Why a Backend call failed. For `status`, "not running" is a [`Status`]
/// instead; the other calls have nothing to show without the Backend.
#[derive(Debug)]
pub enum BackendError {
    /// The Backend is not running or not reachable.
    Unavailable,
    Failed(String),
    /// The Backend (or Nexus) turned the request down, with words for the person.
    Refused(Failure),
}

impl std::fmt::Display for BackendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BackendError::Unavailable => f.write_str("NetworkManager is not running"),
            BackendError::Failed(msg) => f.write_str(msg),
            BackendError::Refused(failure) => f.write_str(&failure.message),
        }
    }
}

/// `CLOCK_BOOTTIME` in milliseconds: the clock NetworkManager stamps scans with.
pub fn boottime_ms() -> i64 {
    let t = rustix::time::clock_gettime(rustix::time::ClockId::Boottime);
    t.tv_sec * 1000 + t.tv_nsec / 1_000_000
}

pub trait Backend: Send + Sync + 'static {
    #[allow(dead_code)]
    fn capabilities(&self) -> Capabilities;

    /// Overall state and the primary Link. A Backend that is not running
    /// yields `State::Unavailable`, not an error.
    fn status(&self) -> impl Future<Output = Result<Status, BackendError>> + Send;

    /// Nearby Networks (one per SSID) and saved Networks out of range.
    fn wifi_list(&self) -> impl Future<Output = Result<WifiList, BackendError>> + Send;

    /// Refreshes the Wi-Fi list, reusing recent results instead of
    /// re-requesting inside the Backend's rate limit.
    fn wifi_scan(&self) -> impl Future<Output = Result<ScanOutcome, BackendError>> + Send;

    /// What the Doctor needs to know about the Backend and the primary Link.
    fn net_facts(&self) -> impl Future<Output = Result<NetFacts, BackendError>> + Send;

    /// Byte counters of the primary Link's Device, `None` without one.
    fn counters(&self) -> impl Future<Output = Result<Option<Counters>, BackendError>> + Send;
    /// Joins a Wi-Fi Network (`network` is a `WifiNetwork.id`) and waits
    /// until it is up or has failed. A Profile exists afterwards only if the
    /// join worked.
    fn wifi_join(
        &self,
        network: String,
        password: Option<Secret>,
    ) -> impl Future<Output = Result<Joined, BackendError>> + Send;

    /// Drops the Wi-Fi Link, keeping its Profile.
    fn wifi_disconnect(&self) -> impl Future<Output = Result<Disconnected, BackendError>> + Send;

    /// Deletes the Profile(s) of a Network.
    fn wifi_forget(
        &self,
        network: String,
    ) -> impl Future<Output = Result<Forgotten, BackendError>> + Send;

    /// Turns automatic joining of a saved Network on or off.
    fn wifi_autoconnect(
        &self,
        network: String,
        enabled: bool,
    ) -> impl Future<Output = Result<AutoconnectSet, BackendError>> + Send;

    /// Bluetooth's state and the paired phones that offer network access.
    fn tethers(&self) -> impl Future<Output = Result<Tethers, BackendError>> + Send;

    /// Tethers through a phone (address or name; the only one when `None`) and
    /// waits until the Link is up or has failed. A Profile exists afterwards
    /// only if it worked. Refused with `bluetooth_off` or `no_phone`.
    fn tether_connect(
        &self,
        phone: Option<String>,
    ) -> impl Future<Output = Result<Tethered, BackendError>> + Send;

    /// Drops the Tethering Link, keeping the Profile.
    fn tether_disconnect(&self) -> impl Future<Output = Result<Untethered, BackendError>> + Send;

    /// Switches Wi-Fi on or off. Refused with `hard_blocked` while a hardware
    /// switch holds it off.
    fn wifi_power(&self, on: bool) -> impl Future<Output = Result<Radios, BackendError>> + Send;

    /// Airplane mode: every radio off (`on`) or back on.
    fn airplane(&self, on: bool) -> impl Future<Output = Result<Radios, BackendError>> + Send;

    /// Starts listening for [`Backend::changed`] and [`Backend::resumed`]
    /// now, so nothing that happens between this call and the first wait on
    /// them is lost. Best effort: they retry on their own if this fails.
    fn arm(&self) -> impl Future<Output = ()> + Send;

    /// Resolves each time the machine wakes from sleep, when the Backend's
    /// picture of the world may be stale. Cancel-safe.
    fn resumed(&self) -> impl Future<Output = ()> + Send;

    /// Answers the Backend's requests for secrets through Secret prompts, for
    /// as long as the Daemon runs: registers with the Backend, and again
    /// whenever it comes back. Never returns.
    fn secrets(&self) -> impl Future<Output = ()> + Send;

    /// Resolves after the Backend changed, once per burst: a storm of
    /// property changes is coalesced into a single tick. Cancel-safe.
    fn changed(&self) -> impl Future<Output = ()> + Send;
}
