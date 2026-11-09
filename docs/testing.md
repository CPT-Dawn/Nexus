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
- Portals (`crates/nexus/tests/s1_portal.rs`): `nm.set_connectivity(NmConnectivity::Portal)` with a primary Link is a Portal. The browser is a stand-in: `world.install_tool("xdg-open", "echo \"$1\" >> log")` records the URL, so no browser or request ever happens. With `.notifications(true)`, `notes.invoke(id, "open-login")` plays the click on "Open login page". Disable `joined`/`lost` toasts in the config to keep counts simple, and use a `Lost` toast as the sentinel when asserting that nothing else was sent.
- Ethernet (`crates/nexus/tests/s1_ethernet.rs`): `nm.add_ethernet_device("enp3s0")` adds a port with its cable in at 1 Gbps on a network that answers DHCP. `nm.set_carrier(dev, false)` pulls the cable (Device state 20, reason 40; activating is refused), `set_link_speed`, `set_lease(dev, Lease{..})` (address, prefix, gateway, DNS), `set_dhcp(dev, false)` makes the network ignore DHCP (a join dies in state 70 with reason 5, which reads as a misconfigured port), `set_port_eap(dev, identity, password)` makes the port demand 802.1X (a plain Profile gets no address; a wrong login dies in NEED_AUTH with reason 7). `add_ethernet_profile(uuid, id, iface)` saves a plain DHCP Profile; `ethernet_up`, `ethernet_profiles()` and `profile_text(uuid, "802-1x", "identity")` assert. A failed join leaves the Device failed (state 120) and its active connection empty, like NetworkManager; the Daemon deletes a Profile it just made and restores one it edited. `scenarios::ethernet_busy` is `demo ethernet`.
- Keyring (`crates/nexus/tests/s1_keyring.rs`): `World::builder(bin).keyring(true)` starts a fake Secret Service (`FakeSecretService`, plain sessions only, one unlocked `login` collection) on the private session bus, shared with the fake Notifications when both are asked for; without it no keyring answers. `world.keyring`: `put(uuid, secret)`, `secret_of(uuid)`, `items()`, `reads()`, `stop()` (the keyring goes away mid-test). The fake NM treats `psk-flags` bit 1 as agent-owned: the stored secret is cleared, `GetSecrets` on the Profile returns nothing, and activating the Profile makes it ask the Daemon's agent; `SaveSecrets`/`DeleteSecrets` carry the whole connection (uuid, flags), like the real thing. Profile settings carry `keyring: bool`; assert NM's side with `nm.profile_int(uuid, "802-11-wireless-security", "psk-flags")`.
- Waybar and the launcher menu (`crates/nexus/tests/s1_bar.rs`): `nexus status --waybar --follow` is spawned with a piped stdout and its JSON lines are read on a channel (`Follow::until` skips lines until one matches, since a change can print several). The menu is driven with a fake launcher: `fake_launcher(&world, "fuzzel")` installs a shell script that logs its arguments, prints the menu line containing `$TMP/pick`, and in hidden-input mode (`--password`, `-password`, `--hide-input=true`) prints the next line of `$TMP/answers`. Write the config through `configure` (it waits for `config_reloaded`). Tests that need no launcher override `PATH` on the command only.
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
- The Ethernet tab (`tests/s2_ethernet.rs`) uses `ethernet_busy`: `enp3s0` plugged in behind an 802.1X switch (misconfigured), `enp4s0` unplugged. Commands come back as `Msg::Wired`; `rig.settle()` (drain until the Daemon is quiet for 400 ms) before any snapshot taken after a join, since the port's state changes arrive as separate `ethernet_changed` events.
- Portal frames (`tests/s2_portal.rs`) use the same pattern with `app.attach`; `o` sends `portal_open` to the Daemon, which runs the stand-in `xdg-open`.
- Palette, help and mouse (`tests/s2_palette.rs`): `rig.palette("forget")` types `:` and a query; `rig.click_on("text")` renders (which records the clickable areas), finds the text on screen and sends a left click there, `click_footer` does the same on the bottom row, `rig.mouse(kind, x, y)` sends any mouse event. Mouse events are `Msg::Mouse`; only a left click and the wheel redraw.
- Effects (`tests/s2_effects.rs`): effects are a post-pass, so `tui::render` stays effect-free and deterministic. `tui::render_fx(&mut app, w, h, step)` draws a frame and runs the effects for `step` of virtual time; `app.animating()` says whether any still run and `app.next_tick()` is `None` once they end. Settle after setup with a loop of `render_fx` until `!animating()`. The zero-idle proof there runs the real `event_loop` and counts frames after the last effect ended. Tests that count frames over a quiet window (like `an_idle_tui_draws_no_frames`) must wait for `!app.animating()` first: a toast or tab change runs a 200-300 ms effect.
- Settings and themes (`tests/s2_settings.rs`): `settings_world` (also `demo settings` / `settings-bad`) starts a Daemon on a commented config with a good custom theme (`sunrise`) and a mistaken one (`broken`). Keys edit the real file through `config_set`; the screen changes only when `config_reloaded` comes back, so wait with `pump_until(.., |a| a.look().theme.name == ..)`. `every_theme_renders_at_every_width` snapshots the styled frame of every theme at narrow/standard/wide; theme contrast and token rules are unit tests in `tui/theme.rs`. A hand edit is `world.write_config(..)`.
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

