//! Real mode: a real Daemon against the real NetworkManager the rig starts.
//!
//! The rig (`rig/run.sh`) builds a throwaway world of virtual radios and
//! hostapd access points and exports `NEXUS_RIG=1` plus `NEXUS_RIG_BUS`. This
//! type refuses to run without them, and refuses a bus that is not the rig's.

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use tempfile::TempDir;

use crate::world::{PATIENCE, Wire};

/// The only bus address real mode accepts: the one the rig creates.
const RIG_BUS_PREFIX: &str = "unix:path=/run/nexus-rig/";

/// A Daemon on the rig's private system bus.
#[derive(Debug)]
pub struct RealWorld {
    bin: PathBuf,
    bus: String,
    daemon: Child,
    tmp: TempDir,
}

impl RealWorld {
    /// `None` (with a note on stderr) when the rig is not running, so the
    /// real-mode tests skip in a plain `cargo test`.
    ///
    /// # Panics
    /// When the rig variables are set but unsafe: a bus that is not the rig's,
    /// or a wired network interface visible (a sign this is the host).
    pub fn start(bin: impl Into<PathBuf>) -> Option<RealWorld> {
        if std::env::var("NEXUS_RIG").as_deref() != Ok("1") {
            eprintln!("skipped: real mode needs the rig (rig/run.sh)");
            return None;
        }
        let bus = std::env::var("NEXUS_RIG_BUS").expect("NEXUS_RIG_BUS is set by the rig");
        assert!(
            bus.starts_with(RIG_BUS_PREFIX),
            "refusing to run: {bus} is not the rig's private bus"
        );
        for entry in std::fs::read_dir("/sys/class/net").expect("list interfaces") {
            let entry = entry.expect("interface").path();
            let wireless = entry.join("phy80211").exists();
            let lo = entry.file_name().is_some_and(|n| n == "lo");
            assert!(
                wireless || lo,
                "refusing to run: {} is not a virtual radio, this looks like the host",
                entry.display()
            );
        }
        let tmp = tempfile::tempdir().expect("tempdir");
        for dir in ["run", "home"] {
            std::fs::create_dir(tmp.path().join(dir)).expect("mkdir");
        }
        let bin = bin.into();
        let mut world = RealWorld {
            daemon: Command::new("true").spawn().expect("placeholder child"),
            bin,
            bus,
            tmp,
        };
        let _ = world.daemon.wait();
        let log = std::fs::File::create(world.tmp.path().join("daemon.log")).expect("log");
        let mut cmd = world.nexus();
        cmd.arg("daemon")
            .stdin(Stdio::null())
            .stdout(log.try_clone().expect("clone log"))
            .stderr(log);
        world.daemon = cmd.spawn().expect("spawn nexus daemon");
        let socket = world.socket();
        let start = std::time::Instant::now();
        while std::os::unix::net::UnixStream::connect(&socket).is_err() {
            assert!(
                start.elapsed() < PATIENCE,
                "daemon socket never appeared:\n{}",
                world.daemon_log()
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        Some(world)
    }

    pub fn socket(&self) -> PathBuf {
        self.tmp.path().join("run/nexus/nexus.sock")
    }

    pub fn daemon_log(&self) -> String {
        std::fs::read_to_string(self.tmp.path().join("daemon.log")).unwrap_or_default()
    }

    /// A `nexus` command with only the rig's bus reachable.
    pub fn nexus(&self) -> Command {
        let mut cmd = Command::new(&self.bin);
        cmd.env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", self.tmp.path().join("home"))
            .env("XDG_RUNTIME_DIR", self.tmp.path().join("run"))
            .env("DBUS_SYSTEM_BUS_ADDRESS", &self.bus)
            .env(
                "DBUS_SESSION_BUS_ADDRESS",
                format!(
                    "unix:path={}",
                    self.tmp.path().join("no-session-bus").display()
                ),
            );
        if let Some(log) = std::env::var_os("RUST_LOG") {
            cmd.env("RUST_LOG", log);
        }
        cmd
    }

    pub fn wire(&self) -> Wire {
        Wire::connect_patiently(&self.socket(), Duration::from_secs(60))
    }

    pub fn run_nexus(&self, args: &[&str]) -> std::process::Output {
        self.nexus().args(args).output().expect("run nexus")
    }
}

impl Drop for RealWorld {
    fn drop(&mut self) {
        let _ = self.daemon.kill();
        let _ = self.daemon.wait();
    }
}
