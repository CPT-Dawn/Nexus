//! Seam S3: the Performance budgets, measured on real processes (a release
//! `nexus daemon` and the real `nexus` TUI on a pty) against the fake
//! NetworkManager. See `docs/testing.md`.
//!
//! The numbers only mean something for a release build, so the budget tests
//! are `#[ignore]`d in the normal run; the bench workflow runs
//! `cargo test --release -p nexus --test s3_budgets -- --ignored --test-threads=1`.
//! Every test prints what it measured, and a failure names the budget and
//! the measured value.

use std::fs;
use std::io::{Read as _, Write as _};
use std::os::fd::OwnedFd;
use std::process::{Child, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use nexus_testkit::{ActiveConnection, Ap, DeviceId, NmConnectivity, NmState, World};
use serde_json::json;

const NEXUS: &str = env!("CARGO_BIN_EXE_nexus");

/// Performance budgets (spec #2). Resident memory is the peak (`VmHWM`).
/// The spec says 15 MB, not 15 MiB.
const DAEMON_RSS_BUDGET_BYTES: u64 = 15_000_000;
const TUI_FIRST_FRAME_BUDGET: Duration = Duration::from_millis(50);
/// "0%": anything the scheduler charges above this over the window fails.
/// The Daemon does nothing when idle (measured 0.000%). The TUI redraws the
/// traffic graph once a second while the Overview shows it (ADR-0003), about
/// 0.07%, so its ceiling is a little looser; a busy loop is orders above both.
const DAEMON_IDLE_CPU_BUDGET_PERCENT: f64 = 0.1;
/// Overview only: the traffic graph redraws at 1 Hz, which ADR-0003 allows as
/// a deliberate processing cost for aesthetics. Any tab without that graph is
/// held to `TUI_GRAPH_FREE_IDLE_CPU_BUDGET_PERCENT` instead.
const TUI_IDLE_CPU_BUDGET_PERCENT: f64 = 0.2;
/// A tab with no animated or polled surface must cost nothing. Scheduler
/// accounting is in nanoseconds, so this is only a float-rounding allowance:
/// 0.001% of the 5 s window is 50 us, which a single stray wakeup exceeds.
/// Measured: 0.000%. (It once read 0.002%: the Daemon link thread woke every
/// second even with the graphs hidden; it now sleeps in `poll`.)
const TUI_GRAPH_FREE_IDLE_CPU_BUDGET_PERCENT: f64 = 0.001;
const IDLE_WINDOW: Duration = Duration::from_secs(5);

// ---- /proc measurements ----------------------------------------------------

/// Value of a `Key:   123 kB` line of /proc/<pid>/status, in kB.
fn status_kib(pid: u32, key: &str) -> u64 {
    let text = fs::read_to_string(format!("/proc/{pid}/status")).expect("read /proc status");
    parse_status_kib(&text, key).unwrap_or_else(|| panic!("no {key} in /proc/{pid}/status"))
}

fn parse_status_kib(text: &str, key: &str) -> Option<u64> {
    let rest = text.lines().find_map(|l| l.strip_prefix(key))?;
    rest.strip_prefix(':')?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

/// CPU time used by every thread of `pid` so far, in nanoseconds, from the
/// scheduler's own books (so it is finer than the 10 ms `utime` tick).
fn cpu_ns(pid: u32) -> u64 {
    fs::read_dir(format!("/proc/{pid}/task"))
        .expect("list threads")
        .flatten()
        .filter_map(|t| fs::read_to_string(t.path().join("schedstat")).ok())
        .filter_map(|s| s.split_whitespace().next()?.parse::<u64>().ok())
        .sum()
}

/// CPU used by `pid` over `window`, as a percentage of one core.
fn idle_cpu_percent(pid: u32, window: Duration) -> f64 {
    let before = cpu_ns(pid);
    thread::sleep(window);
    let used = cpu_ns(pid) - before;
    used as f64 / window.as_nanos() as f64 * 100.0
}

/// Prints a measurement and appends it to the CI job summary when there is one.
fn report(line: &str) {
    println!("{line}");
    if let Some(path) = std::env::var_os("GITHUB_STEP_SUMMARY")
        && let Ok(mut f) = fs::OpenOptions::new().append(true).open(path)
    {
        let _ = writeln!(f, "- {line}");
    }
}

#[test]
fn proc_status_lines_parse() {
    let text = "Name:\tnexus\nVmHWM:\t    9876 kB\nVmRSS:\t    5432 kB\n";
    assert_eq!(parse_status_kib(text, "VmHWM"), Some(9876));
    assert_eq!(parse_status_kib(text, "VmRSS"), Some(5432));
    assert_eq!(parse_status_kib(text, "VmSwap"), None);
}

// ---- a representative world --------------------------------------------------

/// A laptop in a busy cafe: one Wi-Fi device with 40 Networks in range, online.
fn busy_world() -> (World, DeviceId) {
    let world = World::start(NEXUS);
    let wlan0 = world.nm.add_wifi_device("wlan0");
    for i in 0..40u8 {
        let bssid = format!("AA:BB:CC:00:00:{i:02X}");
        world
            .nm
            .add_ap(wlan0, Ap::wpa2(&format!("Network-{i:02}"), &bssid, 30 + i));
    }
    world
        .nm
        .set_primary(Some(ActiveConnection::wifi("Network-00")));
    world.nm.set_connectivity(NmConnectivity::Full);
    world.nm.set_state(NmState::ConnectedGlobal);
    world.nm.set_traffic(wlan0, 1_000_000, 100_000);
    (world, wlan0)
}

/// Exercises every Daemon op a Front end uses, a few hundred times.
fn work_the_daemon(world: &World, wlan0: DeviceId) {
    let mut wire = world.wire();
    for round in 0..200u64 {
        for op in ["status", "wifi_list", "traffic", "activity"] {
            let resp = wire.call(json!({ "op": op }));
            assert!(resp.get("ok").is_some(), "{op} failed: {resp}");
        }
        world
            .nm
            .set_traffic(wlan0, 1_000_000 + round * 50_000, 100_000 + round * 5_000);
    }
}

// ---- the budgets ---------------------------------------------------------------

#[test]
#[ignore = "release-mode budget; run by the bench workflow"]
fn the_daemon_stays_under_its_memory_budget() {
    let (world, wlan0) = busy_world();
    work_the_daemon(&world, wlan0);
    let pid = world.daemon_pid().expect("a daemon runs");
    let peak = status_kib(pid, "VmHWM");
    let now = status_kib(pid, "VmRSS");
    report(&format!(
        "Daemon resident memory: peak {:.1} MiB, now {:.1} MiB (budget < {} MB)",
        peak as f64 / 1024.0,
        now as f64 / 1024.0,
        DAEMON_RSS_BUDGET_BYTES / 1_000_000
    ));
    assert!(
        peak * 1024 < DAEMON_RSS_BUDGET_BYTES,
        "Daemon peak RSS {peak} KiB exceeds the {DAEMON_RSS_BUDGET_BYTES} byte budget"
    );
}

#[test]
#[ignore = "release-mode budget; run by the bench workflow"]
fn an_idle_daemon_uses_no_cpu() {
    let (world, wlan0) = busy_world();
    work_the_daemon(&world, wlan0);
    let pid = world.daemon_pid().expect("a daemon runs");
    // A Front end subscribed is the Daemon's normal idle state; let the last
    // burst drain before the window opens.
    let mut events = world.wire();
    events.subscribe();
    thread::sleep(Duration::from_secs(1));
    let used = idle_cpu_percent(pid, IDLE_WINDOW);
    report(&format!(
        "Daemon idle CPU over {} s: {used:.3}% (budget < {DAEMON_IDLE_CPU_BUDGET_PERCENT}%)",
        IDLE_WINDOW.as_secs()
    ));
    assert!(
        used < DAEMON_IDLE_CPU_BUDGET_PERCENT,
        "idle Daemon used {used:.3}% CPU, budget is {DAEMON_IDLE_CPU_BUDGET_PERCENT}%"
    );
}

// ---- the TUI on a pty ----------------------------------------------------------

fn pty() -> (std::fs::File, OwnedFd) {
    use rustix::fs::{Mode, OFlags};
    use rustix::pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt};
    use rustix::termios::{Winsize, tcsetwinsize};
    let master = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY).expect("openpt");
    grantpt(&master).expect("grantpt");
    unlockpt(&master).expect("unlockpt");
    let name = ptsname(&master, Vec::new()).expect("ptsname");
    let slave = rustix::fs::open(
        name.as_c_str(),
        OFlags::RDWR | OFlags::NOCTTY,
        Mode::empty(),
    )
    .expect("open the slave side");
    tcsetwinsize(
        &slave,
        Winsize {
            ws_row: 32,
            ws_col: 120,
            ws_xpixel: 0,
            ws_ypixel: 0,
        },
    )
    .expect("set the window size");
    (std::fs::File::from(master), slave)
}

