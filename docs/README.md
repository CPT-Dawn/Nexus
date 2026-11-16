# Documentation

Reference for using, scripting and building Nexus. Start with the [README](../README.md) for what Nexus is and how to install it.

## Using Nexus

| Guide | What it covers |
| --- | --- |
| [Installation](installation.md) | AUR, from source, the systemd units, uninstalling |
| [CLI and scripting](cli.md) | Every command, `--json`, Waybar, the launcher menu |
| [Keybindings](keybindings.md) | Every key in the TUI, tab by tab |
| [Configuration](configuration.md) | Every `config.toml` option, its default, and themes |
| [Permissions](polkit.md) | What polkit allows, and the optional rule |
| [Troubleshooting](troubleshooting.md) | The errors people hit, each with its fix |

## Building on Nexus

| Guide | What it covers |
| --- | --- |
| [Socket API](ipc.md) | The JSON-lines protocol, every op and event, and the exit codes |
| [`nexus-ipc`](../crates/nexus-ipc/README.md) | The Rust crate with the protocol types and a client |

## Working on Nexus

| Guide | What it covers |
| --- | --- |
| [Contributing](../CONTRIBUTING.md) | Setup, the checks CI runs, the demo world, the release |
| [Testing](testing.md) | The test seams: fake NetworkManager, TUI on a pty, real hardware, budgets |
| [Glossary](../GLOSSARY.md) | The words the code and the UI use, and the ones they avoid |
| [Decisions](adr/) | The architecture decision records |
| [Research](research/) | The landscape, crate and API research behind v2 |

[Back to the README](../README.md)