`crates/nexus/tests/s3_budgets.rs` measures real release processes against the fake NetworkManager: the Daemon after a busy scenario (40 Networks, 200 rounds of every op, a subscriber), and the real `nexus` TUI on a pty (`rustix` openpt, 120x32). Budgets: Daemon peak RSS (`VmHWM`) < 15 MiB, TUI first frame < 50 ms (spawn to the first ` Nexus ` header byte, best of 5), idle CPU over 5 s (sum of `/proc/<pid>/task/*/schedstat`) < 0.1% Daemon, < 0.2% TUI on the Overview (its traffic graph redraws at 1 Hz, allowed by ADR-0003), and strictly 0.000% (< 0.001%) for the TUI parked on Settings, a tab with no graph. The link thread sleeps in `poll` while the graphs are hidden, so any new timer or poll on a graph-free tab fails that check.

- They are `#[ignore]`d (debug numbers mean nothing). Run: `cargo test --release -p nexus --test s3_budgets -- --ignored --test-threads=1 --nocapture`. Each test prints what it measured; a failure names the budget and the value.
- CI: `.github/workflows/bench.yml` runs that command on every push and PR touching `crates/**`, and writes the numbers to the job summary.
- Reference numbers (dev laptop, release): Daemon 7.3 MiB, TUI first frame ~1 ms, idle CPU Daemon 0.000% / TUI 0.07%.
- To budget a new thing: add a test there; use `World::daemon_pid()` for the Daemon and `Tui::start(&world, &[markers])` + `wait_for` for the TUI.

## Desktop notifications (ticket #13)

- S1 (`crates/nexus/tests/s1_notifications.rs`): `World::builder(bin).notifications(true)` also starts a second private `dbus-daemon` as the session bus with a fake `org.freedesktop.Notifications` on it (default: the session bus points at nothing and the Daemon copes). Read it through `world.notifications`: `notified(n)` (1-based, waits), `count()`, `all()`, `closed()`, `wait_closed(id)`; act as the server with `invoke(id, "enter")` (click an action) and `dismiss(id, reason)` (it timed out).
- The Daemon coalesces bursts, so a test that goes online and offline right after lets a Front end read the `activity` events in between (`front_end(&world)` then `next_event("activity")`), or the two changes merge into nothing. A subscribed Front end also means "someone is looking": no Secret prompt notification is sent while any connection is subscribed.
- The terminal launcher is never mocked in code: `world.install_tool("xdg-terminal-exec", "printf '%s\n' \"$@\" >> log")` shadows the real one on the Daemon's `PATH`. `WorldBuilder::env("TERMINAL", ..)` sets a variable for the Daemon. The choice between `xdg-terminal-exec` and `$TERMINAL -e` is unit-tested in `daemon/notify.rs`.

## Enterprise Wi-Fi / 802.1X (ticket #16)

