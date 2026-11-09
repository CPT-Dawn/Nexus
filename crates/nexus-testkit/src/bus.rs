//! A private `dbus-daemon`, so tests never touch the real system or session bus.

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command, Stdio};

#[derive(Debug)]
pub struct Bus {
    pub address: String,
    child: Child,
}

impl Bus {
    /// Starts a bus listening on `<dir>/<name>`. Nothing is D-Bus-activatable
    /// (no servicedir) and no polkit exists, which is what a test wants.
    ///
    /// The same config serves the fake system bus and the fake session bus:
    /// `type system` would want a `<user>` and policy directories.
    pub fn start(dir: &Path, name: &str) -> Bus {
        let socket = dir.join(name);
        let config = dir.join(format!("{name}.conf"));
        std::fs::write(
            &config,
            format!(
                r#"<busconfig>
  <type>session</type>
  <listen>unix:path={}</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
"#,
                socket.display()
            ),
        )
        .expect("write dbus-daemon config");
        let mut child = Command::new("dbus-daemon")
            .arg("--nofork")
            .arg("--nopidfile")
            .arg("--print-address")
            .arg(format!("--config-file={}", config.display()))
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .stdout(Stdio::piped())
            .spawn()
            .expect("dbus-daemon should be installed (apt install dbus / pacman -S dbus)");
        let mut address = String::new();
        BufReader::new(child.stdout.take().expect("piped stdout"))
            .read_line(&mut address)
            .expect("read the bus address");
        let address = address.trim().to_owned();
        assert!(!address.is_empty(), "dbus-daemon printed no address");
        Bus { address, child }
    }
}

impl Drop for Bus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
