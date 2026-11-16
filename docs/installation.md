# Installation

How to install Nexus on Arch Linux from the AUR or on any distribution from source, and how its systemd units start the Daemon. The packaging lives in [`packaging/`](../packaging).

## Requirements

| Needs | Why |
| --- | --- |
| NetworkManager | Nexus drives it; it does no networking of its own. iwd can stay as its Wi-Fi backend |
| systemd (user session) | Starts the Daemon on demand through `nexus.socket` |
| A desktop session (Wayland) | Desktop notifications, the launcher menu, opening login pages |

Optional, each switched on when present:

| Package | Adds |
| --- | --- |
| `polkit` | Authorizes Wi-Fi and Profile changes ([Permissions](polkit.md)) |
| `iw` | Counts the devices joined to your Hotspot |
| `xdg-utils` | Opens a Portal login page in your browser |
| `xdg-terminal-exec` | Opens a terminal from notification actions (else `$TERMINAL`) |

## Arch Linux (AUR)

```sh
paru -S nexus-nm-bin   # prebuilt, x86_64 and aarch64
paru -S nexus-nm       # built from source
```

Both install the `nexus` binary, the systemd user units, shell completions for bash, zsh and fish, and the man pages. `nexus.socket` is enabled for every user, so the Daemon starts on demand from your next login. To start it now:

```sh
systemctl --user enable --now nexus.socket
nexus status
```

## From source

Needs Rust 1.90 or newer.

1. Build and install the binary:

   ```sh
   cargo install --locked --git https://github.com/CPT-Dawn/Nexus nexus
   ```

2. Install the systemd units, pointed at that binary. From a clone of the repo:

   ```sh
   mkdir -p ~/.config/systemd/user
   cp packaging/systemd/nexus.socket ~/.config/systemd/user/
   sed "s|/usr/bin/nexus|$(command -v nexus)|" packaging/systemd/nexus.service \
     > ~/.config/systemd/user/nexus.service
   systemctl --user daemon-reload
   systemctl --user enable --now nexus.socket
   ```

3. Optionally, the man pages and completions (zsh shown):

   ```sh
   mkdir -p ~/.local/share/man/man1 ~/.local/share/zsh/site-functions
   nexus man --dir ~/.local/share/man/man1
   nexus completions zsh > ~/.local/share/zsh/site-functions/_nexus
   ```

## Uninstalling

```sh
systemctl --user disable --now nexus.socket nexus.service
paru -R nexus-nm-bin        # or nexus-nm; for a source install: cargo uninstall nexus
```

Nexus keeps its config in `~/.config/nexus/` and its state (Trust marks) in `~/.local/state/nexus/`. Delete them to remove every trace. Your NetworkManager Profiles stay, since they belong to NetworkManager.

[Back to the README](../README.md)
