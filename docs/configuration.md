# Configuration

Every option in `config.toml`, with its default. The file lives at `$XDG_CONFIG_HOME/nexus/config.toml`, else `~/.config/nexus/config.toml`, and is optional: a missing file or key means the default. The options and their checks are defined in [`crates/nexus/src/config.rs`](../crates/nexus/src/config.rs).

The Daemon reloads the file when it changes. A file it cannot read never stops it: it keeps the defaults and says why.

## Reading and changing it

```sh
nexus config get                          # the effective config, as JSON
nexus config get connectivity.url         # one value by dotted key
nexus config set notifications.lost false # edits the file in place, keeping your comments
nexus config set theme null               # null resets a key to its default
```

The TUI's Settings tab (<kbd>6</kbd>) edits the same file.

## Options

| Key | Default | What it does |
| --- | --- | --- |
| `theme` | `"midnight-bloom"` | The TUI theme: a built-in name or one of your `[themes]` |
| `ui.nerd_font` | `false` | Nerd Font icons on the tabs. `--nerd-font` also turns it on |
| `ui.reduce_motion` | `false` | No gliding, sliding, shaking or pulsing. `NEXUS_REDUCE_MOTION=1` also turns it on |
| `connectivity.enabled` | `true` | NetworkManager's connectivity check. `false` switches it off in NetworkManager |
| `connectivity.url` | `"https://ping.archlinux.org/nm-check.txt"` | The address the Doctor and the Portal login use. Must start with `http://` or `https://` |
| `menu.launcher` | unset | The launcher `nexus menu` drives; unset auto-detects fuzzel, walker, rofi or tofi |
| `waybar.icons` | `false` | Put a state glyph in front of the Waybar `text`. `alt` always carries the state |
| `prompts.timeout_secs` | `120` | Seconds a password prompt waits for an answer (1 to 3600) |
| `doctor.gateway` | `"auto"` | The gateway the Doctor tests: `auto`, `off`, or an IP address |
| `hotspot.poll_secs` | `5` | Seconds between counts of devices joined to your Hotspot (1 to 60) |
| `trust.auto_vpn` | `""` | The VPN (name or Profile UUID) to bring up after joining an Untrusted Network. Empty: none |
| `trust.metered` | `false` | Also mark an Untrusted Network's Profile metered |
| `notifications.joined` | `true` | "Joined *network*" |
| `notifications.lost` | `true` | "Lost *network*" |
| `notifications.wrong_password` | `true` | A join failed on a wrong password |
| `notifications.secret_prompt` | `true` | A password prompt opened while no TUI runs, with an action that opens one |
| `notifications.vpn_dropped` | `true` | A VPN went down without being asked to |
| `notifications.portal` | `true` | "Login required for *network*", then "You're online" |
| `notifications.policy` | `true` | A Trust policy brought up a VPN or marked a Network metered |

The address NetworkManager itself probes is set in `NetworkManager.conf`; Nexus only reads it.

## Themes

The built-in themes are `midnight-bloom`, `catppuccin`, `gruvbox`, `nord`, `tokyo-night`, `rose-pine` and `high-contrast`. A test holds each one's text to a contrast of 4.5:1 or better on its background, and the TUI falls back to 256 colours, 16 colours and plain ASCII (`--ascii`) on terminals that need it.

A custom theme starts from a built-in one (`base`, default Midnight Bloom) and overrides any of the tokens `bg`, `fg`, `muted`, `dim`, `accent`, `ok`, `warn`, `err`, `border` and `selection`:

```toml
theme = "sunrise"

[themes.sunrise]
base = "gruvbox"
accent = "#ff9e64"
border = "#5c4a3a"
```

Colours are `#rrggbb`, and `accent` must differ from `border`. A theme with a mistake falls back to Midnight Bloom, and the TUI says which entry is wrong.

[Back to the README](../README.md)
