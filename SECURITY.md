# Security

Nexus handles Wi-Fi and VPN secrets and talks to NetworkManager for your user, so security reports matter.

## Supported versions

Only the latest 2.x release gets fixes.

## Reporting a vulnerability

Email **ping@cptdawn.dev** with "Nexus security" in the subject. Please don't open a public issue. Include the version (`nexus --version`), what an attacker can do, and the steps to reproduce.

You'll get an answer within a week. Once a fix ships, the release notes credit you unless you'd rather stay anonymous.

## What Nexus promises

- The Daemon never runs as root; NetworkManager's polkit rules decide what it may do ([docs/polkit.md](docs/polkit.md)).
- Its socket is created mode `0600` in a `0700` directory under `$XDG_RUNTIME_DIR`, and it refuses connections from other users.
- Secrets never travel in socket events and are never logged.
- No telemetry and no update checks.
