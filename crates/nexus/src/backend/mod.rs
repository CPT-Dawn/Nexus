//! The domain-level Backend interface (ADR-0001). The Daemon depends on this
//! trait, never on NetworkManager types, so another Backend can be added later.

pub mod nm;
mod wifi;

use std::future::Future;

use nexus_ipc::{ScanOutcome, Status, WifiList};

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

/// Why a Backend call failed. For `status`, "not running" is a [`Status`]
/// instead; the other calls have nothing to show without the Backend.
#[derive(Debug)]
pub enum BackendError {
    /// The Backend is not running or not reachable.
    Unavailable,
    Failed(String),
}

impl std::fmt::Display for BackendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BackendError::Unavailable => f.write_str("NetworkManager is not running"),
            BackendError::Failed(msg) => f.write_str(msg),
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

    /// Byte counters of the primary Link's Device, `None` without one.
    fn counters(&self) -> impl Future<Output = Result<Option<Counters>, BackendError>> + Send;

    /// Resolves after the Backend changed, once per burst: a storm of
    /// property changes is coalesced into a single tick. Cancel-safe.
    fn changed(&self) -> impl Future<Output = ()> + Send;
}
