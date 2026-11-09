# Errors are typed Failures with human text, not `thiserror`/`color-eyre`

The spec planned `thiserror` for library-style modules and `color-eyre` at the binary edge. Nexus ships neither. Every error a person can see already has a fixed shape: a public `ErrorCode` from `nexus-ipc` (frozen with the socket API, ADR-0006) plus one sentence from the text catalog (`text::Failure { code, message }`). The CLI turns the code into its exit status through the single exit-code table in `cli.rs`. An error type per module would only be converted straight back into that pair, and an `eyre` report with a backtrace is the opposite of the plain-language failures the spec asks for.

## Consequences

- Backends return `Result<_, Failure>` (or an `ErrorCode`), never `Box<dyn Error>` across a module boundary. Internal plumbing (`zbus::Error`, `io::Error`) is mapped to a `Failure` at the Backend edge, where the reason is still known.
- Each new failure adds an `ErrorCode` (if it is a new class), a sentence in `text.rs` and, for the CLI, an exit-code row: three places a reviewer can check, all tested for drift.
- No `thiserror`/`color-eyre`/`anyhow` dependencies, which also keeps the binary inside the S3 size and start-up budgets.
- Panics stay bugs, not error handling: `expect` is only for invariants (wire types always serialize).
