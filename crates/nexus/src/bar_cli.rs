//! `nexus status --waybar [--follow]`: talks to the Daemon and prints what
//! `waybar::render` makes of it.

use std::io::Write as _;
use std::process::ExitCode;
use std::time::Duration;

use nexus_ipc::{Connection, Event, Hotspot, Op, Status, VpnList};

use crate::cli::{EXIT_DAEMON_ERROR, unreachable};
use crate::waybar::{self, Output};

/// How long `--follow` waits before it looks for a Daemon that went away.
const RECONNECT: Duration = Duration::from_secs(2);

/// Everything the module is made of.
#[derive(Default)]
struct Picture {
    status: Option<Status>,
    vpns: Option<VpnList>,
    hotspot: Option<Hotspot>,
}

impl Picture {
    fn output(&self) -> Output {
        match &self.status {
            Some(status) => waybar::render(status, self.vpns.as_ref(), self.hotspot.as_ref()),
            None => waybar::unavailable(),
        }
    }

    /// Reads all three from the Daemon.
    fn fetch(conn: &mut Connection) -> std::io::Result<Result<Self, nexus_ipc::Failure>> {
        let status = match conn.status()? {
            Ok(s) => s,
            Err(f) => return Ok(Err(f)),
        };
        // A Daemon that cannot say is not a reason to blank the bar.
        let vpns = conn
            .call(Op::VpnList)?
            .ok()
            .and_then(|v| serde_json::from_value(v).ok());
        let hotspot = conn
            .call(Op::Hotspot)?
            .ok()
            .and_then(|v| serde_json::from_value(v).ok());
        Ok(Ok(Self {
            status: Some(status),
            vpns,
            hotspot,
        }))
    }

    /// Folds an event in; whether it changed anything the module shows.
    fn apply(&mut self, event: Event) -> bool {
        match event {
            Event::StatusChanged { status } => self.status = Some(status),
            Event::VpnsChanged { vpns } => self.vpns = Some(vpns),
            Event::HotspotChanged { hotspot } => self.hotspot = Some(hotspot),
            _ => return false,
        }
        true
    }
}

/// Prints one line; false when stdout is gone (Waybar quit).
fn emit(out: &Output) -> bool {
    let mut stdout = std::io::stdout().lock();
    let line = serde_json::to_string(out).expect("output serializes");
    writeln!(stdout, "{line}")
        .and_then(|()| stdout.flush())
        .is_ok()
}

/// `nexus status --waybar`: one line.
pub fn once() -> ExitCode {
    let mut conn = match Connection::connect() {
        Ok(c) => c,
        Err(e) => {
            emit(&waybar::unavailable());
            return unreachable(&e);
        }
    };
    match Picture::fetch(&mut conn) {
        Err(e) => {
            emit(&waybar::unavailable());
            unreachable(&e)
        }
        Ok(Err(failure)) => {
            emit(&waybar::unavailable());
            eprintln!("nexus: the daemon refused: {}", failure.message);
            ExitCode::from(EXIT_DAEMON_ERROR)
        }
        Ok(Ok(picture)) => {
            emit(&picture.output());
            ExitCode::SUCCESS
        }
    }
}

/// `nexus status --waybar --follow`: a line per change, until Waybar goes.
/// A Daemon that is not there shows as "unavailable" and is retried.
pub fn follow() -> ExitCode {
    let mut last: Option<Output> = None;
    let mut show = |out: Output| -> bool {
        if last.as_ref() == Some(&out) {
            return true;
        }
        let ok = emit(&out);
        last = Some(out);
        ok
    };
    loop {
        if !run_session(&mut show) {
            return ExitCode::SUCCESS;
        }
        if !show(waybar::unavailable()) {
            return ExitCode::SUCCESS;
        }
        std::thread::sleep(RECONNECT);
    }
}

/// Follows one connection until it breaks. False when stdout closed.
fn run_session(show: &mut impl FnMut(Output) -> bool) -> bool {
    let Ok(mut conn) = Connection::connect() else {
        return true;
    };
    // Subscribe first, so a change between the reads and the first wait is
    // parked, not lost.
    if !matches!(conn.call(Op::Subscribe), Ok(Ok(_))) {
        return true;
    }
    let Ok(Ok(mut picture)) = Picture::fetch(&mut conn) else {
        return true;
    };
    if !show(picture.output()) {
        return false;
    }
    loop {
        match conn.next_event(Duration::from_secs(3600)) {
            Err(_) => return true,
            Ok(None) => {}
            Ok(Some(event)) => {
                if picture.apply(event) && !show(picture.output()) {
                    return false;
                }
            }
        }
    }
}
