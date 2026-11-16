# Troubleshooting

The problems people hit with Nexus, each with its fix. When the network itself misbehaves, start with `nexus doctor`: it checks NetworkManager, the gateway and DNS, and lists the fixes in order. `nexus doctor --fix` offers to run them, asking first; the privileged ones go through `pkexec`.

## `nexus: cannot reach the daemon` (exit code 3)

The Daemon's socket is not running. Start it:

```sh
systemctl --user enable --now nexus.socket
```

If it still fails, read the Daemon's log with `journalctl --user -u nexus.service`, or run it in the foreground with `nexus daemon` and watch what it prints. `RUST_LOG=debug` makes it chatty.

## NetworkManager is not running

Nexus drives NetworkManager and does no networking of its own. `nexus doctor` says whether NetworkManager is installed, whether another service manages your network (systemd-networkd, dhcpcd, a standalone wpa_supplicant), and the commands to switch, in order. NetworkManager can keep iwd as its Wi-Fi backend.

## A password dialog asks to authorize a change

NetworkManager's polkit rules decide what you may change. A normal graphical login may scan, connect and manage its own connections without a password. A session polkit treats as inactive (SSH, lingering services) and reading a stored password can ask. [Permissions](polkit.md) has an optional rule that lets the `wheel` group skip those prompts.

## The login page of a café or hotel Wi-Fi won't show

A browser with secure DNS (DNS over HTTPS) skips the network's own resolver, which is how a Portal steers you to its page. Turn secure DNS off until you have signed in, or press <kbd>o</kbd> (`nexus portal open`): Nexus opens a plain `http://` page the Portal can catch.

## A VPN says its plugin is missing (exit code 24)

Plugin VPNs such as OpenVPN need NetworkManager's plugin package, and the error names it. On Arch, OpenVPN is `networkmanager-openvpn`. WireGuard needs no plugin.

## "Keep in keyring" fails (exit code 25)

No Secret Service answered, or it is locked. Start your keyring (gnome-keyring, KWallet, KeePassXC with Secret Service on) and try again. Nothing changes when this fails: the password stays in NetworkManager's store.

## The Hotspot is unavailable (exit code 26)

Your Wi-Fi card's driver does not offer access point mode, so it cannot host a Hotspot. On a card with one radio, the Wi-Fi Link usually drops while a Hotspot runs.

## Wi-Fi won't turn on (exit code 12)

A hardware switch or key holds the radio off, and software cannot undo it. Flip the switch or press the laptop's wireless key.

## Icons show as boxes, or colours look flat

- Boxes: `--nerd-font` and `ui.nerd_font` need a Nerd Font patched font in your terminal. Turn them off, or pass `--ascii` for plain ASCII.
- Flat colours: Nexus uses 24-bit colour when the terminal sets `COLORTERM=truecolor`, and falls back to 256 or 16 colours otherwise. `NO_COLOR` turns colour off.

## My config is ignored

A config file with a mistake is ignored as a whole and the Daemon keeps the defaults. The TUI's Settings tab (<kbd>6</kbd>) shows the reason, with the line number. [Configuration](configuration.md) lists every key and its allowed values.

## Still stuck?

[Open an issue](https://github.com/CPT-Dawn/Nexus/issues/new/choose) with `nexus --version`, the output of `nexus doctor`, and the Daemon's log.

[Back to the README](../README.md)
