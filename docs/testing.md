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
- Sharing: `nm.set_profile_secret(uuid, "wpa-psk"|"sae", pw)` stores a Profile's secret (a Daemon join stores its own); `nm.deny_secret_reads(true)` makes `GetSecrets` on Profiles fail like a polkit denial; `nm.secret_reads()` counts reads that got through, so tests prove nothing is read before a reveal.
- Joining: `nm.set_psk(ssid, pw)` makes other passwords fail like a wrong one; `nm.script_join_failure(ssid, died_in_state, reason)` kills a join in an `NMDeviceState` with an `NMDeviceStateReason`. Assert with `nm.persisted_profiles()`, `nm.profile_count()` (memory + disk), `nm.joined_ssid(dev)`. A join is a real state walk (40, 50, [60], 70, 100 or 120) on the fake's own task, so `wifi_join` over the wire blocks until it ends. The CLI password tests spawn `world.nexus()` with a piped stdin (`--password-stdin`).
- Secret prompts (`crates/nexus/tests/s1_secrets.rs`): the fake plays NetworkManager's AgentManager. A join of a secured Network without a password makes it call `GetSecrets` on the Daemon's agent (repeats carry `REQUEST_NEW`), so tests act as the Front end: `let mut fe = world.wire(); fe.subscribe();`, start the join on another wire in a thread, `fe.next_event("secret_prompt_opened")`, then `fe.call(json!({"op": "secret_answer", "prompt": id, "secret": ".."}))`. `Wire::call` keeps events that arrive meanwhile for `next_event`. Knobs: `nm.set_request_new(false)` (fail with reason 8 instead of asking again), `nm.cancel_pending_secrets()`, `nm.secrets_pending()`, `nm.agent_registered()`, `nm.agent_calls()`. A short prompt timeout needs the config in place before the Daemon starts: `World::builder(bin).daemon(false).start()`, `write_config("[prompts]\ntimeout_secs = 1\n")`, `start_daemon()`. The CLI's masked prompt is tested on a pty (`pty()` helper in that file).
- Resilience and radios (`crates/nexus/tests/s1_resilience.rs`): `nm.stop()`/`restart()` also end joins in flight and drop memory-only Profiles, like a real restart. `nm.sleep()` / `nm.resume()` play logind's `PrepareForSleep` (a fake `org.freedesktop.login1` lives beside the fake NM and survives its restarts) and move NM to `Asleep` and back. `nm.wifi_enabled()`, `nm.wwan_enabled()`, `nm.set_wifi_hardware_blocked(bool)` model the soft switches and the hardware one (`WirelessEnabled` reads false while blocked, `WirelessHardwareEnabled` false). Tests subscribe BEFORE building the world when they need to see its events (`watched_world()`), and read status and networks events in either order (`wait_for_all`): the Daemon pushes both after a burst and their order is not promised. The Daemon coalesces a burst for 100 ms, so do not tear the world down right after building it and expect the Daemon to have seen the middle.
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
- The Wi-Fi tab (`tests/s2_wifi.rs`) acts on the Daemon, so its `Rig` calls `app.attach(world.socket(), tx)`; commands run on threads and come back as `Msg::Done`, so use `rig.pump_until(..)` after a key that sends one. Worlds come from `nexus_testkit::scenarios` (`wifi_busy`, `wifi_with_open_prompt`), the same ones `demo wifi` / `demo wifi-prompt` show. Feed `Msg::Tick` by hand to move the scan sweep.
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
## Overview data and the graphs (ticket #12)

- S1 (`crates/nexus/tests/s1_overview.rs`): `{"op":"traffic"}` and `{"op":"activity"}` over the wire, plus `activity` events. Script counters with `nm.set_traffic(dev, rx_bytes, tx_bytes)` (the first device added carries the primary connection); `nm.statistics_reads()` proves whether anyone asked NetworkManager for them.
- S2: `rig.traffic(rx_bps, tx_bps)` injects a scripted sample so graph snapshots do not depend on a clock; `rig.sync_watch()` does what `nexus` does after each frame (poll traffic only while `App::traffic_visible()`). Tests that need the Daemon's own rates pump until `app.traffic_samples()` / `traffic_now()` show them.
- Demo: `... --example demo -- target/debug/nexus busy` shows the moving graphs and activity strip.

## The Doctor (ticket #26)