type Seen = Arc<Mutex<(String, Vec<(String, Instant)>)>>;

/// The real TUI on a pty. A reader thread drains the terminal and stamps
/// when each marker first appeared in what the TUI wrote.
struct Tui {
    child: Child,
    started: Instant,
    seen: Seen,
    _master: std::fs::File,
}

impl Tui {
    fn start(world: &World, markers: &[&str]) -> Tui {
        let (master, slave) = pty();
        let mut cmd = world.nexus();
        cmd.env("TERM", "xterm-256color")
            .env("COLORTERM", "truecolor")
            .env("LANG", "C.UTF-8")
            .stdin(Stdio::from(slave.try_clone().expect("dup slave")))
            .stdout(Stdio::from(slave.try_clone().expect("dup slave")))
            .stderr(Stdio::from(slave));
        let started = Instant::now();
        let child = cmd.spawn().expect("spawn nexus");
        drop(cmd); // closes our copies of the slave end
        let seen: Seen = Arc::new(Mutex::new((String::new(), Vec::new())));
        let shared = Arc::clone(&seen);
        let wanted: Vec<String> = markers.iter().map(|m| m.to_string()).collect();
        let mut reader = master.try_clone().expect("dup master");
        thread::spawn(move || {
            let mut chunk = [0u8; 4096];
            // EIO (the child hung up) or EOF ends the thread.
            while let Ok(n @ 1..) = reader.read(&mut chunk) {
                let at = Instant::now();
                let mut guard = shared.lock().unwrap();
                let (text, stamps) = &mut *guard;
                text.push_str(&String::from_utf8_lossy(&chunk[..n]));
                for m in &wanted {
                    if text.contains(m.as_str()) && !stamps.iter().any(|(s, _)| s == m) {
                        stamps.push((m.clone(), at));
                    }
                }
            }
        });
        Tui {
            child,
            started,
            seen,
            _master: master,
        }
    }

