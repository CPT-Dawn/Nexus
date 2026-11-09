use std::process::ExitCode;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        Some("--version" | "-V") => {
            println!("nexus {VERSION}");
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("nexus {VERSION}: v2 is in development");
            ExitCode::FAILURE
        }
    }
}
