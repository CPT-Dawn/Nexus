# Testing

Seams are defined in the spec (#2, "Testing Decisions"). This page is the how-to.

## S1: a real Daemon against a fake NetworkManager

`crates/nexus-testkit` is the shared harness. Integration tests live in `crates/nexus/tests/` and pass `env!("CARGO_BIN_EXE_nexus")`.

```rust
let world = World::start(env!("CARGO_BIN_EXE_nexus"));       // bus + fake NM + daemon
world.nm.set_primary(Some(ActiveConnection::wifi("Home")));
world.nm.set_state(NmState::ConnectedGlobal);
let resp = world.wire().call(json!({"op": "status"}));        // raw JSON-lines
let out = world.run_nexus(&["status", "--json"]);             // the real CLI
```

- `World::builder(bin).nm_running(false).daemon(false).start()` controls what starts. `world.nm.stop()` / `restart()` simulate NetworkManager going away and coming back.
- Everything runs with a cleared environment: a private `dbus-daemon` stands in for the system bus (`DBUS_SYSTEM_BUS_ADDRESS`), the session bus points at nothing, `XDG_RUNTIME_DIR` is a tempdir. The production code path is unchanged; no test-only env var exists.
- Assert on the wire (`Wire::call` returns the whole response `Value`) and on CLI stdout/exit codes, never on Daemon internals.
- To fake more of NetworkManager: add the member to the interface in `nexus-testkit/src/fake_nm.rs`, keep its value in `Model`, add a `set_*` that mutates the model and emits `PropertiesChanged`. Fake only what the Backend reads, so a proxy member the fake lacks fails loudly.
- Events: `let mut ev = world.wire(); ev.subscribe();` then act, then `ev.next_event("config_reloaded")` (skips other lines, panics on timeout). Subscribe before acting, or the event is missed. Config: `world.write_config(text)` / `world.config_path()` (`$HOME/.config/nexus/config.toml` in the world's `HOME`).
- Never `sleep`: poll (the harness polls the socket) or wait for an event.

Needs `dbus-daemon` installed. Optional tools, tests skip with a message without them: `systemd-socket-activate` (socket activation), and for the peer-credential test `socat`, passwordless `sudo -u <user>` and `NEXUS_TEST_INTRUDER=<user>` (CI creates user `intruder`).

Run: `cargo test --workspace`.
