//! Packaging helpers: shell completions and the man page, produced by the
//! real binary so the AUR packages never ship hand-written copies.

use std::process::Command;

const NEXUS: &str = env!("CARGO_BIN_EXE_nexus");

fn run(args: &[&str]) -> String {
    let out = Command::new(NEXUS)
        .args(args)
        .output()
        .expect("spawn nexus");
    assert!(out.status.success(), "{out:?}");
    String::from_utf8(out.stdout).expect("utf-8 output")
}

#[test]
fn completions_cover_each_shell() {
    for (shell, marker) in [
        ("bash", "_nexus"),
        ("zsh", "#compdef nexus"),
        ("fish", "complete -c nexus"),
    ] {
        let script = run(&["completions", shell]);
        assert!(script.contains(marker), "{shell}: {script}");
        assert!(script.contains("status"), "{shell} lacks subcommands");
    }
}

#[test]
fn man_page_is_roff_for_nexus() {
    let page = run(&["man"]);
    assert!(page.starts_with(".ie") || page.starts_with(".TH"), "{page}");
    assert!(page.contains("nexus"));
    assert!(page.contains("status"));
}

#[test]
fn the_man_page_ends_with_the_exit_status_table() {
    let page = run(&["man"]);
    let (_, table) = page
        .split_once(".SH EXIT STATUS")
        .expect("EXIT STATUS section");
    for needle in ["no_portal", "unknown_device", "doctor", "cannot be reached"] {
        assert!(table.contains(needle), "{needle} missing from {table}");
    }
}

#[test]
fn man_dir_writes_a_page_for_every_command() {
    let dir = tempfile::tempdir().unwrap();
    run(&["man", "--dir", dir.path().to_str().unwrap()]);
    for page in [
        "nexus.1",
        "nexus-status.1",
        "nexus-wifi.1",
        "nexus-wifi-join.1",
        "nexus-portal-open.1",
    ] {
        let text = std::fs::read_to_string(dir.path().join(page))
            .unwrap_or_else(|e| panic!("{page}: {e}"));
        assert!(text.contains(".TH"), "{page}: {text}");
    }
    let join = std::fs::read_to_string(dir.path().join("nexus-wifi-join.1")).unwrap();
    assert!(join.contains("password\\-stdin"), "{join}");
}

#[test]
fn completions_know_the_newest_commands() {
    for shell in ["bash", "zsh", "fish"] {
        let script = run(&["completions", shell]);
        for word in ["portal", "autoconnect", "activity"] {
            assert!(script.contains(word), "{shell} lacks {word}");
        }
    }
}
