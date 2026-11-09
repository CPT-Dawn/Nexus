# Daemon API: versioned JSON-lines over a Unix socket, in one binary

Every Front end (the TUI, the CLI, `nexus menu`, `nexus status --waybar`, the Zenith `nexus` module) talks to the Daemon through one versioned JSON-lines protocol on `$XDG_RUNTIME_DIR/nexus/nexus.sock`: request/response plus a `subscribe` event stream, documented in `docs/ipc.md` as a public contract. This mirrors Hoarder (Hoarder ADR-0004) so the CPT Dawn tools share one integration pattern and Zenith consumes Nexus exactly as it consumes Hoarder. A session D-Bus service was the obvious alternative; it was rejected for heavier tooling and because a socket is scriptable with `socat`/`jq`. Event rates are low, so JSON's cost is irrelevant.

## Consequences

- The protocol types live in a `nexus-ipc` crate, consumed by git pinned to a release tag (not crates.io). Unknown fields are ignored and new enum values decode as `Unknown`, so additive changes keep `v: 1`; breaking changes bump it.
- The Daemon is `nexus daemon`, a subcommand of the single `nexus` binary, started by systemd user socket activation, so a Front end never needs to start it.
- The socket carries no secrets in events; answers to Secret prompts are the only secret-bearing requests and are never logged.
