//! The domain-level Backend interface (ADR-0001). The Daemon depends on this
//! trait, never on NetworkManager types, so another Backend can be added later.

mod agent;
pub mod nm;
mod wifi;

use std::future::Future;

use nexus_ipc::{AutoconnectSet, Disconnected, Forgotten, Joined, ScanOutcome, Secret, Status, WifiList};

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

    /// Answers the Backend's requests for secrets through Secret prompts, for
    /// as long as the Daemon runs: registers with the Backend, and again
    /// whenever it comes back. Never returns.
    fn secrets(&self) -> impl Future<Output = ()> + Send;

    /// Resolves after the Backend changed, once per burst: a storm of
    /// property changes is coalesced into a single tick. Cancel-safe.
    fn changed(&self) -> impl Future<Output = ()> + Send;
}