    /// Types `keys` into the TUI.
    fn press(&self, keys: &str) {
        use std::io::Write;
        (&self._master).write_all(keys.as_bytes()).expect("type");
    }

    /// When `marker` first appeared, measured from the spawn.
    fn wait_for(&self, marker: &str) -> Duration {
        let give_up = Instant::now() + Duration::from_secs(20);
        loop {
            {
                let guard = self.seen.lock().unwrap();
                if let Some((_, at)) = guard.1.iter().find(|(s, _)| s == marker) {
                    return *at - self.started;
                }
            }
            assert!(
                Instant::now() < give_up,
                "the TUI never showed `{marker}`; it wrote: {:?}",
                self.seen.lock().unwrap().0
            );
            thread::sleep(Duration::from_millis(2));
        }
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
#[ignore = "release-mode budget; run by the bench workflow"]
fn the_tui_draws_its_first_frame_in_time() {
    // Best of several starts: the budget is about the program, not about a
    // CI neighbour stealing one scheduler slice.
    let mut times = Vec::new();
    for _ in 0..5 {
        let (world, _) = busy_world();
        let tui = Tui::start(&world, &[" Nexus "]);
        times.push(tui.wait_for(" Nexus "));
    }
    times.sort();
    let (best, median) = (times[0], times[times.len() / 2]);
    report(&format!(
        "TUI first frame: best {} ms, median {} ms of {} starts (budget < {} ms)",
        best.as_millis(),
        median.as_millis(),
        times.len(),
        TUI_FIRST_FRAME_BUDGET.as_millis()
    ));
    assert!(
        best < TUI_FIRST_FRAME_BUDGET,
        "TUI first frame took {best:?} (best of {}), budget is {TUI_FIRST_FRAME_BUDGET:?}",
        times.len()
    );
}

#[test]
#[ignore = "release-mode budget; run by the bench workflow"]
fn an_idle_tui_uses_no_cpu() {
    let (world, _) = busy_world();
    let tui = Tui::start(&world, &[" Nexus ", "Network-00"]);
    // Idle starts once the Daemon's status is on screen.
    tui.wait_for("Network-00");
    thread::sleep(Duration::from_secs(1));
    let used = idle_cpu_percent(tui.child.id(), IDLE_WINDOW);
    report(&format!(
        "TUI idle CPU over {} s: {used:.3}% (budget < {TUI_IDLE_CPU_BUDGET_PERCENT}%)",
        IDLE_WINDOW.as_secs()
    ));
    assert!(
        used < TUI_IDLE_CPU_BUDGET_PERCENT,
        "idle TUI used {used:.3}% CPU, budget is {TUI_IDLE_CPU_BUDGET_PERCENT}%"
    );
}

#[test]
#[ignore = "release-mode budget; run by the bench workflow"]
fn an_idle_tui_on_a_graph_free_tab_uses_strictly_no_cpu() {
    let (world, _) = busy_world();
    let tui = Tui::start(&world, &[" Nexus ", "Network-00"]);
    tui.wait_for("Network-00");
    // Settings has no traffic graph, scan sweep or countdown.
    tui.press("6");
    // Let the tab glide (#29 effects) finish and the Daemon go quiet.
    thread::sleep(Duration::from_secs(2));
    let used = idle_cpu_percent(tui.child.id(), IDLE_WINDOW);
    report(&format!(
        "TUI idle CPU on Settings over {} s: {used:.3}% (budget < {TUI_GRAPH_FREE_IDLE_CPU_BUDGET_PERCENT}%)",
        IDLE_WINDOW.as_secs()
    ));
    assert!(
        used < TUI_GRAPH_FREE_IDLE_CPU_BUDGET_PERCENT,
        "idle TUI on a graph-free tab used {used:.3}% CPU, budget is {TUI_GRAPH_FREE_IDLE_CPU_BUDGET_PERCENT}%"
    );
}