- S1 (`crates/nexus/tests/s1_enterprise.rs`): `wifi_join_eap` over the wire and `nexus wifi join <ssid> --eap peap ...` on the CLI. `Ap::enterprise(..)` is an 802.1X access point; `nm.set_eap_user(ssid, identity, password)` makes any other pair fail like a refused sign-in; `nm.eap_setting(ssid, key)` reads back what the Daemon put in the Profile's `802-1x` setting as text (certificates show as `file://<path>`).
- S2 (`crates/nexus/tests/s2_enterprise.rs`): the sign-in form at narrow/standard/wide. World: `nexus_testkit::scenarios::wifi_enterprise` (also `demo wifi-enterprise`). The form starts on Identity; `Tab`/`BackTab` move, `Left`/`Right` change a choice.
- Real mode: `rig/aps.d/eap.conf` is a hostapd access point (`NexusEap`) with the built-in EAP server; `rig/inner.sh` generates a throwaway CA, server certificate (`radius.nexus.test`) and a `student` client certificate under `/run/nexus-rig/pki`. User `student` / `correct-horse-4`. Needs `openssl` on the machine.
## Bluetooth Tethering (ticket #23)

- S1 (`crates/nexus/tests/s1_tether.rs`): `World::builder(bin).bluez(true)` (or `.bluez(false)` for an adapter that is off) starts a fake `org.bluez` on the same private system bus: one `org.bluez.Adapter1` and an ObjectManager. `world.bluez().set_powered(bool)` flips it and emits `PropertiesChanged`; `stop()`/`restart()` take the service away. Without `.bluez(..)` nobody owns `org.bluez` (Bluetooth reads as `absent`), and the Daemon must not start it by looking.
- Phones are the fake NetworkManager's Bluetooth Devices: `nm.add_phone(addr, name)` (NAP, listed), `nm.add_dialup_only_phone(..)` (DUN only, not listed). A Tethering Link is a state walk 40, 50, 70, 100 like a join; `nm.script_tether_failure(addr, died_in, reason)` kills it. Assert with `nm.tethered(dev)`, `nm.phone_profiles(addr)` (memory + disk), `nm.phone_profiles_persisted(addr)`.
- S2 (`crates/nexus/tests/s2_tether.rs`): Tethering lives on the Hotspot tab (key `5`). `scenarios::tether_world(bin, powered)` is shared with `demo tether` / `demo tether-off`; the tests' own `Rig` waits for `app.tether().has_data()`.
## Hotspot (ticket #22)

- S1 (`crates/nexus/tests/s1_hotspot.rs`): `hotspot`, `hotspot_start {ssid?, password?}`, `hotspot_stop`, `hotspot_reveal` over the wire and `nexus hotspot status|start|stop|qr` on the CLI (exit code 26 `hotspot_unsupported`; a Hotspot that fails to start is `join_failed` 8, a bad name or password `invalid_settings` 15). Fake NM: `nm.set_ap_support(dev, false)` makes a card that cannot host one (a Device with no AP capability bit), `nm.script_hotspot_failure(true)` kills the next Hotspot on its way up. Assert with `nm.hotspots()` (names of those up), `nm.hotspot_profiles()` (zero once stopped), `nm.hotspot_psk(ssid)`, `nm.hotspot_setting(ssid, section, key)`. The password is only read back through `hotspot_reveal`: `nm.secret_reads()` stays 0 before it.
- NetworkManager does not list the stations that joined, so the Daemon runs `iw dev <interface> station dump` and counts the `Station` lines (no `iw`: the count is unknown, not zero). Tests put a stand-in on the world's PATH: `world.install_tool("iw", ..)` reading `$HOME/stations`; `scenarios::hotspot_stations(world, n)` writes it. While a Hotspot runs the Daemon re-counts every `[hotspot] poll_secs` (default 5, 1 to 60; tests set 1) and pushes `hotspot_changed` only when the Hotspot or its count differs. Nothing is polled while none runs.
- `hotspot_changed` does not fire for the machine *becoming able* to host one (a card plugged in, the radio switched): the TUI asks again when its Hotspot tab opens. That keeps unrelated event streams (`s1_wifi`) free of hotspot lines.
- S2 (`crates/nexus/tests/s2_hotspot.rs`): the panel above the Tethering list on the Hotspot tab. Worlds: `scenarios::hotspot_world(bin, HotspotStart::{Capable, Running, NoCard})`, the same ones `demo hotspot`, `demo hotspot-on`, `demo hotspot-nocard` show. The rig waits for both `app.tether().has_data()` and `app.hotspot().has_data()` and settles 400 ms before snapshotting; `s2_tether.rs` does the same. Keys: `n` start, `c` form, `s` QR (asks first), `S` stop.

