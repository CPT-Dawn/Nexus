//! The one way the TUI acts on the Daemon from a Front end thread: a named
//! thread, its own connection, one op, and the answer in the Daemon's words.
//! The Wi-Fi, VPN and Ethernet tabs all go through here, so a failure to
//! spawn or to reach the Daemon reads the same everywhere.

use std::path::Path;
use std::thread;

use nexus_ipc::{Connection, Failure, Op};

/// Runs `job` on a thread called `name`. A thread that cannot be spawned is
/// logged, not a panic: the person simply gets no answer to that key.
pub(super) fn spawn(name: &str, job: impl FnOnce() + Send + 'static) {
    if let Err(e) = thread::Builder::new().name(name.into()).spawn(job) {
        tracing::warn!("cannot spawn the {name} thread: {e}");
    }
}

/// Sends `op` on a new connection and returns the Daemon's answer, or the
/// sentence to show when there is none.
pub(super) fn call(socket: &Path, op: Op) -> Result<serde_json::Value, String> {
    let mut conn =
        Connection::connect_to(socket).map_err(|_| "Can't reach the Nexus daemon.".to_string())?;
    conn.call(op)
        .map_err(|_| "Lost the Nexus daemon.".to_string())?
        .map_err(|f: Failure| f.message)
}