- S1 (`crates/nexus/tests/s1_doctor.rs`): `{"op":"doctor"}` and `{"op":"doctor_fix"}` over the wire, the `nexus doctor [--json]` CLI, and `doctor_findings` events.
- The Daemon reads services from systemd over the system bus, so the host is a **fake systemd on the private bus**: `world.systemd.set_unit("systemd-networkd.service", Unit::Active)` (`Absent` is the default, `Inactive` means installed but stopped). Radio switches, the gateway and IPv6 come from the fake NM: `nm.set_radio(soft, hard)`, `nm.set_gateway(Some("127.0.0.1"))`, `nm.set_ipv6(true)`, `nm.set_connectivity_check(false)`.
- DNS resolves the host of `connectivity.url`: use `http://localhost/..` for "works" and `http://x.invalid/` for "never resolves" (`world.write_config`). A world with NM down must set `[connectivity] enabled = false`, or the Doctor looks up the default URL on the real network.
- Fixes: `world.install_tool("pkexec", "printf '%s|' \"$@\" >> log")` puts a stand-in first on the PATH of the Daemon; assert on the log. `confirm: true` is required on the wire; unprivileged fixes (`nmcli radio wifi on`) must not touch `pkexec`.
- S2: first-run screen (`first_run_*` snapshots: NM installed but stopped, networkd active), Diagnostics panel (`diagnostics_*`, Portal), and the `f` / `y` / `n` confirmation flow. Findings arrive on a thread, so `pump_until` for them.
- Never run the pkexec fixes for real on a dev machine; `nexus doctor` (no `--fix`) is read-only and safe anywhere.
## S3: Performance budgets (ticket #31)

`crates/nexus/tests/s3_budgets.rs` measures real release processes against the fake NetworkManager: the Daemon after a busy scenario (40 Networks, 200 rounds of every op, a subscriber), and the real `nexus` TUI on a pty (`rustix` openpt, 120x32). Budgets: Daemon peak RSS (`VmHWM`) < 15 MiB, TUI first frame < 50 ms (spawn to the first ` Nexus ` header byte, best of 5), idle CPU over 5 s (sum of `/proc/<pid>/task/*/schedstat`) < 0.1% Daemon, < 0.2% TUI.

- They are `#[ignore]`d (debug numbers mean nothing). Run: `cargo test --release -p nexus --test s3_budgets -- --ignored --test-threads=1 --nocapture`. Each test prints what it measured; a failure names the budget and the value.
- CI: `.github/workflows/bench.yml` runs that command on every push and PR touching `crates/**`, and writes the numbers to the job summary.
- Reference numbers (dev laptop, release): Daemon 7.3 MiB, TUI first frame ~1 ms, idle CPU Daemon 0.000% / TUI 0.07%.
- To budget a new thing: add a test there; use `World::daemon_pid()` for the Daemon and `Tui::start(&world, &[markers])` + `wait_for` for the TUI.

## Desktop notifications (ticket #13)

- S1 (`crates/nexus/tests/s1_notifications.rs`): `World::builder(bin).notifications(true)` also starts a second private `dbus-daemon` as the session bus with a fake `org.freedesktop.Notifications` on it (default: the session bus points at nothing and the Daemon copes). Read it through `world.notifications`: `notified(n)` (1-based, waits), `count()`, `all()`, `closed()`, `wait_closed(id)`; act as the server with `invoke(id, "enter")` (click an action) and `dismiss(id, reason)` (it timed out).
- The Daemon coalesces bursts, so a test that goes online and offline right after lets a Front end read the `activity` events in between (`front_end(&world)` then `next_event("activity")`), or the two changes merge into nothing. A subscribed Front end also means "someone is looking": no Secret prompt notification is sent while any connection is subscribed.
- The terminal launcher is never mocked in code: `world.install_tool("xdg-terminal-exec", "printf '%s\n' \"$@\" >> log")` shadows the real one on the Daemon's `PATH`. `WorldBuilder::env("TERMINAL", ..)` sets a variable for the Daemon. The choice between `xdg-terminal-exec` and `$TERMINAL -e` is unit-tested in `daemon/notify.rs`.

## Profile settings and checkpoints (ticket #18)

- S1 (`crates/nexus/tests/s1_profile.rs`): `profile_get`, `profile_update`, `profile_confirm`, `profile_revert` over the wire, the `nexus profile show|set` CLI, and `profile_pending` / `profile_settled` events. `profile_update` takes `rollback_secs`, so the missed-confirmation test runs the real timer for one second instead of fifteen.
- The fake NetworkManager serves `Update2` with a checked `version-id`, a `VersionId` property and the checkpoint calls (`CheckpointCreate/Destroy/Rollback`, with a real timer that restores the Profiles). Assert with `nm.checkpoints_open()`, `nm.checkpoint_rollbacks()`, `nm.profile_text(uuid, "ipv4", "method")`, `nm.profile_int(..)`, `nm.profile_version(uuid)`. `nm.edit_profile_externally(uuid)` plays `nmcli` changing the Profile behind the Daemon's back (a stale `version` then fails with `conflict`).
- A Profile counts as "in use" (and so is checkpointed) when a Wi-Fi Device is associated with its SSID: join it first with `wifi_join`.
- S2 (`crates/nexus/tests/s2_profile.rs`): the editor (`e` on a saved row) at narrow/standard/wide, the inline complaint, the countdown (`Reverting in 15s`; the number only moves on `Msg::Tick`, so frames never depend on the clock), keep with `y`, revert with `n`. Worlds come from `nexus_testkit::scenarios::profile_busy`, the same one `demo profile` shows.
