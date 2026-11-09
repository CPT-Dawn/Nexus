//! Front end commands. They talk to the Daemon only through `nexus-ipc`.

use std::io::ErrorKind;
use std::process::ExitCode;

use nexus_ipc::Connection;

/// Exit code when the Daemon cannot be reached.
pub const EXIT_UNREACHABLE: u8 = 3;
/// Exit code when the Daemon answered with an error.
pub const EXIT_DAEMON_ERROR: u8 = 4;

pub fn status(json: bool) -> ExitCode {
    let mut conn = match Connection::connect() {
        Ok(c) => c,
        Err(e) => return unreachable(&e),
    };
    match conn.status() {
        Err(e) => unreachable(&e),
        Ok(Err(failure)) => {
            eprintln!("nexus: the daemon refused: {}", failure.message);
            ExitCode::from(EXIT_DAEMON_ERROR)
        }
        Ok(Ok(status)) => {
            if json {
                println!(
                    "{}",
                    serde_json::to_string(&status).expect("status serializes")
                );
            } else {
                println!("{}", status.human());
            }
            ExitCode::SUCCESS
        }
    }
}

fn unreachable(e: &std::io::Error) -> ExitCode {
    let path = nexus_ipc::socket_path().map_or_else(
        || "(XDG_RUNTIME_DIR is not set)".into(),
        |p| p.display().to_string(),
    );
    let hint = match e.kind() {
        ErrorKind::NotFound | ErrorKind::ConnectionRefused => {
            "Start it with `systemctl --user start nexus.socket`, or run `nexus daemon`."
        }
        _ => "Is it running as your user?",
    };
    eprintln!("nexus: cannot reach the daemon at {path}: {e}\n{hint}");
    ExitCode::from(EXIT_UNREACHABLE)
}
