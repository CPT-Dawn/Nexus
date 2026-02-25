<p align="center">
  <h1 align="center">󰤨 Nexus</h1>
  <p align="center">
    <em>A beautiful, modern TUI WiFi manager for Linux</em>
  </p>
  <p align="center">
    <a href="https://github.com/CPT-Dawn/Nexus/blob/main/LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License: MIT"></a>
    <a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/rust-2024_edition-orange.svg" alt="Rust"></a>
    <img src="https://img.shields.io/badge/platform-Linux-lightgrey.svg" alt="Platform: Linux">
    <img src="https://img.shields.io/badge/backend-NetworkManager-green.svg" alt="Backend: NetworkManager">
  </p>
</p>

---

Nexus is a fast, keyboard-driven WiFi manager that runs entirely in your terminal. Built in Rust with `ratatui`, it communicates directly with NetworkManager over D-Bus — no shell commands, no `nmcli` wrappers. It respects your terminal's transparency and color scheme out of the box.

<!-- Screenshot placeholder — replace the path below with your actual screenshot -->
<p align="center">
  <img src="assets/screenshot.png" alt="Nexus TUI Screenshot" width="800">
</p>

## Features

- **Scan & discover** WiFi networks with real-time signal strength
- **Connect / Disconnect** with password input (WPA/WPA2/WPA3/WEP/Open)
- **Forget** saved network profiles
- **Hidden network** support — connect by entering SSID manually
- **Detail panel** — IP, gateway, MAC, speed, frequency, channel, signal
- **Animated UI** — smooth signal bar interpolation, braille spinners, dialog slide-ins, cursor blink
- **Nerd Font icons** with `--no-nerd-fonts` plain-unicode fallback
- **Transparency-friendly** — uses `Color::Reset` backgrounds so your terminal blur/opacity shines through
- **Vim keybindings** (`j`/`k`/`g`/`G`) alongside arrow keys
- **Pure D-Bus** — talks to NetworkManager directly via `zbus`, zero subprocess spawning
- **Auto-detect** WiFi interface, or override with `--interface`
- **30 FPS** render loop with adaptive tick rate

## Requirements

| Dependency | Purpose |
|---|---|
| **NetworkManager** | WiFi backend (D-Bus API) |
| **D-Bus** (system bus) | IPC transport |
| **Rust 2024 edition** | Build toolchain (1.85+) |
| **A Nerd Font** *(optional)* | For icon glyphs; use `--no-nerd-fonts` without one |

> Nexus is developed and tested on **Arch Linux**. It should work on any distro that runs NetworkManager.

## Installation

### From source

```bash
git clone https://github.com/CPT-Dawn/Nexus.git
cd Nexus
cargo build --release
sudo install -Dm 755 target/release/nexus /usr/local/bin/nexus
```

### AUR (Arch Linux)

<!-- TODO: update with actual AUR package name once published -->
```bash
yay -S nexus-wifi
```

> *AUR package coming soon — this section will be updated.*

### Uninstall

```bash
sudo rm /usr/local/bin/nexus
```

## Usage

```bash
nexus                          # auto-detect WiFi interface
nexus --interface wlan0        # use a specific interface
nexus --no-nerd-fonts          # plain Unicode (no Nerd Font required)
nexus --log-level debug        # verbose file logging
```

Logs are written to `~/.local/share/nexus/nexus.log`.

## Keybindings

| Key | Action |
|---|---|
| `↑` / `k` | Move up |
| `↓` / `j` | Move down |
| `g` | Go to top |
| `G` | Go to bottom |
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

## Architecture

```
src/
├── main.rs               # Entry point, event loop, terminal setup
├── app.rs                # State machine, key dispatch, actions
├── config.rs             # CLI args (clap)
├── event.rs              # Async event multiplexer (input, tick, network)
├── animation/
│   ├── mod.rs            # AnimationState, tick driver
│   ├── spinner.rs        # Braille / bar / pulse frame generators
│   └── transitions.rs    # Signal smoothing, fade-in opacity
├── network/
│   ├── mod.rs            # NetworkBackend trait
│   ├── manager.rs        # NmBackend — full D-Bus implementation
│   ├── signals.rs        # Polling-based NM change listener
│   └── types.rs          # WiFiNetwork, ConnectionInfo, SecurityType
└── ui/
    ├── mod.rs            # Root layout, modal overlay dispatch
    ├── theme.rs          # Colors, icons, style constructors
    ├── header.rs         # Title bar + connection status
    ├── network_list.rs   # Scrollable WiFi list with signal bars
    ├── details.rs        # Right-side detail panel
    ├── password.rs       # Password input modal
    ├── hidden.rs         # Hidden network SSID+password modal
    ├── help.rs           # Keybinding overlay
    └── status_bar.rs     # Context-sensitive footer hints
```

**Key design decisions:**

- **`NetworkBackend` trait** abstracts the WiFi backend — swap in `iwd` or a mock without touching UI code.
- **Event-driven** — `tokio::spawn` tasks for input polling, tick generation, and D-Bus operations feed a single `mpsc` channel consumed by the main loop.
- **`Color::Reset` everywhere** — no hardcoded background colors, so terminal transparency and themes are preserved.
- **Zero `unsafe`** — pure safe Rust throughout.

## CLI Options

```
nexus — A beautiful modern TUI WiFi manager

Usage: nexus [OPTIONS]

Options:
  -i, --interface <INTERFACE>    WiFi interface to use (default: auto-detect)
  -l, --log-level <LOG_LEVEL>    Log level filter [default: info]
      --no-nerd-fonts            Disable Nerd Font icons, use plain Unicode
  -h, --help                     Print help
  -V, --version                  Print version
```

## Troubleshooting

**"NetworkManager is not running"**
```bash
sudo systemctl start NetworkManager
sudo systemctl enable NetworkManager   # persist across reboots
```

**No WiFi adapter detected**
```bash
ip link                    # verify your interface is visible
rfkill list                # check for soft/hard blocks
sudo rfkill unblock wifi
```

**Icons look broken**
Install a [Nerd Font](https://www.nerdfonts.com/) and set it as your terminal font, or run with `--no-nerd-fonts`.

## License

[MIT](LICENSE) © Swastik Patel