# Nexus v2 crate selection (as of 2026-11-09)

Sources: crates.io API (versions, release dates; https://crates.io/crates/<name>), GitHub API (pushed_at), RustSec advisory-db clone (https://github.com/rustsec/advisory-db). "Last release" = crates.io version date. Not verified: runtime behaviour of any crate; I did not build anything. Version bumps are fast, re-check at lock time.

IMPORTANT correction: zbus is at **6.2.1** (2026-11-01), not 5.x. zbus_xmlgen 6.0.0 (2026-10-16). oo7 0.7.0 still depends on zbus ^5.16 (and nmrs on zbus 5.x? see below), so mixing may duplicate zbus 5 + 6 in the tree.

RustSec: advisory-db has NO entries for ratatui, crossterm, zbus, nmrs, iwdrs, rtnetlink, neli, x25519-dalek, boringtun, oo7, keyring, zeroize, secrecy, notify-rust, ksni, tarpc, jsonrpsee, postcard, toml, notify, miette, color-eyre, qrcode, figment. Only tracing-subscriber RUSTSEC-2025-0055 (ANSI log poisoning, patched >=0.3.20; 0.3.23 is fine). Unmaintained notices exist for `paste`, `bincode` (RUSTSEC-2025-0141), `proc-macro-error`, `instant`, `atty`, `number_prefix`, `fxhash`, `yaml-rust`, `derivative`: avoid bincode (use postcard), and check `cargo deny` for transitive `paste` (older ratatui pulled it).

## TUI
| Concern | Recommended | Version / last release | Why | Runner-up |
|---|---|---|---|---|
| Framework | ratatui | 0.30.2, 2026-06-19 | Standard; 0.30 split into ratatui-core/-widgets/backend crates; backends: crossterm, termion, termwiz, new termina | tui-realm (component framework on top), cursive |
| Backend | crossterm | 0.29.0, 2025-04-05 (ratatui-crossterm 0.1.2 supports 0.28 and 0.29) | Default, works on Wayland terminals | termina via ratatui-termina (new, young) |
| Single-line input | tui-input | 0.15.5, 2026-09-26 | Maintained, backend-agnostic (good for password fields) | hand-rolled |
| Multi-line | ratatui-textarea | 0.9.4, 2026-10-31 | Official ratatui org continuation (repo ratatui/ratatui-textarea) | tui-textarea-2 0.13.2 (2026-08-23). AVOID original tui-textarea 0.7.0 (2024-10, repo idle since 2024-12) |
| Effects | tachyonfx | 0.25.2, 2026-09-06 | Active, ratatui org | none; skip if you want minimal deps |
| Widgets | ratatui-widgets | 0.3.2, 2026-06-19 | Re-exported by ratatui; no direct dep needed | |
| Spinner | throbber-widgets-tui | 0.11.1, 2026-06-19 | Maintained, tracks ratatui 0.30 | write own (10 lines) |
| Scroll | tui-scrollview | 0.6.8, 2026-09-24 | Active | ratatui built-in Scrollbar + own state |
| Images/QR graphics | ratatui-image | 11.1.0, 2026-10-18 | Optional kitty/sixel | use text QR |
Caveat: ratatui 0.30 is a workspace split; companion crates must target ratatui-core 0.1 / ratatui 0.30 or you get duplicate types. Check each companion's ratatui req before adding.

## D-Bus and NM
| Concern | Recommended | Version / date | Why | Runner-up |
|---|---|---|---|---|
| D-Bus | zbus (tokio feature) | 6.2.1, 2026-11-01 | De facto; pure Rust; both client and server (needed for daemon service) | dbus-rs (libdbus FFI), no |
| Proxy gen | zbus_xmlgen | 6.0.0, 2026-10-16 | Generates `#[proxy]` traits from introspection XML; commit output, hand-trim | write `#[proxy]` by hand for the ~60 NM members you use |
| NM bindings | **Generate/hand-write own proxies** on zbus | n/a | nmrs is young (created 2025-12-12, 35 releases, 3.5.7 on 2026-10-17, active, 328 stars, MSRV 1.90, Wi-Fi-focused, depends zbus 5.x) so its API churns (major 3 in <1 year) and it hides raw settings dicts, which you need for WireGuard/VPN/802.1X/secrets agent. Use as reference, not dependency | nmrs 3.5.7 (acceptable for prototyping); `networkmanager` 0.5.0 (2025-06, dbus-rs based, stale: avoid) |
| iwd | iwdrs | 0.2.7, 2026-04-30 (repo last push 2026-09) | Only option, zbus-based, small (about 3k dl/mo); thin, so may also hand-write iwd proxies | own zbus proxies |
Note: NM Secret Agent (org.freedesktop.NetworkManager.SecretAgent) must be implemented as a zbus server by Nexus; no crate does this.

## Netlink
| Concern | Recommended | Version / date | Why | Runner-up |
|---|---|---|---|---|
| Link stats / rates | rtnetlink + netlink-packet-route | 0.24.0 (2026-11-04) / 0.34.0 (2026-11-03) | Active (rust-netlink org), tokio async, RTM_GETLINK stats64 | **Simplest: read /sys/class/net/<if>/statistics/{rx,tx}_bytes** (no dep) |
| nl80211 | wl-nl80211 | 0.9.0, 2026-11-04 | Same org; station info/signal | NM/iwd D-Bus already expose strength/bitrate; neli 0.7.5 (2026-10-20) generic-netlink alternative |

## Crypto, secrets
| Concern | Recommended | Version / date | Why | Runner-up |
|---|---|---|---|---|
| WireGuard keygen | x25519-dalek (feature static_secrets) + rand_core/getrandom | 3.0.0, 2026-07-06 | Audited-ish, standard; clamp handled; base64 encode pubkey | boringtun 0.7.1 (2026-05) is a whole userspace WG: overkill. Or shell `wg genkey` |
| Secret Service | oo7 | 0.7.0, 2026-11-05 (MSRV 1.95, pre-releases alpha Jul/beta Sep) | Pure-Rust, async, zbus-based, active (linux-credentials org), supports file backend fallback | keyring 4.2.0 (2026-08-29): simple sync API, multi-platform, less control; cannot do attributes lookups as richly |
| Hygiene | zeroize 1.9.1 (2026-10-06) | | ubiquitous; derive ZeroizeOnDrop | |
| Secret wrapper | secrecy 0.10.3 (2024-10-09) | | Stable, low churn (stale but done, no advisories); Debug-redacting wrapper | own newtype over Zeroizing<String> |
Caveat: oo7 depends on zbus ^5.16 while zbus is 6.x: two zbus majors in tree until oo7 moves. Also NM `psk-flags` and agent-owned secrets: let NM own storage where possible; only use Secret Service for what NM cannot store.

## Notifications / tray
| Concern | Recommended | Version / date | Why | Runner-up |
|---|---|---|---|---|
| Notifications | notify-rust (zbus feature, no libdbus) | 4.19.0, 2026-11-08 | Very active, supports actions/replace-id | direct zbus proxy for org.freedesktop.Notifications (already have zbus; about 40 lines, avoids another zbus version) |
| Tray (SNI) | ksni | 0.3.7, 2026-10-20 (pushed 2026-11-09) | Pure Rust SNI, active, MSRV 1.80, tokio/async | tray-icon 0.26.1 (Tauri, GTK-dependent on Linux: avoid) |

## IPC daemon <-> TUI/CLI
| Option | Verdict |
|---|---|
| **zbus session-bus service (org.cptdawn.Nexus1)** | Recommended: signals for state changes, introspectable (busctl/gdbus debug), activation via .service file, peer auth free, same crate as NM client. Cost: verbose types (zvariant), need serde-friendly DTOs |
| Unix socket + postcard (1.1.3, 2025-07, ok; avoid bincode) or JSON lines | Runner-up if you want a non-D-Bus-typed API and streaming |
| tarpc 0.38.0 (2026-08), jsonrpsee 0.26.1 (2026-09) | Active but add heavy dependency trees; not needed |

## Config
| Concern | Recommended | Version / date | Why | Runner-up |
|---|---|---|---|---|
| Parse | toml (serde) | 1.2.1, 2026-11-06 | TOML 1.1 | |
| Preserve comments (when daemon writes) | toml_edit | 0.25.19, 2026-11-06 | | |
| Layering | figment 0.10.19 (2024-05) : stale, works; or plain serde + XDG merge | | | config crate |
| Hot reload | notify | 8.2.2, 2026-11-09 | Watch the parent dir, debounce (notify-debouncer-full) | inotify directly |

## CLI, errors, logging
| Concern | Recommended | Version / date | Why | Runner-up |
|---|---|---|---|---|
| CLI | clap 4.6.7 (2026-09-14) + clap_complete 4.6.11 + clap_mangen 0.3.3 (2026-08-12) | | standard; generate in build.rs/xtask | argh |
| Errors lib | thiserror 2.0.21 (2026-09-23) | | | snafu |
| Errors bin | color-eyre 0.6.5 (2025-05-30) for TUI (needs panic hook restoring terminal) | | | miette 7.6.0 (2025-04-27) for config-diagnostics with spans; anyhow |
| Logging | tracing 0.1.44 + tracing-subscriber 0.3.23 (>=0.3.20 for RUSTSEC-2025-0055) + tracing-journald 0.3.2 (2025-11-26) | | daemon -> journald; TUI -> file in XDG_STATE_HOME (never stdout) | systemd-journal-logger |

## Testing
| Concern | Recommended | Version | Notes |
|---|---|---|---|
| Snapshots | insta 1.49.0 (2026-10-03) + ratatui TestBackend | | Render `buffer` to string |
| Property | proptest 1.12.0 (2026-10-10) | | parsers, state machines |
| PTY e2e | rexpect 0.7.1 (2026-05) | | optional |
| Mock D-Bus | Run a zbus server implementing fake NM on a private `dbus-daemon --session` per test (dbus-daemon from CI image) | | no maintained Rust "dbus-test-runner"; write fake NM in tests |
| Wi-Fi integration | mac80211_hwsim kernel module + hostapd + real NM/iwd in a privileged VM/container | n/a (not a crate) | Needs root/modprobe; run in nightly CI job (not verified here) |

## Misc
| Concern | Recommended | Version / date | Notes |
|---|---|---|---|
| QR | qrcode 0.14.1 (2024-07-05; repo pushed 2025-08) | | Stable, render with unicode half-blocks; WIFI:T:WPA;S:..;P:..;; payload; no advisories | fast_qr alternative |
| Latency probe | surge-ping 0.9.1 (2026-09-16) (needs CAP_NET_RAW or ping_group_range) | | or TCP connect timing with tokio (no privileges) |
| Speed test | reqwest (rustls) download timing against a configurable URL; no good dedicated crate | | |
| Release | cargo-dist 0.34.0 (2026-11-02) for GitHub binaries/installers; AUR via existing workflow (repo already has Aur automation) | | cargo-dist does not do AUR |

## Recommended stack
ratatui 0.30.2 + crossterm 0.29, tui-input, ratatui-textarea, throbber-widgets-tui, tachyonfx optional; zbus 6.2.1 + own proxies (zbus_xmlgen once to bootstrap), iwdrs optional later; /sys + rtnetlink for rates; x25519-dalek 3; oo7 (watch zbus dual-major) ; zeroize + secrecy; zbus for notifications (or notify-rust); ksni; D-Bus session service for IPC; toml + toml_edit + notify; clap suite; thiserror + color-eyre; tracing + journald; insta + proptest; qrcode; cargo-dist.
