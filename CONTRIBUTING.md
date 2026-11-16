# Contributing

Thanks for helping. Bug reports, ideas and pull requests are all welcome; for anything bigger than a fix, open an issue first so we can agree on the shape.

## Setup

Needs Rust 1.90 or newer and `dbus-daemon` (the tests run a private one). `socat` and `systemd-socket-activate` are optional: the tests that need them skip with a message.

```sh
git clone https://github.com/CPT-Dawn/Nexus && cd Nexus
cargo build
cargo test --workspace
```

## The checks CI runs

Run these before you push; CI fails on any of them.

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
shellcheck scripts/nexus-switch.sh scripts/test-nexus-switch.sh
scripts/test-nexus-switch.sh
cargo deny check
```

The performance budgets run in their own workflow (`bench.yml`) on every change to `crates/`: the Daemon under 15 MB of memory, the TUI's first frame under 50 ms, and no CPU at idle. To measure them locally:

```sh
cargo test --release -p nexus --test s3_budgets -- --ignored --test-threads=1 --nocapture
```

[docs/testing.md](docs/testing.md) explains every test seam: the fake NetworkManager, the TUI on a pty, the real-hardware rig and the budgets.

## Running Nexus without touching your network

The `demo` example starts a private world: a private D-Bus, a scripted fake NetworkManager and a real Daemon, all in a temp dir. Nothing on your machine changes.

```sh
cargo build -p nexus
cargo run -p nexus-testkit --example demo -- target/debug/nexus wifi
```

Scenarios include `online`, `offline`, `portal`, `wifi`, `wifi-enterprise`, `radios`, `vpn`, `ethernet`, `hotspot`, `tether`, `profile`, `settings`, `effects` and `busy`. The comment at the top of [`crates/nexus-testkit/examples/demo.rs`](crates/nexus-testkit/examples/demo.rs) says what each one shows and which keys to try.

## Words and decisions

The UI, the code and the docs share one vocabulary: [GLOSSARY.md](GLOSSARY.md) lists each term and the words to avoid. The decisions behind the design are in [docs/adr/](docs/adr). The socket API is frozen at version 1 (ADR-0006): a new op, event or error code needs its documented example in [docs/ipc.md](docs/ipc.md) in the same change, and the conformance test runs it.

## Pull requests

- Branch from `main`, one logical change per PR.
- Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/) (`fix(tui): …`, `feat(daemon): …`).
- A user-facing change updates the docs page it touches.

## Re-recording the demo

The README's GIF is recorded with [VHS](https://github.com/charmbracelet/vhs) from [`demo/nexus.tape`](demo/nexus.tape), against the demo world above:

```sh
cargo build --release -p nexus
cargo build --release -p nexus-testkit --example demo
vhs demo/nexus.tape
ffmpeg -i assets/demo.mp4 -vf "fps=25,scale='min(2560,iw)':-1:flags=lanczos,split[a][b];[a]palettegen=max_colors=256:stats_mode=diff[p];[b][p]paletteuse=dither=none:diff_mode=rectangle" -loop 0 assets/demo.gif
```

## Releases

A maintainer tags `v2.x.y` with a signed tag. [`.github/workflows/release.yml`](.github/workflows/release.yml) builds the x86_64 and aarch64 tarballs, publishes the GitHub Release and updates the `nexus-nm` and `nexus-nm-bin` AUR packages. Add the release's entry to [CHANGELOG.md](CHANGELOG.md) before tagging.
