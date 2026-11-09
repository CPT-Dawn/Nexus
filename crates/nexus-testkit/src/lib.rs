//! Test harness S1 (see the spec's "Testing Decisions"): a real `nexus daemon`
//! against a scriptable fake NetworkManager on a private `dbus-daemon`.
//!
//! ```ignore
//! let world = World::start(env!("CARGO_BIN_EXE_nexus"));
//! world.nm.set_state(NmState::ConnectedGlobal);
//! let resp = world.wire().call(json!({"op": "status"}));
//! ```
//!
//! Usage notes for later tickets live in `docs/testing.md`.

mod bus;
mod fake_bluez;
mod fake_nm;
mod fake_notifications;
mod fake_systemd;
mod real;
pub mod scenarios;
mod world;

pub use bus::Bus;
pub use fake_bluez::FakeBluez;
pub use fake_nm::{
    ActiveConnection, Ap, ApId, DeviceId, FakeNm, Lease, NmConnectivity, NmState,
    SCAN_RATE_LIMIT_MS, boottime_ms,
};
pub use fake_notifications::{FakeNotifications, Notified};
pub use fake_systemd::{FakeSystemd, Unit};
pub use real::RealWorld;
pub use world::{PATIENCE, Wire, World, WorldBuilder};
