# NEXUS

**A modern, memory-safe TUI WiFi manager built in Rust for Linux.**

[![Rust](https://img.shields.io/badge/Rust-2024_Edition-f74c00?logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Wayland Native](https://img.shields.io/badge/Wayland-Native-yellow?logo=wayland&logoColor=white)](#)
[![Maintenance](https://img.shields.io/badge/Maintained-Actively-brightgreen.svg)](#)

![Demo](assets/demo.gif)

---

## Why Nexus

Most WiFi management on Linux falls into two camps: graphical network applets coupled to a full desktop environment, or raw `nmcli` one-liners piped through shell scripts. The former drags in hundreds of megabytes of GUI toolkit dependencies. The latter demands memorising a dense CLI grammar for what should be a simple, visual task.

Nexus is neither. It is a single, statically-optimised binary that speaks directly to NetworkManager over the system D-Bus — no subprocesses, no `nmcli` wrappers, no GTK, no Qt. It renders a composable TUI via `ratatui` and `crossterm`, using `Color::Reset` backgrounds by default so your terminal's transparency, blur, and color scheme pass through untouched.

Built in safe, modern Rust (2024 edition) with `zbus` for type-safe, pure-Rust D-Bus IPC and `tokio` for fully async I/O, Nexus targets users who live in the terminal — Arch, NixOS, Void, or any distribution running NetworkManager.

---

## Features

- **Direct D-Bus IPC** — communicates with `org.freedesktop.NetworkManager` via `zbus`. Zero subprocess spawning; no shell, no `nmcli`, no stdout parsing.
- **Signal-driven architecture** — subscribes to `org.freedesktop.DBus.Properties.PropertiesChanged` on the WiFi device object. State updates arrive as D-Bus signals with a 2-second debounce; blind polling is only a fallback when signal subscription fails.
- **Async, non-blocking core** — `tokio` multi-threaded runtime with crossterm's async `EventStream`. A unified `mpsc` event channel multiplexes terminal input, render ticks, D-Bus signals, and network command results. No worker thread is ever blocked on I/O.
- **Full WiFi lifecycle** — scan, connect (WPA/WPA2/WPA3/WEP/Open), disconnect, forget saved profiles, hidden network SSID entry — all via typed `NetworkCommand` variants, not stringly-typed messages.
- **Animated UI** — smooth signal-strength interpolation (exponential ease-out), braille/bar/pulse spinners, cubic ease-out dialog slide-in transitions, configurable up to 144 FPS. Disable entirely with `animations = false`.
- **Configurable theme engine** — every color is user-defined via TOML. Supports named colors, `"reset"` (terminal default / transparency), and `#RRGGBB` true color hex. Ship your Catppuccin, Gruvbox, or Dracula palette.
- **Transparency-native** — backgrounds default to `Color::Reset`. Terminal blur, opacity, and compositor effects are preserved.
- **Nerd Font icons** with automatic plain-Unicode fallback (`--no-nerd-fonts`).
- **Vim-native navigation** — `j`/`k`/`g`/`G` alongside arrow keys and Home/End. Designed for `hjkl` muscle memory.
- **Inline search** — real-time `/` filtering across the network list.
- **Multi-sort modes** — cycle through signal strength, alphabetical, security type, and frequency band with `S`.
- **Detail panel** — toggle a split-view panel showing BSSID, channel, frequency, IPv4/IPv6, gateway, DNS, MAC address, and link speed for the active connection.
- **Embedded config bootloader** — `default_config.toml` is baked into the binary via `include_str!`. First launch writes `~/.config/nexus/config.toml` automatically. Delete to regenerate. The binary can never fail to start due to a missing config.
- **CLI override layer** — any config value can be overridden per-invocation (`--interface`, `--fps`, `--no-nerd-fonts`, `--log-level`, `--config`).
- **Trait-abstracted backend** — the `NetworkBackend` trait cleanly separates D-Bus logic from UI, enabling future `iwd` or mock backends without touching rendering code.
- **Release-optimised** — `opt-level = 3`, full LTO, single codegen unit, symbol stripping. Minimal binary footprint.

---

## Requirements

| Dependency | Purpose | Install |
|---|---|---|
| **NetworkManager** | WiFi backend (D-Bus API) | `sudo pacman -S networkmanager` |
| **D-Bus** (system bus) | IPC transport | Included in `dbus` / `systemd` |
| **Rust ≥ 1.85** | Build toolchain (2024 edition) | [rustup.rs](https://rustup.rs/) |
| **A Nerd Font** *(optional)* | Icon glyphs | [nerdfonts.com](https://www.nerdfonts.com/) |

> [!IMPORTANT]
> Nexus renders at the PTY layer and works on **any terminal emulator** — X11, Wayland, or a raw TTY. It does not depend on a specific display server or compositor. However, if you are running a minimal Wayland compositor (e.g. **Hyprland**, **Sway**, **river**) without a full desktop environment, ensure that NetworkManager is running — most minimal setups do not start it by default.

---

## Installation

### Building from Source

```bash
git clone https://github.com/CPT-Dawn/Nexus.git
cd Nexus
cargo build --release
```

The binary is placed at `target/release/nexus`. Install it:

```bash
sudo install -Dm755 target/release/nexus /usr/local/bin/nexus
```

The release profile ships with `opt-level = 3`, full LTO, symbol stripping, and single codegen unit for a minimal binary.

### Arch Linux (AUR)

```bash
yay -S nexus
```

> [!NOTE]
> The AUR package name is a placeholder and will be updated once the package is published.

### Uninstall

```bash
sudo rm /usr/local/bin/nexus
rm -rf ~/.config/nexus ~/.local/share/nexus
```

---

## Configuration

Nexus uses an **embedded asset bootloader** pattern. The full default configuration is compiled into the binary at build time via `include_str!("../default_config.toml")`. On first launch, if no config file exists, Nexus writes the embedded defaults to disk automatically. The application can never crash due to a missing config file.

**Config path:** `~/.config/nexus/config.toml`
**Log path:** `~/.local/share/nexus/nexus.log` (daily rotation)

Delete the config file to regenerate defaults on next launch, or dump the built-in defaults at any time:

```bash
nexus --print-default-config > ~/.config/nexus/config.toml
```

### Config Structure

```toml
[general]
interface = ""              # WiFi interface (empty = auto-detect)
log_level = "info"          # trace | debug | info | warn | error
scan_interval_secs = 5      # D-Bus poll fallback interval (seconds)

[appearance]
nerd_fonts = true           # false → plain Unicode fallback
animations = true           # false → instant updates, no easing
fps = 60                    # Render loop target (30–144)
show_details = true         # Detail panel visible on launch
border_style = "rounded"    # rounded | plain | thick | double

[theme]
bg = "#0D0B14"              # Background (use "reset" for transparency)
fg = "#E0DEE6"              # Primary text
fg_dim = "#4A4458"          # Dimmed / inactive text
accent = "#00FFFF"          # Selected items, active borders, key hints
accent_secondary = "#FF4500"# Section headers in detail panel
border = "#2A2438"          # Inactive borders
border_focused = "#00FFFF"  # Active panel border

[theme.semantic]
connected = "#00FF9F"       # Connected / success indicator
warning = "#FFB347"         # Open networks, rfkill
error = "#FF4500"           # Error text
selected_bg = "#1E1A2E"    # Selected row background

[theme.signal]
excellent = "#00FF9F"       # 80–100%
good = "#00FFFF"            # 60–79%
fair = "#FFB347"            # 40–59%
weak = "#FF4500"            # 20–39%
none = "#4A4458"            #  0–19%

[keys]
scan = "s"
connect = "enter"
disconnect = "d"
forget = "f"
hidden = "h"
details = "i"
refresh = "r"
help = "?"
quit = "q"
sort = "S"
search = "/"
```

All color values accept named colors (`"red"`, `"cyan"`, `"darkgray"`, …), `"reset"` / `"transparent"` for the terminal default, or `"#RRGGBB"` hex for true color.

---

## Usage

```bash
nexus                                # Auto-detect interface, default config
nexus --interface wlan0              # Use a specific WiFi interface
nexus --no-nerd-fonts                # Plain Unicode (no Nerd Font required)
nexus --fps 144                      # High-refresh rendering
nexus --log-level debug              # Verbose file logging
nexus --config /path/to/custom.toml  # Custom config file
nexus --print-default-config         # Dump embedded defaults to stdout
```

### CLI Flags

| Flag | Description |
|---|---|
| `-i`, `--interface <IFACE>` | Override WiFi interface (e.g. `wlan0`) |
| `-l`, `--log-level <LEVEL>` | Override log level |
| `-c`, `--config <PATH>` | Use a custom config file path |
| `--fps <N>` | Override target FPS |
| `--no-nerd-fonts` | Disable Nerd Font icons |
| `--print-default-config` | Print built-in defaults to stdout and exit |

### Keybindings

All action keys are remappable in the `[keys]` config section. Navigation keys (`j`/`k`, arrows, `g`/`G`) and modifier combos (`Ctrl+H`) are hard-coded.

| Key | Action |
|---|---|
| `↑` / `k` | Move up |
| `↓` / `j` | Move down |
| `g` / `Home` | Jump to first network |
| `G` / `End` | Jump to last network |
| `Enter` | Connect to selected network |
| `d` | Disconnect active connection |
| `s` | Trigger WiFi scan |
| `f` | Forget saved network profile |
| `h` | Connect to hidden SSID |
| `i` | Toggle detail panel |
| `S` | Cycle sort mode (Signal → A-Z → Security → Band) |
| `/` | Inline search / filter |
| `r` | Refresh connection info |
| `?` | Help overlay |
| `Ctrl+H` | Toggle password visibility (in dialogs) |
| `Tab` | Switch fields (in multi-field dialogs) |
| `Esc` | Close dialog / clear filter / quit |
| `q` | Quit |

---

## Architecture

```
src/
├── main.rs              # Entry point, terminal setup, async event loop
├── app.rs               # Application state machine, key dispatch, action routing
├── config.rs            # TOML parsing, CLI args (clap), embedded config bootloader
├── event.rs             # Async event multiplexer (input, ticks, D-Bus, commands)
├── animation/
│   ├── mod.rs           # AnimationState bitflags, tick driver, cubic ease-out
│   ├── spinner.rs       # Braille, bar, and pulse frame generators
│   └── transitions.rs   # Signal smoothing (exponential ease-out), fade-in curves
├── network/
│   ├── mod.rs           # NetworkBackend trait (async, swap NM / iwd / mock)
│   ├── manager.rs       # NmBackend — full D-Bus implementation via zbus
│   ├── signals.rs       # D-Bus PropertiesChanged signal listener + polling fallback
│   └── types.rs         # WiFiNetwork, ConnectionInfo, SecurityType, FrequencyBand
└── ui/
    ├── mod.rs           # Root layout, modal overlay dispatch, size guards
    ├── theme.rs         # Runtime Theme struct, Nerd Font icon constants, style builders
    ├── header.rs        # Title bar with live connection status
    ├── network_list.rs  # Scrollable network list with signal bars + security badges
    ├── details.rs       # Split-view detail panel (IP, MAC, channel, speed, …)
    ├── password.rs      # Password input modal with visibility toggle
    ├── hidden.rs        # Hidden network SSID + password modal
    ├── help.rs          # Keybinding reference overlay
    └── status_bar.rs    # Context-sensitive footer hints
```

### Design Decisions

- **`NetworkBackend` trait** — abstracts the WiFi backend behind an async interface. The current `NmBackend` targets NetworkManager over D-Bus. An `iwd` or mock implementation can be dropped in without touching any UI or event code.
- **Single-channel event architecture** — all `tokio::spawn` tasks (terminal input, tick generation, D-Bus signal listener, network operations) feed one `mpsc::UnboundedSender<Event>`, consumed sequentially by the main loop. No shared mutable state, no locks.
- **Config layering** — embedded TOML defaults → user config file → CLI flags. Each layer overrides the previous. The binary is always fully self-contained.
- **`Color::Reset` support** — setting `bg = "reset"` uses the terminal's native background, preserving transparency, blur, and whatever your compositor provides.
- **Zero-allocation animation state** — `AnimationState` uses a `u8` bitflag instead of `HashSet` for tracking active animations. Cache-friendly, zero heap allocation.

---

## Troubleshooting

**"NetworkManager is not running"**
```bash
sudo systemctl enable --now NetworkManager
```

**No WiFi adapter detected**
```bash
ip link                     # Verify your interface is visible
rfkill list                 # Check for soft/hard blocks
sudo rfkill unblock wifi
```

**Icons render as boxes or question marks**
Install a [Nerd Font](https://www.nerdfonts.com/) and configure your terminal to use it, or launch with `--no-nerd-fonts`.

**Config parse error on startup**
Delete the config file to regenerate from embedded defaults:
```bash
rm ~/.config/nexus/config.toml
nexus
```

---

## Contributing

Contributions are welcome. Please open an issue to discuss non-trivial changes before submitting a pull request.

1. Fork the repository
2. Create a feature branch (`git checkout -b feat/my-feature`)
3. Commit with clear messages (`git commit -m "feat: add foobar support"`)
4. Ensure `cargo clippy` and `cargo check` pass with zero warnings
5. Open a Pull Request

---

## License

[MIT](LICENSE) © Swastik Patel