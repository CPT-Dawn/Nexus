//! The domain-level Backend interface (ADR-0001). The Daemon depends on this
//! trait, never on NetworkManager types, so another Backend can be added later.

pub mod nm;

use std::future::Future;

use nexus_ipc::Status;

/// What a Backend can do, so Front ends can hide what is not there.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[allow(dead_code)] // read by Front-end-facing ops in later tickets
pub struct Capabilities {
    /// Reports whether the internet is actually reachable.
    pub connectivity_check: bool,
    /// Can tell a Portal from a plain outage.
    pub portal_detection: bool,
}

/// A failure that is not "the Backend is not running" (that is a [`Status`]).
#[derive(Debug)]
pub struct BackendError(pub String);

impl std::fmt::Display for BackendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

pub trait Backend: Send + Sync + 'static {
    #[allow(dead_code)]
    fn capabilities(&self) -> Capabilities;

    /// Overall state and the primary Link. A Backend that is not running
    /// yields `State::Unavailable`, not an error.
    fn status(&self) -> impl Future<Output = Result<Status, BackendError>> + Send;
}
