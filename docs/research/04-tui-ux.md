# TUI UX patterns for Nexus v2 (network manager)

Date: 2026-11-09. Evidence labels: [V] fetched this session; [K] from my prior knowledge of the app (not re-verified, treat as likely-but-check).

## Sources fetched [V]
- impala (iwd Wi-Fi TUI): https://github.com/pythops/impala
- bluetui: https://github.com/pythops/bluetui
- tachyonfx: https://github.com/ratatui/tachyonfx
- NO_COLOR: https://no-color.org/

Not verified this session: lazygit, btop, yazi, gitui, helix, posting, superfile, k9s, atuin, ratatui showcase winners (no 2024-26 winner list was checked). Claims about them are [K].

## Principles (opinionated)

1. **Contextual footer + `?` overlay; never make users memorize.** lazygit shows per-panel key hints in the bottom bar and `?` opens a filterable keybinding menu [K]; helix shows a which-key style popup after a prefix key (space, g) [K]; k9s shows a header key legend [K]. impala/bluetui have only single letters and little on-screen guidance [V], which is the gap to beat. Nexus: footer = 4-6 keys valid for the focused item right now; `?` = full searchable list; `:`/Ctrl-P = command palette that lists every action with its key (teaches shortcuts, serves novices who search by words like "forget network").
2. **Verbs in words, keys as accelerators.** Every action exists in the palette with a plain-English name ("Forget network", "Turn Wi-Fi off"); keys are secondary. Novices never need to know `d` = remove.
3. **Vim + arrows + mouse all work, no modes to learn.** j/k and arrows, g/G, `/` filter, Enter act, Esc back. yazi/lazygit/k9s all mix these [K]. Esc must never quit (impala and bluetui both ship Esc-quit disabled by default [V]); `q` quits from root only.
4. **Single focus list + detail, not a wall of panes.** lazygit/gitui multi-pane suits a dense domain [K]; a network manager has few objects. Prefer a master/detail split (list left, detail right), tabs for domains (Overview/Wi-Fi/VPN/...), and collapse to single column under ~100 cols (detail becomes a push-in screen; Esc pops). Define breakpoints: >=120 two-pane + sparkline, 80-119 list+compact detail, <80 single column, <60 list only with a "terminal too small" hint instead of corrupt layout.
5. **Show state as a sentence first, data second.** Top status line: "Connected to Home-5G, good signal, 312 Mbps down" before raw tables. Use plain words ("Weak signal") plus bars; avoid dBm/BSSID/ifname as the primary label (put in details/"advanced" toggle). impala exposes device info behind `i` [V]: good pattern, keep it.
6. **Safe by default: confirm destructive, undo reversible.** Forget network / disconnect SSH-critical link / delete VPN = confirm dialog naming the object with default focus on Cancel. Toggle actions (Wi-Fi power, autoconnect) are instant with a toast + `u` undo for ~5 s. Warn when an action will drop the user's current connection (e.g. switching the link carrying the session). Detect VPN kill-switch/DNS side effects in the dialog text.
7. **Forms: one field per line, inline validation on blur, secrets masked with a reveal toggle.** Password prompt appears as a modal over the list, shows security type, "Show" toggle (Ctrl-R or Tab to checkbox), validates length (WPA2 8-63) live, and keeps the form open on auth failure with the reason. Hidden-network connect (impala `n` [V]) and Enterprise 802.1X (impala supports [V]) need multi-field forms with sensible defaults and "Advanced" collapsed. Paste must work (bracketed paste).
8. **Errors: human sentence, cause, next action, details on demand.** "Couldn't join Home-5G: wrong password. [Retry] [Edit password]  (d for technical details)". Persistent errors live in a status area; transient ones as auto-dismissing toasts; raw backend errors go to a log pane/`~/.local/state` log, not the main view. Use semantic colors (impala's theme exposes info/warning/error [V]) but never color alone: add a glyph/word.
9. **Live data with restraint.** Signal bars (4-level glyphs ▂▄▆█) per AP, a sparkline for throughput on the active link (btop/bottom style graphs [K]), latency to gateway. Update only the cell that changed. Scan results sorted stably (do not reorder rows under the cursor; keep selection pinned by BSSID/SSID, re-sort only on explicit refresh or when idle).
10. **Render on change, not on a fixed FPS.** Event-driven loop: redraw on input, D-Bus signal, resize, or when an animation/sparkline tick is due (1 Hz for stats; 30 fps only while an animation runs, then back to idle). Saves battery and avoids flicker; ratatui diffs buffers already, so cost is event rate, not frame rate [K].
11. **Animation: tiny and optional.** tachyonfx offers 50+ effects, composition, and cell targeting [V] but its README gives no restraint guidance [V], so set your own: only (a) a short fade/slide for modal open/close (<=150 ms), (b) a spinner on pending connects, (c) a brief highlight flash on state change. No looping decoration. Disable all under `NO_MOTION`/config `reduced_motion = true` or when `TERM`/SSH is slow.
12. **Color: semantic tokens, graceful degradation.** Define ~10 tokens (fg, dim, accent, ok, warn, err, border, selection, bg). Truecolor when `COLORTERM=truecolor|24bit`, else map to 256 then 16 ANSI (use ANSI palette so the user's terminal theme drives look). Default background transparent/`Reset` so terminal theme and Wayland blur show through. NO_COLOR: when set non-empty, no color is added, but bold/underline/reverse remain legal [V]; keep selection via reverse video so UI still works. Light/dark: query OSC 11 with a short timeout, fall back to dark, allow `theme = "auto|dark|light"` [K: OSC 11 behavior; verify with a terminal matrix incl. foot, kitty, alacritty, wezterm, ghostty].
13. **Icons optional with real fallbacks.** impala and bluetui both ship `--ascii` / config switch for terminals without Nerd Fonts [V]. Do better: default to plain Unicode (box drawing, ▂▄▆█, ●○) which works everywhere, make Nerd Font glyphs an opt-in or auto-hinted setting, and test every screen in ASCII mode.
14. **Mouse is a bonus, not a requirement.** Click to focus/select, wheel to scroll, click footer hints and tabs, double-click = primary action. Keep text selection possible (hold Shift in most terminals; document it).
15. **First run in 10 seconds.** If no backend (NetworkManager/iwd) found, show a diagnosis screen with the exact command to fix (not a stack trace). On first launch show a dismissible one-line hint bar: "Press ? for keys, : for commands". No tutorial modal. impala requires disabling NetworkManager/wpa_supplicant to avoid conflicts [V]; Nexus should detect and explain such conflicts rather than silently misbehave.
16. **Accessibility, honest about limits.** Screen readers read the terminal cell grid poorly; mitigate with: a linear `--plain`/`--once` CLI (`nexus status`, `nexus connect <ssid>`) for AT users and scripts, high-contrast theme, no meaning by color alone, no reliance on animation, stable focus (cursor moved to the selected row for tools that track the hardware cursor). Mark as design intent; no screen-reader testing performed.
17. **Everything configurable, nothing required.** Keymap and theme in TOML (impala/bluetui precedent: `~/.config/<app>/config.toml`, `-c` override [V]). Ship 3-4 themes. Errors in config show line + fix, and the app still starts with defaults.
18. **Speed is the feature.** Start in <100 ms with cached state, then refresh asynchronously; never block UI on D-Bus calls; show skeleton/"Scanning..." rows. k9s/yazi feel premium largely from async [K].

## Anti-patterns
- Esc quits the app (both reference Wi-Fi/Bluetooth TUIs disable this for a reason [V]).
- Single-letter bindings only, no on-screen discoverability (impala/bluetui style [V]) and then no palette.
- Rows re-sorting under the cursor during a scan; selection jumping.
- Showing dBm/BSSID/netmask to beginners as primary data.
- Destructive actions without naming the object; irreversible toggles without undo.
- Fixed 60 fps redraw loop; looping/decorative animation.
- Hard-coded RGB colors; opaque background; color-only status; Nerd Font glyphs with no fallback; crash on small terminal.
- Raw backend error strings / stack traces in the main view.
- Modal-in-modal chains; forms that clear input on failure.
- Silent conflict with another network daemon.

## Layout sketches (width ~100; "▂▄▆█" = signal)

### A. Tabs + master/detail (recommended default)
```
 Nexus   Overview  [Wi-Fi]  VPN  Ethernet  Details                 ? help  : cmd
┌ Networks ─────────────────────────────┐┌ Home-5G ────────────────────────────┐
│ ● Home-5G            ▂▄▆█  WPA3  5GHz ││ Connected · Strong signal           │
│   Home-2.4           ▂▄▆_  WPA2  2.4  ││ Speed   ▁▂▃▅▇▆▅▃ 312 Mbps ↓ 18 ↑    │
│   CafeGuest          ▂▄__  Open  2.4  ││ IP      192.168.1.42                │
│   Neighbor_Net       ▂___  WPA2  2.4  ││ Gateway 192.168.1.1  · 4 ms         │
│ ── Saved, out of range ───────────────││ Autoconnect  [x]                    │
│   Office-WiFi        (not in range)   ││ Advanced ▸ (BSSID, channel, DNS)    │
└───────────────────────────────────────┘└─────────────────────────────────────┘
 ⏎ connect   d forget   a autoconnect   / filter   s rescan        Wi-Fi: on
```

### B. Dashboard overview (home screen, single column below 100 cols)
```
 Nexus                                                          ? help  : cmd
 You're online via Wi-Fi (Home-5G). Everything looks healthy.
┌ Wi-Fi ─────────────┐┌ VPN ───────────────┐┌ Ethernet ──────────┐
│ Home-5G  ▂▄▆█      ││ Work-WG   ○ off    ││ Not connected      │
│ ↓ 312  ▁▃▅▇▆▅▃     ││ [c] connect        ││                    │
└────────────────────┘└────────────────────┘└────────────────────┘
┌ Recent ──────────────────────────────────────────────────────────┐
│ 10:42 Joined Home-5G    10:40 Wi-Fi turned on                    │
└──────────────────────────────────────────────────────────────────┘
 Tab next card   ⏎ open   w Wi-Fi   v VPN   ? help
```

### C. Single focus list + slide-in sheet (small terminals / minimalist mode)
```
 Wi-Fi ▾                                   ● Home-5G  ▂▄▆█
 ──────────────────────────────────────────────────────────
 ▸ Home-5G         ▂▄▆█  connected
   Home-2.4        ▂▄▆_
   CafeGuest       ▂▄__  open
 ⏎ details  / filter                                  ? help
```
Enter pushes a full-screen detail sheet; Esc pops. Same data model as A, different breakpoint.

## Recommendation
Build A as the default with B as the Overview tab; C is the automatic <80-col fallback of the same widgets (one layout engine, three breakpoints). Gate all of it on a semantic theme + ASCII mode from day one.

## Confidence / gaps
Medium. Only the four sources above were fetched. Peer-app behavior (lazygit, helix, btop, yazi, k9s, posting, atuin, etc.) is from memory; ratatui showcase winners 2024-26 not surveyed; OSC 11 and truecolor detection behavior across terminals unverified.
