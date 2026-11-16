# Changelog

Notable changes to Nexus. Versions follow [Semantic Versioning](https://semver.org/); the socket API's compatibility rules are in [docs/ipc.md](docs/ipc.md#compatibility).

## [Unreleased]

- Docs: a new README, the reference moved into [docs/](docs/README.md), a recorded demo and a logo.

## [2.0.0] - 2026-11-10

A from-scratch rewrite. Nexus is now an unprivileged user Daemon over NetworkManager with three front ends: the TUI, the CLI and a launcher menu.

- Wi-Fi: WPA2/WPA3, OWE, hidden Networks, Enterprise (802.1X) sign-in, QR sharing, access point details, several cards.
- Ethernet with 802.1X, told apart as Unplugged or Misconfigured.
- VPNs: WireGuard import, creation and QR sharing; plugin VPNs such as OpenVPN, with their prompts.
- Hotspot with a generated password and QR code; Bluetooth tethering.
- Profile edits (IP, DNS, MAC, metered, priority) under a NetworkManager checkpoint that rolls back unless you keep it; passwords optionally kept in your keyring.
- Trust: Untrusted Networks can bring up a VPN and go metered on join.
- Captive Portal detection and login, desktop notifications, and `nexus doctor`.
- `nexus status --waybar`, `nexus menu`, `--json` everywhere, one exit code per failure, and a documented JSON-lines socket API frozen at version 1.
- Seven themes and custom ones, event-driven effects with no CPU at idle, and fallbacks down to 16 colours and ASCII.

## [1.0.1] - 2026-03-22

- The log moved from `XDG_DATA_HOME` to `XDG_STATE_HOME` ([#1](https://github.com/CPT-Dawn/Nexus/pull/1)).

## [1.0.0] - 2026-03-18

- First release.

[Unreleased]: https://github.com/CPT-Dawn/Nexus/compare/v2.0.0...HEAD
[2.0.0]: https://github.com/CPT-Dawn/Nexus/compare/v1.0.1...v2.0.0
[1.0.1]: https://github.com/CPT-Dawn/Nexus/compare/v1.0.0...v1.0.1
[1.0.0]: https://github.com/CPT-Dawn/Nexus/releases/tag/v1.0.0
