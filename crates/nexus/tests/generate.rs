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
