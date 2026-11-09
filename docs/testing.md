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
- Wi-Fi fixtures: `nm.add_wifi_device("wlan0")`, `nm.add_ap(dev, Ap::wpa2("Home", "AA:..", 70))`, `nm.set_strength`, `nm.remove_ap`, `nm.add_wifi_profile`, `nm.set_last_scan_ago(dev, ms)`, `nm.refuse_scans(true)`, `nm.scan_requests()`. The fake's `RequestScan` enforces NetworkManager's 10 s limit. `Wire::try_read_line(timeout)` is for asserting an event did or did not arrive; subscribe first, wait for the event your own setup causes, then assert on the next.
- Joining: `nm.set_psk(ssid, pw)` makes other passwords fail like a wrong one; `nm.script_join_failure(ssid, died_in_state, reason)` kills a join in an `NMDeviceState` with an `NMDeviceStateReason`. Assert with `nm.persisted_profiles()`, `nm.profile_count()` (memory + disk), `nm.joined_ssid(dev)`. A join is a real state walk (40, 50, [60], 70, 100 or 120) on the fake's own task, so `wifi_join` over the wire blocks until it ends. The CLI password tests spawn `world.nexus()` with a piped stdin (`--password-stdin`).
- Never `sleep`: poll (the harness polls the socket) or wait for an event.

Needs `dbus-daemon` installed. Optional tools, tests skip with a message without them: `systemd-socket-activate` (socket activation), and for the peer-credential test `socat`, passwordless `sudo -u <user>` and `NEXUS_TEST_INTRUDER=<user>` (CI creates user `intruder`).

## S2: the TUI on top of S1

`crates/nexus/tests/s2_tui.rs`. The TUI is a library (`nexus_tui::tui`, lib target of the `nexus` crate) so tests drive it without a terminal: a `Rig` starts a `World`, spawns the real `Client` (the Daemon link thread) and feeds its `Msg`s into an `App`.

```rust
let mut rig = Rig::online();                       // world + client + App, status received
rig.key(KeyCode::Char('?'));                        // keystrokes in
insta::assert_snapshot!("help_overlay", rig.text(STANDARD));   // frame out
rig.pump_until("the offline status", |a| ...);     // after changing the fake NM
```

- Widths: `NARROW` 60x20, `STANDARD` 100x28, `WIDE` 140x32 (plus a 30x6 "too small"). Colour tiers: `styled_of(&buffer)` snapshots the text plus a table of every distinct style, so truecolor/256/16/none/ASCII all have a frame.
- `tui::render(&app, w, h)` returns the `Buffer`; `App::update(Msg)` returns whether a redraw is needed, which is how the zero-idle-frames test counts: it runs `tui::event_loop` on a `TestBackend` thread and asserts no frame is drawn over a quiet period.
- New snapshot: write the test, run `INSTA_UPDATE=always cargo test -p nexus --test s2_tui`, read the `.snap` file in `crates/nexus/tests/snapshots/`, commit it. CI never updates snapshots.
- Eyeball it for real: `cargo build -p nexus && cargo run -p nexus-testkit --example demo -- target/debug/nexus [online|offline|portal] [--ascii|--nerd-font]` runs the TUI on your terminal against a private fake NetworkManager and Daemon.

Run: `cargo test --workspace`.

## Real mode: hwsim + hostapd + real NetworkManager

`crates/nexus/tests/real_mode.rs` runs the S1 wire and CLI against real NetworkManager (status, wifi list/scan, join open/WPA2/SAE, wrong password, Forget). It skips in a plain `cargo test`. The rig runs it:

```
rig/run.sh [test-name-filter]      # as your user; builds, then uses sudo
```

CI runs it as `.github/workflows/rig.yml` (ubuntu-24.04 with `linux-modules-extra-$(uname -r)` for `mac80211_hwsim`).

Needs on the machine: `network-manager wpa_supplicant hostapd dnsmasq iw dbus` (dnsmasq-base is enough), the `mac80211_hwsim` module, root via sudo. The rig does not install or configure anything on the host.

How it stays off your network:
- It loads `mac80211_hwsim` itself (refuses if already loaded), then moves each new radio into a throwaway namespace: the station radio into a mount+net+pid namespace with a tmpfs `/run`, `/var/lib` and `/etc/NetworkManager` plus a private `dbus-daemon` (`/run/nexus-rig/bus`), the AP radios into a bare network namespace with hostapd and dnsmasq (DHCP, `10.99.N.0/24`).
- `inner.sh` refuses to continue if any non-wireless interface is visible; `RealWorld::start` refuses a bus address outside `/run/nexus-rig/` and any wired interface. The test Daemon gets a cleared environment like in the fake world.
- If NetworkManager runs on the host, the rig stops unless `NEXUS_RIG_ALLOW_HOST_NM=1`: for a few milliseconds the new radios exist in the host namespace before they are moved and the host's NM may glimpse them (it has no Profile for them).
- Teardown kills the pid namespace's init (everything inside dies), removes the radios and temp files.
- On failure the tail of every rig log (`dbus`, `hostapd-*`, `dnsmasq`, `wpa_supplicant`, `networkmanager`) is printed.

Add an AP: drop a hostapd config in `rig/aps.d/<name>.conf` with `interface=@IFACE@` (the rig substitutes it). One radio and one `10.99.N.0/24` DHCP subnet per file, in alphabetical order. Then add its SSID and password as constants in `real_mode.rs`. Sketches:
- OWE: `wpa=2`, `wpa_key_mgmt=OWE`, `rsn_pairwise=CCMP`, `ieee80211w=2`.
- Transition: WPA-PSK and SAE in one `wpa_key_mgmt=WPA-PSK SAE`, `ieee80211w=1`.
- Hidden: `ignore_broadcast_ssid=1`.
- Enterprise: `wpa_key_mgmt=WPA-EAP`, `ieee8021x=1`, `eap_server=1`, `eap_user_file=` (a file next to the config), plus server certificates generated in `rig/`.
- Simulated Portal: a small HTTP responder in the AP namespace (`in_ap` in `inner.sh`) that answers NetworkManager's connectivity check URL (set `[connectivity]` in the generated `NetworkManager.conf`) with a redirect.
