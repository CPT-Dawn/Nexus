# Event-driven spectacle: rich motion, zero idle frames

Nexus aims to be a showcase TUI, so it goes further than Zenith's restraint (Zenith ADR-0003): big-type status, glow and gradient surfaces on truecolor terminals, and tachyonfx transitions on tab changes, joins, scans, connecting, errors and toasts. Every effect is triggered by an event; with nothing happening Nexus renders no frames and uses no CPU, and live graphs tick at 1 Hz only while visible. Text cells only: no Kitty graphics or sixel, so Nexus looks the same in every terminal.

## Consequences

- Every effect has a designed fallback across truecolor → 256 → 16 colours → reduced motion → ASCII, and snapshot tests cover each tier.
- The 0% idle CPU budget is enforced in CI, so an effect that keeps a timer alive after it finishes is a bug.
