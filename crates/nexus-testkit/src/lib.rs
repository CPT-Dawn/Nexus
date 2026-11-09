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
mod fake_nm;
pub mod scenarios;
mod world;

pub use bus::Bus;
pub use fake_nm::{
    ActiveConnection, Ap, ApId, DeviceId, FakeNm, NmConnectivity, NmState, SCAN_RATE_LIMIT_MS,
    boottime_ms,
};
pub use world::{PATIENCE, Wire, World, WorldBuilder};