## Profile settings and checkpoints (ticket #18)

- S1 (`crates/nexus/tests/s1_profile.rs`): `profile_get`, `profile_update`, `profile_confirm`, `profile_revert` over the wire, the `nexus profile show|set` CLI, and `profile_pending` / `profile_settled` events. `profile_update` takes `rollback_secs`, so the missed-confirmation test runs the real timer for one second instead of fifteen.
- The fake NetworkManager serves `Update2` with a checked `version-id`, a `VersionId` property and the checkpoint calls (`CheckpointCreate/Destroy/Rollback`, with a real timer that restores the Profiles). Assert with `nm.checkpoints_open()`, `nm.checkpoint_rollbacks()`, `nm.profile_text(uuid, "ipv4", "method")`, `nm.profile_int(..)`, `nm.profile_version(uuid)`. `nm.edit_profile_externally(uuid)` plays `nmcli` changing the Profile behind the Daemon's back (a stale `version` then fails with `conflict`).
- A Profile counts as "in use" (and so is checkpointed) when a Wi-Fi Device is associated with its SSID: join it first with `wifi_join`.
- S2 (`crates/nexus/tests/s2_profile.rs`): the editor (`e` on a saved row) at narrow/standard/wide, the inline complaint, the countdown (`Reverting in 15s`; the number only moves on `Msg::Tick`, so frames never depend on the clock), keep with `y`, revert with `n`. Worlds come from `nexus_testkit::scenarios::profile_busy`, the same one `demo profile` shows.

## WireGuard and the VPN tab (ticket #20)

- S1 (`crates/nexus/tests/s1_vpn.rs`): `vpn_list`, `vpn_up`/`vpn_down`, `vpn_import`, `vpn_create`, `vpn_key`, `vpn_reveal`, `vpns_changed` and the `nexus vpn ...` CLI. The fake NetworkManager serves `Settings.AddConnection`, `Manager.ActiveConnections`/`DeactivateConnection` and a `Connection.Active` object per VPN that is up. A VPN Profile keeps the settings as added (private key included) but `GetSettings` leaves the private key and pre-shared keys out, like the real one; `GetSecrets("wireguard")` hands them back and counts as a `secret_read`.
- Fixtures: `nm.add_plugin_vpn(uuid, name)` (a `vpn`-type Profile), `nm.vpn_uuid(name)`, `nm.vpn_active_state(uuid)` (0, 1 coming up, 2 up), `nm.drop_vpn(uuid)` (the tunnel dies on its own: this is what the drop alert reacts to), `nm.script_vpn_failure(uuid, bool)`, `nm.vpn_private_key(uuid)`, `nm.vpn_setting(uuid, section, key)`.
- Keys are checked against RFC 7748 section 6.1 (Alice), not recomputed in the test. The parser is fuzzed with `proptest` in `wireguard.rs` (arbitrary text never panics; every rendered config reads back unchanged).
- S2 (`crates/nexus/tests/s2_vpn.rs`): the VPN tab (key `4`) at narrow/standard/wide, the forms (`i` import, `c` create), the key view (`p`) and the config QR (`s`, then `enter`). A config QR is about 65 modules wide and 33 rows tall, so it needs a window of roughly 70 by 38: the tests draw it at 120x46 and snapshot the "needs a window" fallback at 100x28. Worlds come from `scenarios::vpn_busy`, the same one `demo vpn` shows.
- Real mode (`real_mode.rs`, `a_wireguard_profile_round_trips_through_real_networkmanager`): imports a config, lists it, reads the key and the reveal back, brings the tunnel up and down with real NetworkManager and the kernel's wireguard module, no peer answering. `RealWorld::start` accepts a `DEVTYPE=wireguard` device next to the radios.

