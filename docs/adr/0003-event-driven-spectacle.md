# Event-driven spectacle: rich motion, zero idle frames

Nexus aims to be a showcase TUI, so it goes further than Zenith's restraint (Zenith ADR-0003): big-type status, glow and gradient surfaces on truecolor terminals, and tachyonfx transitions on tab changes, joins, scans, connecting, errors and toasts. Every effect is triggered by an event; with nothing happening Nexus renders no frames and uses no CPU, and live graphs tick at 1 Hz only while visible. Text cells only: no Kitty graphics or sixel, so Nexus looks the same in every terminal.

## Consequences

- Every effect has a designed fallback across truecolor → 256 → 16 colours → reduced motion → ASCII, and snapshot tests cover each tier.
- The 0% idle CPU budget is enforced in CI, so an effect that keeps a timer alive after it finishes is a bug.

## How it is built (#29)

- Effects are tachyonfx effects run as a post-pass over the frame the views drew (`tui/fx.rs`); views stay pure. The effect slots are `Tab`, `Bloom`, `Pulse` and `Toast`, one effect per slot, so a new event replaces the old one.
- `App::next_tick()` returns the 33 ms frame only while an effect runs. When the last effect ends it returns `None` again and the loop blocks: idle is zero frames. The connecting pulse is the one effect that repeats; it is cancelled the moment the state leaves Connecting.
- Time is passed in (`App::fx_step`, `tui::render_fx`), so tests step a virtual clock; the real loop uses the wall clock, capped at 100 ms a frame.
- Tiers: truecolor and 256 colours play Effects (256 blends in RGB, then snaps every cell to the nearest palette index); 16 colours and none play none, and their state changes are static. Reduced motion keeps the bloom as a plain colour fade and drops the rest.
- Surfaces (`tui/surface.rs`): focused borders run a hue gradient across the screen (truecolor smooth, 256 stepped) and, on truecolor only, the cells beside them take a 10% tint. Both are a post-pass keyed on the focused border token, so views never name a gradient.
- Dialogs and the command palette sit over the detail pane when the screen is split, so they never cover the list.

## Themes (#28)

- A theme is data: ten tokens, each with an RGB value and a 16-colour fallback. Custom themes in `config.toml` (`[themes.<name>]`, `#rrggbb`, optional `base`) get their 16-colour fallbacks from the nearest ANSI colour. The `accent` must differ from the `border`, because the focused border is recognised by the accent on a box glyph; a theme that breaks this is rejected like any other mistake.
- The TUI resolves the theme from the Daemon's config (`config_get` on connect, `config_reloaded` after), so there is no second copy of the setting. An unknown name or a bad custom theme falls back to Midnight Bloom and says why (toast plus the Settings tab), and the Daemon keeps the rest of the file in force.
- A theme change rebuilds the effects (they hold the theme's colours) and restarts the connecting pulse if one was running. Reduced motion is the environment OR `ui.reduce_motion`; either can turn it on, only the config can turn it off, and the Settings tab says when the environment is holding it on.
- The backdrop stays transparent in every theme; an opaque switch (`Look::surface`) is still unused.

