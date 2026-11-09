//! Seam: the `nexus` binary's stdout and exit code.

use std::process::Command;

#[test]
fn version_flag_prints_name_and_version() {
    let out = Command::new(env!("CARGO_BIN_EXE_nexus"))
        .arg("--version")
        .output()
        .expect("run nexus");
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout), "nexus 2.0.0-dev\n");
}
