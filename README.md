# NEXUS

**A keyboard-driven TUI WiFi manager for Linux, written in Rust.**

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-2024_Edition-f74c00.svg?logo=rust)](https://www.rust-lang.org/)
[![D-Bus](https://img.shields.io/badge/IPC-D--Bus-4a86cf.svg)](https://www.freedesktop.org/wiki/Software/dbus/)
[![NetworkManager](https://img.shields.io/badge/Backend-NetworkManager-4e9a06.svg)](https://networkmanager.dev/)
[![Maintenance](https://img.shields.io/badge/Maintained-actively-brightgreen.svg)]()

![Demo](assets/demo.gif)

---

## Why Nexus

Most WiFi management on Linux falls into two camps: graphical applets tethered to a desktop environment, or raw `nmcli` incantations piped through shell scripts. The former pulls in hundreds of megabytes of GUI toolkit dependencies. The latter demands memorizing a dense CLI grammar for what should be a simple, visual task.

Nexus is neither. It is a single, statically-linkable binary that communicates with NetworkManager directly over the system D-Bus — no subprocesses, no `nmcli` wrappers, no GTK, no Qt. It renders a composable TUI via `ratatui` and `crossterm`, using `Color::Reset` backgrounds throughout so your terminal's transparency, blur, and color scheme are preserved by default.

Built on safe, modern Rust (2024 edition) with `zbus` for type-safe D-Bus IPC and `tokio` for async I/O, Nexus targets the workflow of users who live in the terminal: Arch, NixOS, Void, any distribution running NetworkManager.

---

## Features

- **Direct D-Bus IPC** — communicates with `org.freedesktop.NetworkManager` via `zbus`. Zero subprocess spawning; no shell, no `nmcli`, no pipes.
- **Async, event-driven core** — `tokio` multi-threaded runtime with a unified `mpsc` event channel multiplexing input, ticks, D-Bus signals, and network results.
- **Full WiFi lifecycle** — scan, connect (WPA/WPA2/WPA3/WEP/Open), disconnect, forget saved profiles, hidden network SSID entry.
- **Animated UI** — smooth signal-strength interpolation (exponential ease-out), braille spinners, dialog slide-in transitions, cursor blink. Disable with `animations = false`.
- **Configurable theme engine** — every color is user-defined via TOML. Supports named colors, `"reset"` (terminal default), and `#RRGGBB` true color hex. Ship your Catppuccin, Gruvbox, or Dracula palette.
- **Transparency-native** — all backgrounds default to `Color::Reset`. Terminal blur, opacity, and compositor effects pass through untouched.
- **Nerd Font icons** with automatic plain-Unicode fallback (`--no-nerd-fonts`).
- **Vim-native navigation** — `j`/`k`/`g`/`G` alongside arrow keys. Designed for `hjkl` muscle memory.
- **Embedded config bootloader** — `default_config.toml` is baked into the binary via `include_str!`. First launch writes `~/.config/nexus/config.toml` automatically. Delete it to regenerate. The binary can never fail to start due to a missing config.
- **CLI override layer** — any config value can be overridden per-invocation via flags (`--interface`, `--fps`, `--no-nerd-fonts`, etc.).
- **Detail panel** — IP, gateway, DNS, MAC, BSSID, speed, frequency band, channel, signal strength for the selected network.
- **Zero `unsafe`** — pure safe Rust throughout.

---

## Requirements

| Dependency | Purpose | Install |
|---|---|---|
| **NetworkManager** | WiFi backend (D-Bus API) | `sudo pacman -S networkmanager` |
| **D-Bus** (system bus) | IPC transport | Included in `dbus` / `systemd` |
| **Rust ≥ 1.85** | Build toolchain (2024 edition) | [rustup.rs](https://rustup.rs/) |
| **A Nerd Font** *(optional)* | Icon glyphs | [nerdfonts.com](https://www.nerdfonts.com/) |

> [!NOTE]
> Nexus is developed and tested on **Arch Linux** with Hyprland (Wayland) and Kitty. It runs on any compositor or X11 session — the only hard requirement is a running NetworkManager instance on the system D-Bus. If your terminal supports true color and a Nerd Font, you get the full experience.

---

## Installation

### Build from source

```bash
git clone https://github.com/CPT-Dawn/Nexus.git
cd Nexus
cargo build --release
sudo install -Dm755 target/release/nexus /usr/local/bin/nexus
```

The release profile is tuned for a minimal binary: `opt-level = 3`, full LTO, symbol stripping, single codegen unit.

### Arch Linux (AUR)

```bash
yay -S nexus-wifi
```

> *AUR package coming soon — this section will be updated with the canonical package name.*

### Uninstall

```bash
sudo rm /usr/local/bin/nexus
rm -rf ~/.config/nexus ~/.local/share/nexus
```

---

## Configuration

Nexus uses an **embedded asset bootloader** pattern. The full default configuration is compiled into the binary at build time via `include_str!("../default_config.toml")`. On first launch, it is written to disk:

```
~/.config/nexus/config.toml
```

If the file is deleted or corrupted, Nexus regenerates it from the embedded copy on next startup. You can also dump the defaults at any time:

```bash
nexus --print-default-config > ~/.config/nexus/config.toml
```

### Structure

The config file is organized into five sections:

```toml
[general]
interface = ""              # WiFi interface. Empty = auto-detect.
log_level = "info"          # trace | debug | info | warn | error
scan_interval_secs = 5      # D-Bus poll interval (seconds)

[appearance]
nerd_fonts = true            # false → plain Unicode fallback
animations = true            # false → instant updates, no easing
fps = 30                     # Render loop target (15–60)
show_details = true          # Detail panel visible on launch
border_style = "rounded"     # rounded | plain | thick | double

[theme]
bg = "reset"                 # "reset" = terminal default (transparent)
fg = "white"
fg_dim = "darkgray"
accent = "cyan"              # Selected items, active borders
accent_secondary = "magenta" # Section headers in detail panel
border = "darkgray"
border_focused = "cyan"

[theme.semantic]
connected = "green"
warning = "yellow"
error = "red"
selected_bg = "darkgray"

[theme.signal]
excellent = "green"          # 80–100%
good = "green"               # 60–79%
fair = "yellow"              # 40–59%
weak = "red"                 # 20–39%
none = "darkgray"            #  0–19%

[keys]
scan = "s"
connect = "enter"
disconnect = "d"
forget = "f"
hidden = "h"
details = "i"
refresh = "r"
help = "/"
quit = "q"
```

**Color values** accept named colors (`"red"`, `"cyan"`, `"darkgray"`, …), `"reset"` / `"transparent"` for terminal default, or `"#RRGGBB"` hex for true color.

---

## Usage

```bash
nexus                              # Auto-detect interface, default config
nexus --interface wlan0            # Use a specific WiFi interface
nexus --no-nerd-fonts              # Plain Unicode (no Nerd Font required)
nexus --fps 60                     # Smoother rendering
nexus --log-level debug            # Verbose file logging
nexus --config /path/to/custom.toml
nexus --print-default-config       # Dump embedded defaults to stdout
```

### Keybindings

| Key | Action |
|---|---|
| `↑` / `k` | Move up |
| `↓` / `j` | Move down |
| `g` | Jump to top |
| `G` | Jump to bottom |
| `Enter` | Connect to selected network |
| `d` | Disconnect |
| `s` | Scan for networks |
| `f` | Forget saved network |
| `h` | Connect to hidden network |
| `i` | Toggle detail panel |
| `r` | Refresh connection info |
| `Ctrl+H` | Show/hide password |
| `Tab` | Switch fields (in dialogs) |
| `Esc` | Close dialog / cancel |
| `/` or `?` | Toggle help overlay |
| `q` | Quit |

Logs are written to `~/.local/share/nexus/nexus.log` (daily rotation).

---

## Architecture

```
src/
├── main.rs                # Entry point, terminal setup, event loop
├── app.rs                 # State machine, key dispatch, async action dispatch
├── config.rs              # TOML parsing, CLI args (clap), embedded bootloader
├── event.rs               # Async event multiplexer (input, ticks, network)
├── animation/
│   ├── mod.rs             # AnimationState, tick driver
│   ├── spinner.rs         # Braille, bar, and pulse frame generators
│   └── transitions.rs     # Signal smoothing (exponential ease-out), fade-in
├── network/
│   ├── mod.rs             # NetworkBackend trait (swap NM / iwd / mock)
│   ├── manager.rs         # NmBackend — full D-Bus implementation via zbus
│   ├── signals.rs         # Polling-based NM state change listener
│   └── types.rs           # WiFiNetwork, ConnectionInfo, SecurityType, enums
└── ui/
    ├── mod.rs             # Root layout, modal overlay dispatch
    ├── theme.rs           # Runtime Theme struct, style constructors, icons
    ├── header.rs          # Title bar + live connection status
    ├── network_list.rs    # Scrollable WiFi list with signal bars
    ├── details.rs         # Right-side detail panel (IP, MAC, channel, …)
    ├── password.rs        # Password input modal
    ├── hidden.rs          # Hidden network SSID + password modal
    ├── help.rs            # Keybinding overlay
    └── status_bar.rs      # Context-sensitive footer hints
```

### Design decisions

- **`NetworkBackend` trait** abstracts the WiFi backend behind an async interface. The current implementation targets NetworkManager; an `iwd` or mock backend can be dropped in without touching any UI code.
- **Single-channel event architecture** — `tokio::spawn` tasks for input polling, tick generation, and D-Bus calls all feed one `mpsc::UnboundedSender<Event>`, consumed by the main loop. No shared mutable state, no locks.
- **Config layering** — embedded TOML defaults → user config file → CLI flags. Each layer overrides the previous. The binary is always self-contained.
- **`Color::Reset` by default** — every background uses the terminal's native color, preserving transparency, blur, and whatever your compositor provides.

---

## Troubleshooting

**"NetworkManager is not running"**
```bash
sudo systemctl start NetworkManager
sudo systemctl enable NetworkManager
```

**No WiFi adapter detected**
```bash
ip link                     # Verify your interface is visible
rfkill list                 # Check for soft/hard blocks
sudo rfkill unblock wifi
```

**Icons look broken**
Install a [Nerd Font](https://www.nerdfonts.com/) and set it as your terminal font, or launch with `--no-nerd-fonts`.

**Config parse error on startup**
Delete the config file to regenerate defaults:
```bash
rm ~/.config/nexus/config.toml
nexus
```

---

## Contributing

Contributions are welcome. Fork the repository, create a feature branch, and open a pull request.

```bash
cargo check      # Type-check
cargo clippy      # Lint
cargo build       # Debug build
cargo test        # Run tests (if any)
```

Please ensure `cargo check` and `cargo clippy` pass with zero warnings before submitting.

---

## License

[MIT](LICENSE) © Swastik Patel