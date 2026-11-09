# The socket API is frozen at v1 for 2.0, and its documentation is executable

ADR-0002 made the JSON-lines socket the public contract. With 2.0 that contract is final: `docs/ipc.md` documents every op, event and error code with `socat`/`jq` examples, `nexus-ipc` carries the compatibility policy in its README, and the Zenith `nexus` module is built against it.

The page is also a test. `crates/nexus/tests/ipc_conformance.rs` sends every request example to a real Daemon on a fake NetworkManager and requires the answer to have the documented shape (same keys, same JSON types, equal error `code`); every event example must decode and match a live event; and `nexus-ipc`'s `contract_tests` fail when an op, event or error code exists that the page does not document, or the page documents one that does not exist. Adding to the protocol therefore means adding its documented example in the same change.

## Consequences

- Additive changes (new op, event, error code, optional field, enum value) keep `v: 1` and are minor releases. Clients must ignore unknown fields and decode unknown events, codes and enum values as `Unknown`. Breaking changes bump `v`.
- Error `code`s and CLI exit codes are public. The exit code table is one list (`EXIT_CODES` in `cli.rs`), printed in the man page and in `docs/ipc.md`, and a test keeps both equal to it. New codes take the next free number; none is reused.
- Examples show the first element of each array and the shapes the Daemon sends, not its values (ids, times and the fake machine's names differ on a real one).
- `nexus-ipc` is consumed by git tag, not crates.io (ADR-0002). Tagging a 2.x release is what "tagging `nexus-ipc` v1" means.