## Plugin VPNs (ticket #21)

- S1 (`crates/nexus/tests/s1_plugin_vpn.rs`): `vpn_import` with `plugin` set, the missing-plugin errors, and the VPN Secret prompts. The world has no VPN plugin installed until `world.install_vpn_plugin("openvpn")` drops an `nm-openvpn-service.name` into a world directory the Daemon reads through `NEXUS_VPN_PLUGIN_DIRS` (the same variable can point a real Daemon elsewhere). `world.install_fake_nmcli()` puts a stand-in `nmcli` on the `PATH`: it supports only `connection import type <t> file <f>`, adds a `vpn` Profile named after the file to the fake NetworkManager over `busctl`, refuses a file with no `remote` line, and logs the file and directory modes it saw (`world.nmcli_log()`), which the test uses to prove the provider file was 0600 in a 0700 directory and is gone afterwards.
- Prompts: `nm.add_plugin_vpn_of(uuid, name, Some("openvpn"))` gives the Profile a `vpn.service-type`; `nm.script_vpn_asks(uuid, &["username", "password", "otp"])` makes activation call the Nexus secret agent's `GetSecrets("vpn", hints)` first; `nm.vpn_answers(uuid)` returns what the agent handed back. A hint `x-vpn-message:<text>` becomes the prompt sentence. A Profile without a `service-type` is never checked for its plugin.
- S2 (`s2_vpn.rs`): prompt overlays at narrow/standard/wide, the user name shown as typed, the code masked, Esc declining, `.ovpn` import and the missing-plugin toast. `s1_notifications.rs` covers the "One-time code needed" notification.
- Demo: `cargo run -p nexus-testkit --example demo -- target/debug/nexus vpn-plugin`, press `4`.

## Trust and auto-VPN (ticket #25)

- S1 (`crates/nexus/tests/s1_trust.rs`): defaults (open Untrusted, secured Trusted), `trust_set` and its persistence across a Daemon restart (the world's `$HOME/.local/state/nexus/trust.json`), Untrusted join brings the VPN up and notifies, Portal then VPN, Trusted join untouched, metered marking (`world.nm.profile_int(uuid, "connection", "metered")`), the `policy` notification switch, and the CLI.
- The fake does not make a joined Network the primary connection or set connectivity by itself. After `wifi_join`, a test calls `go_online(&world, ssid, NmConnectivity::Full)` (set primary, connectivity, state), as NetworkManager would; `Portal` instead of `Full` holds the policy back until it is switched to `Full`.
- Demo: `cargo run -p nexus-testkit --example demo -- target/debug/nexus wifi`, press `2`, pick an open Network (Cafe Guest), press `t`.

## Wi-Fi edge cases (ticket #15)

- S1 (`crates/nexus/tests/s1_wifi_edge.rs`): `wifi_join` with `hidden` (a security) and `device` (interface name or id), `wifi_aps`, the CLI (`wifi join --hidden <open|owe|wpa2|wpa3|wpa2-3>`, `--device`, `--hex` for names that are not UTF-8, `wifi aps`). Fake NM: `Ap::hidden()` (`Ssid` reads empty until a Device associates, and a hidden join nobody hears fails with reason 53), `Ap::owe`, `Ap::transition`, `Ap::bitrate(kbps)`, `nm.added_setting(ssid_bytes, "802-11-wireless-security.key-mgmt")` to see which Profile the Daemon built. A second Device is just another `add_wifi_device`.
- SSIDs are untrusted bytes: `nexus_ipc::display_ssid` replaces invalid UTF-8, control and direction-override characters before a name reaches any Front end; the `WifiNetwork.id` (hex of the bytes) is what joins.
- S2 (`crates/nexus/tests/s2_wifi_edge.rs`): the hidden form (`n`), access point details (`i`), the card chooser (`c`) at narrow/standard/wide. World: `nexus_testkit::scenarios::wifi_edge`, also `demo wifi-edge`.
