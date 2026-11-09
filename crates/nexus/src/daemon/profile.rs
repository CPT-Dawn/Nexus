//! Profile settings ops, and the one change that may wait for confirmation.
//!
//! An edit to the Profile carrying the current Link runs under a Backend
//! checkpoint (ADR-0004). The Daemon remembers that change here until it is
//! confirmed, reverted, or its timer runs out; the Backend rolls back by
//! itself at the same moment, so a Daemon that dies cannot strand the change.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use nexus_ipc::{
    ErrorCode, Event, Op, PendingChange, ProfileApplied, Request, Response, Settled, profile,
};
use tokio::sync::{Mutex, broadcast};

use super::backend_reply;
use crate::backend::{Backend, BackendError};
use crate::text;

/// How long a change to the active Profile waits for "Keep these settings?".
pub const ROLLBACK_SECS: u32 = 15;
/// The longest wait a Front end may ask for.
const ROLLBACK_MAX: u32 = 300;

/// The change waiting for confirmation, if any.
#[derive(Debug)]
struct Open {
    id: u64,
    ssid: String,
    checkpoint: String,
}

/// Holds the pending change and tells subscribers how it ends.
#[derive(Debug)]
pub struct Gate {
    events: broadcast::Sender<Event>,
    open: Mutex<Option<Open>>,
    next: AtomicU64,
}

impl Gate {
    pub fn new(events: broadcast::Sender<Event>) -> Self {
        Self {
            events,
            open: Mutex::new(None),
            next: AtomicU64::new(1),
        }
    }

    fn settled(&self, open: &Open, outcome: Settled) {
        let _ = self.events.send(Event::ProfileSettled {
            id: open.id,
            ssid: open.ssid.clone(),
            outcome,
        });
    }
}

pub fn handles(op: &Op) -> bool {
    matches!(
        op,
        Op::ProfileGet { .. }
            | Op::ProfileUpdate { .. }
            | Op::ProfileConfirm { .. }
            | Op::ProfileRevert { .. }
    )
}

fn failure(id: u64, f: text::Failure) -> Response {
    Response::err(Some(id), f.code, f.message)
}

fn done(id: u64) -> Response {
    Response::ok(id, serde_json::json!({}))
}

pub async fn answer<B: Backend>(backend: &Arc<B>, gate: &Arc<Gate>, req: Request) -> Response {
    let id = req.id;
    match req.op {
        Op::ProfileGet { network } => backend_reply(id, backend.profile_get(network).await),
        Op::ProfileUpdate {
            network,
            version,
            settings,
            rollback_secs,
        } => {
            let problems: Vec<String> = profile::validate(&settings)
                .into_iter()
                .map(|e| e.message)
                .collect();
            if !problems.is_empty() {
                return failure(id, text::invalid_settings(&problems));
            }
            let secs = rollback_secs
                .filter(|s| *s > 0)
                .unwrap_or(ROLLBACK_SECS)
                .min(ROLLBACK_MAX);
            update(backend, gate, id, network, version, settings, secs).await
        }
        Op::ProfileConfirm { pending } => settle(backend, gate, id, pending, true).await,
        Op::ProfileRevert { pending } => settle(backend, gate, id, pending, false).await,
        _ => Response::err(Some(id), ErrorCode::UnknownOp, "op not supported"),
    }
}

async fn update<B: Backend>(
    backend: &Arc<B>,
    gate: &Arc<Gate>,
    id: u64,
    network: String,
    version: u64,
    settings: nexus_ipc::ProfileSettings,
    secs: u32,
) -> Response {
    // Held across the Backend call: a second edit cannot slip in between.
    let mut open = gate.open.lock().await;
    if let Some(waiting) = open.as_ref() {
        return failure(id, text::change_waiting(&waiting.ssid));
    }
    let changed = match backend
        .profile_update(network, version, settings, secs)
        .await
    {
        Ok(c) => c,
        Err(e) => return backend_reply::<()>(id, Err(e)),
    };
    let mut applied = ProfileApplied {
        ssid: changed.ssid.clone(),
        version: changed.version,
        pending: None,
    };
    if let Some(checkpoint) = changed.checkpoint {
        let pending_id = gate.next.fetch_add(1, Ordering::Relaxed);
        let change = PendingChange {
            id: pending_id,
            ssid: changed.ssid.clone(),
            rollback_secs: secs,
        };
        *open = Some(Open {
            id: pending_id,
            ssid: changed.ssid,
            checkpoint,
        });
        let _ = gate.events.send(Event::ProfilePending {
            change: change.clone(),
        });
        applied.pending = Some(change);
        tokio::spawn(expire(
            backend.clone(),
            gate.clone(),
            pending_id,
            Duration::from_secs(u64::from(secs)),
        ));
    }
    Response::ok(
        id,
        serde_json::to_value(applied).expect("wire types serialize"),
    )
}

/// The clock ran out: put the old settings back, unless somebody settled the
/// change meanwhile.
async fn expire<B: Backend>(backend: Arc<B>, gate: Arc<Gate>, pending: u64, after: Duration) {
    tokio::time::sleep(after).await;
    let mut open = gate.open.lock().await;
    if open.as_ref().is_none_or(|o| o.id != pending) {
        return;
    }
    let Some(waiting) = open.take() else { return };
    // The Backend usually rolled back by itself at this very moment.
    if let Err(e) = backend
        .checkpoint_rollback(waiting.checkpoint.clone())
        .await
    {
        tracing::debug!("rollback of {}: {e}", waiting.ssid);
    }
    gate.settled(&waiting, Settled::RolledBack);
}

async fn settle<B: Backend>(
    backend: &Arc<B>,
    gate: &Arc<Gate>,
    id: u64,
    pending: u64,
    keep: bool,
) -> Response {
    let mut open = gate.open.lock().await;
    if open.as_ref().is_none_or(|o| o.id != pending) {
        return failure(id, text::nothing_pending());
    }
    let Some(waiting) = open.take() else {
        return failure(id, text::nothing_pending());
    };
    if keep {
        match backend.checkpoint_confirm(waiting.checkpoint.clone()).await {
            Ok(()) => {
                gate.settled(&waiting, Settled::Confirmed);
                done(id)
            }
            Err(BackendError::Unavailable) => {
                // NetworkManager is gone; its checkpoints went with it.
                gate.settled(&waiting, Settled::RolledBack);
                backend_reply::<()>(id, Err(BackendError::Unavailable))
            }
            Err(_) => {
                // The Backend rolled back first: too late to keep it.
                gate.settled(&waiting, Settled::RolledBack);
                failure(id, text::nothing_pending())
            }
        }
    } else {
        match backend
            .checkpoint_rollback(waiting.checkpoint.clone())
            .await
        {
            Ok(()) | Err(BackendError::Failed(_)) => {
                gate.settled(&waiting, Settled::RolledBack);
                done(id)
            }
            Err(e) => {
                gate.settled(&waiting, Settled::RolledBack);
                backend_reply::<()>(id, Err(e))
            }
        }
    }
}
