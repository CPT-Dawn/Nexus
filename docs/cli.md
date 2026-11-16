# CLI and scripting

Every `nexus` command, and how to drive Nexus from scripts, Waybar and your launcher. The commands are defined in [`crates/nexus/src/main.rs`](../crates/nexus/src/main.rs); `nexus <command> --help` and `man nexus-<command>` give every flag.

`nexus` with no command opens the TUI.

## Commands

| Command | What it does |
| --- | --- |
| `nexus status` | The overall network state and the primary Link |
| `nexus menu` | Pick a Network or VPN in your launcher (fuzzel, walker, rofi, tofi) |
| `nexus wifi` | `list`, `scan`, `join`, `aps`, `on`, `off`, `disconnect`, `forget`, `autoconnect`, `share` |
| `nexus vpn` | `list`, `up`, `down`, `import` (`.conf`, `.ovpn`, other provider files), `create`, `key`, `share` |
| `nexus ethernet` | `list`, `connect` (`--eap` for an 802.1X port), `disconnect` |
| `nexus tether` | `list`, `connect`, `disconnect`: get online through a paired phone over Bluetooth |
| `nexus hotspot` | `status`, `start`, `stop`, `qr` |
| `nexus profile` | `list`, `show`, `set`: IP, DNS, MAC, metered, priority, keyring |
| `nexus trust` | `list`, `set`: mark Networks Trusted or Untrusted |
| `nexus portal` | `status`, `open`: the Network's login page |
| `nexus activity` | What the Daemon did lately: joins, drops, VPNs, Portals |
| `nexus doctor` | Why the machine is or is not online, and how to fix it (`--fix` offers to run the fixes) |
| `nexus airplane` | `on` turns every radio off, `off` turns them back on |
| `nexus config` | `get` and `set` the config file ([Configuration](configuration.md)) |
| `nexus daemon` | Run the Daemon in the foreground (systemd socket activation uses this) |
| `nexus completions` | Print shell completions for bash, zsh or fish |
| `nexus man` | Print the man page, or write one page per command into a directory |

Global flags: `--ascii` draws with plain ASCII, `--nerd-font` uses Nerd Font icons.

## A few examples

```sh
nexus wifi join "Home-5G"                       # asks for the password if it needs one
nexus vpn import ~/Downloads/work.conf
nexus profile set "Home-5G" --dns 1.1.1.1,9.9.9.9
nexus hotspot start                              # makes up a name and a strong password
nexus doctor --fix
```

`profile set` on the network you're on applies the change and waits 15 seconds for you to keep it, then rolls back. `--keep` keeps it at once, for scripts.

## JSON and exit codes

Every read command takes `--json`. Every failure has its own exit code, from 1 to 36 (3 means the Daemon cannot be reached); the full table is in [the socket API docs](ipc.md#exit-codes) and `man nexus`.

```sh
nexus status --json | jq -r .state
```

## Waybar

`nexus status --waybar` prints a Waybar custom-module object (`text`, `tooltip`, `class`, `alt`). With `--follow` it keeps running and prints a line whenever the state changes, so the bar updates without polling:

```jsonc
"custom/nexus": {
  "exec": "nexus status --waybar --follow",
  "return-type": "json",
  "on-click": "nexus menu"
}
```

`class` holds the state, the Link's kind and extras such as `vpn` or `hotspot` (for example `["online", "wifi", "vpn"]`) for your CSS. Set `waybar.icons = true` for a glyph in front of the text.

## Launcher menu

`nexus menu` lists nearby Networks and your VPNs in one launcher list and acts on what you pick. A password is asked through the launcher's hidden-input mode. It finds fuzzel, walker, rofi or tofi by itself; set `menu.launcher` to choose one.

## The socket

Anything the CLI does not cover, a program can do over the Daemon's socket at `$XDG_RUNTIME_DIR/nexus/nexus.sock`. [docs/ipc.md](ipc.md) documents the JSON-lines protocol with `socat` and `jq` examples, and [`nexus-ipc`](../crates/nexus-ipc/README.md) is the Rust crate for writing your own front end.

[Back to the README](../README.md)
