# Landscape: Linux network-management frontends on Wayland (as of 2026-11-09)

Evidence tags: **[V]** verified from a primary source fetched this session (README, GitHub API, issue list); **[S]** search-engine summary only; **[K]** my prior knowledge, NOT verified this session. Reddit and the Arch wiki returned 403/Anubis blocks, so Reddit/forum claims rest on search snippets only.

## 1. Per-tool findings

### nmtui / nmcli (NetworkManager, in-tree)
- Backend NM. nmcli covers every NM type: NM's settings reference lists wired, wifi, bluetooth, gsm/cdma, bond, bridge, team, vlan, wireguard, vpn, macsec, ovs, ip-tunnel etc. [V: https://networkmanager.dev/docs/api/latest/nm-settings-nmcli.html]. nmcli is the only tool in this list with full coverage, but it is a scripting CLI, not a UX.
- nmtui: [K] curses; edit/activate connections, hostname. Known gaps (unverified): no WireGuard creation, no live signal/refresh, no QR share, no captive-portal handling, no secret-agent integration beyond prompting. Not themeable beyond terminal palette. Maintenance: ships with NM (active).

### nm-applet + nm-connection-editor (network-manager-applet)
- Backend NM; a GTK secret agent that stores secrets in a keyring (gnome-keyring/libsecret) [K]. Editor covers 802.1X, VPN plugins, bond/bridge/VLAN, proxy tab [K].
- Wayland pain: tray depends on StatusNotifier; reports of icon missing when Waybar starts after nm-applet, duplicate SNI registration warnings, `--indicator` flag errors on Sway, needing `sleep 2` in autostart [S: Hyprland forum / Gentoo / KDE bug 477347 results; e.g. https://bugs.kde.org/show_bug.cgi?id=477347, https://github.com/hyprwm/Hyprland/discussions/13083]. Keyring re-prompts at every login without PAM unlock are a decade-old, recurring bug class [S: https://bugs.launchpad.net/bugs/996578 and siblings].
- Captive portal: a 2017 Debian thread says nm-applet "does not seem to do anything" with the portal state while GNOME Shell does [S, old]. Notifications via libnotify. Theming: GTK only. Maintenance: alive but legacy; last man-page build I saw was 1.36.0 [S].

### GNOME Settings (Network panel) and GNOME Shell
- NM-only; handles Wi-Fi, wired, VPN (WireGuard/OpenVPN/plugins), proxy, hotspot [K]. Captive portal via Shell's Portal Helper; GNOME MR 2228 changed it to read NM's configured check URI rather than hard-coded nmcheck.gnome.org [V-ish: https://gitlab.gnome.org/GNOME/gnome-shell/-/merge_requests/2228]. Requires GNOME session components (gnome-keyring, polkit agent, control-center); not practical standalone. No launcher/theming hook.

### KDE plasma-nm
- NM-only, Qt/QML; a kded module watches NM connectivity and opens a web view when state == PORTAL [V: https://mail.kde.org/pipermail/plasma-devel/2016-September/058679.html]; later uses NM's configured check URI (MR 391, https://invent.kde.org/plasma/plasma-nm/-/merge_requests/391). Bugs: portal redirect breaks on servers that don't rewrite certain URLs, reproduced on Arch with its ping.archlinux.org check (https://bugs.kde.org/show_bug.cgi?id=500849); stale limited-connectivity status after login (https://bugs.kde.org/show_bug.cgi?id=411848); a login notification fires for every network (https://bugs.kde.org/show_bug.cgi?id=452355). Secrets via KWallet. Needs Plasma stack; poor fit for Hyprland/niri.
- `nm-tray` (lxqt-family; fork https://github.com/palinek/nm-tray, 73 stars, pushed 2026-03) is the usual non-KDE Qt tray; the search flagged it as a fallback when nm-applet shows no icon [S].

### impala (pythops/impala) — iwd TUI
- Backend **iwd** only; README: disable NetworkManager/wpa_supplicant to avoid conflicts. Features: WPA-Enterprise 802.1X, station + AP mode, QR share, hidden networks, nerd-font/ASCII, custom keys/colors [V: https://raw.githubusercontent.com/pythops/impala/main/Readme.md]. No Ethernet, VPN, captive portal, or DNS/proxy.
- 2891 stars, pushed 2026-10-07, 1 open issue [V gh api]. Issue history is revealing [V gh issue list]: WireGuard configuration request, "what do I use for wired?", PEAP-MSCHAPv2 generates invalid credentials, EAP-TTLS "Required field", WPA3-Enterprise fails "Operation not supported", "Networks not being saved", "passwords not persistent", 802.1X fails in one building, "assumes sudo", invisible AP form, frozen UI, config.toml not read, WPA3-personal request, password show/hide.
- Forks: wlctl (NM port), impala-nm [S].

### iwgtk — iwd GTK GUI
- iwd only; view/connect, known networks, WPS and Wi-Fi Easy Connect provisioning, tray indicator daemon (`iwgtk -i`), config-file icon colors [V: https://github.com/J-Lentz/iwgtk README]. Wi-Fi only; GTK; no Ethernet/VPN. Needs SNI tray. (Upstream stars/pushed date not collected.)

### iwmenu / bzmenu (e-tho) — launcher-driven
- iwmenu: iwd + dbus, launcher via stdin; fuzzel, rofi, dmenu, bemenu, custom (walker issues); optional notification daemon; `--interactive` stays in menus; font or XDG icons [V: https://raw.githubusercontent.com/e-tho/iwmenu/main/README.md]. README does not claim WPA-Enterprise or hidden networks. 466 stars, pushed 2026-11-07, 5 open [V gh api]. Issues: fuzzel password prompt, walker flags, dbus access denied, "edit/add network" (open), crash dismissing scan notification (open).
- bzmenu: same design for Bluetooth/BlueZ (GPL-3.0, Rust) [S: https://www.linuxlinks.com/bzmenu-launcher-driven-bluetooth-manager-linux/].

### networkmanager-dmenu (firecat53)
- NM via PyGObject. Connect to existing WiFi/wired/VPN/WireGuard/GSM/Bluetooth; new Wi-Fi with passphrase; toggle radios; delete connections; rescan; show password/QR; manage (not create) hotspots; launch nm-connection-editor; optional pinentry; launchers bemenu, dmenu, fuzzel, rofi, tofi, walker, wmenu, wofi, yofi; needs Python 3.11, polkit config for non-root; UWSM needs extra config [V: https://github.com/firecat53/networkmanager-dmenu]. 1006 stars, pushed 2026-10-12, 5 open [V]. Issue history [V gh]: no passphrase prompt on fuzzel, GLib-GObject-CRITICAL on latest Arch, bluetoothctl crashing sway, "Can't get show wifi password to work", fuzzel height exceeds screen, 3GPP manual selection request. In Arch `extra` (issue #… 2025-10).
- Other rofi/fuzzel/walker scripts and `network-dmenu` (Rust crate 1.7.0, [S]) follow the same pattern; no secret agent, no live state.

### nmrs (freedesktop-rs/nmrs, Rust crate) — library, not a frontend
- 3.5.7 on docs.rs; async zbus. Wi-Fi PSK/EAP/open, Ethernet, VPN (WireGuard, OpenVPN, OpenConnect, strongSwan, PPTP, L2TP), saved-profile CRUD, signal monitoring, airplane mode with rfkill, connectivity state + **captive-portal URL detection**, and a **NetworkManager secret agent** API [V: https://docs.rs/crate/nmrs/latest/source/README.md]. 328 stars, pushed 2026-11-09, 14 open [V]. Used by COSMIC per a forum post [S].
- Open gaps [V gh issues]: wired 802.1X builder, OpenConnect/multi-secret VPN auth dialog, OWE, `connection.secondaries`; recent bugs about OpenVPN import routing all traffic, WireGuard PSK dropped, mis-mapped device reason codes. Lesson: NM's D-Bus surface is large and subtle; a thin library still has fresh correctness bugs.

### wlctl (aashish-thapa) and other new TUIs
- wlctl: NM fork of impala, 189 stars, pushed 2026-11-06 [V]. Open issues are the demand list: VPN (WireGuard/OpenVPN) with per-SSID trust, captive-portal auto-login, hotspot, profile export/import, `--ascii`; closed bugs: Ethernet invisible when Wi-Fi off, Wi-Fi shown connected while on Ethernet, soft-blocked radio after power off, "share requires root" [V gh issue list].
- gazelle-tui (Python/Textual, NM, "complete 802.1X"; 144 stars, pushed 2026-04) built for Omarchy/eduroam [V: https://github.com/Zeus-Deus/gazelle-tui]; its existence signals 802.1X in a TUI was unmet.
- nettui (iwd + systemd-networkd for Ethernet: link up/down, DHCP renew, AP mode, QR; 9 stars, pushed 2026-10) [V: https://docs.rs/crate/nettui/0.3.2/source/README.md] — first TUI mixing iwd and networkd.
- Long tail of 0-star nmcli-wrappers (wifi-tui, wifui, nm-wifi, netman (Go), darknet-tui (Go), connmanTUI) pushed 2025-2026 [V gh search]. Nexus v1 itself appears in these listings.

### ConnMan: connmanctl, cmst, connmanTUI
- Backend ConnMan. connmanctl needs you to register an agent yourself; passphrase prompt is cancelled after a wrong entry; backslash-ending passphrases fail; long passphrases hit "Argument list too long" [S: https://bugs.archlinux.org/task/68996, https://lore.kernel.org/connman/e3c2ebb9-9f63-19bc-4969-e892c8aca433@collabora.com/T/]. cmst (Qt tray, andrew-bibb/cmst, 182 stars, pushed 2026-01-31) keeps an agent registered permanently [V gh; S]. ConnMan has WireGuard/OpenVPN/VPN plugins and tethering [K]; ConnMan is embedded-oriented, tiny community on desktops, 802.1X by config file [K]. connmanTUI exists (0 stars) [V gh].

### wpa_gui / wpa_cli
- wpa_supplicant Qt GUI; the only maintained copy I found was a 2022 mirror (https://github.com/stevenhoneyman/wpa_gui) [V gh]; upstream ships in hostap, Qt5-era, effectively legacy [K]. Wi-Fi only; no tray on Wayland without SNI.

### Bar widgets: Waybar, ironbar, HyprPanel, AGS/Astal
- Waybar `network` module (12.1k stars, 721 open issues, pushed today [V]): display only. Open requests: "Support for network/wifi connecting?" (2024), hotspot, bond support, no-internet state, ignored-interfaces, interface by MAC, deadlock under mutex (2026-10), wifi chosen over connected Ethernet (2026-03), "connected" shown when radio is off (2023) [V gh search]. Usual pattern: on-click spawns nm-connection-editor/kitty nmtui/impala.
- ironbar: open issues "Network Manager module" (2023-05) and "iwd module" (2025-06) [V gh]. Tray works via SNI.
- HyprPanel: **archived** (pushed 2026-04-23, 2182 stars) [V gh]; AGS/Astal (3.1k/1.0k stars, active) provide NetworkManager bindings; community hand-rolled panels (e.g. JDongian/ags-nm-panel, "nm-applet alternative") [V gh search]. Each rice reimplements Wi-Fi list + password popup and drops secret agent/captive/VPN.

## 2. Feature matrix

Legend: Y yes, P partial/existing-profiles-only/needs external tool, N no, ? unverified. Basis mixes [V]/[S]/[K] as above.

| Tool | Backend | Wi-Fi PSK | 802.1X | Ethernet | WireGuard | Other VPN | Hotspot | BT tether | Mobile | VLAN/bridge/bond | Proxy | DNS | Secret agent | Captive portal | Tray/notify | Launcher | Theming | Status |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| nmcli | NM | Y | Y | Y | Y | Y | Y | Y | Y | Y | Y | Y | P (`--ask`) | N | N | N | N | active |
| nmtui | NM | Y | P/? | Y | ? (likely N) | P | N? | N? | N? | P | N | P | prompts | N | N | N | terminal only | active |
| nm-applet + editor | NM | Y | Y | Y | Y | Y | Y | Y | Y | Y | Y | Y | Y (keyring) | P | SNI tray + libnotify | N | GTK | legacy, alive |
| GNOME Settings | NM | Y | Y | Y | Y | Y | Y | Y | Y | Y | Y | Y | Y | Y (Shell helper) | Shell | N | GNOME | active, DE-bound |
| plasma-nm | NM | Y | Y | Y | Y | Y | Y | Y | Y | Y | Y | Y | Y (KWallet) | Y (buggy) | Plasma | N | Plasma | active, DE-bound |
| impala | iwd | Y | Y | N | N | N | Y (AP) | N | N | N | N | N | iwd internal | N | N | N | TOML colors | active |
| iwgtk | iwd | Y | ? | N | N | N | ? | N | N | N | N | N | iwd internal | N | tray | N | GTK + icon colors | ? |
| iwmenu | iwd | Y | ? | N | N | N | ? | N | N | N | N | N | via launcher | N | notifs | Y | launcher | active |
| networkmanager-dmenu | NM | Y | P (existing) | Y | P (existing) | P (existing) | P (manage) | P | P | N | N | N | pinentry/launcher | N | notify-send | Y (many) | launcher | active |
| nmrs (lib) | NM | Y | Y | Y | Y | Y (6 types) | partial | ? | ? | ? | ? | global DNS | Y | Y (URL) | n/a | n/a | n/a | very active |
| wlctl | NM | Y | ? | Y | N (requested) | N | N (requested) | N | N | N | N | N | NM stored | N (requested) | N | N | TOML | active |
| gazelle-tui | NM | Y | Y | ? | N | N | N | N | N | N | N | N | NM stored | N | N | N | dynamic theme | slow |
| nettui | iwd + networkd | Y | ? | Y (link/DHCP) | N | N | Y | N | N | N | N | N | iwd | N | N | N | keybinds | new |
| ConnMan (connmanctl/cmst) | ConnMan | Y | P (file) | Y | Y? | Y? | Y | Y | Y? | N | Y | Y | Y (agent) | Y? | cmst tray | N | Qt | niche |
| wpa_gui | wpa_supplicant | Y | Y | N | N | N | N | N | N | N | N | N | internal | N | tray(old) | N | Qt | legacy |
| Waybar/ironbar modules | netlink / NM | display | display | display | N | N | N | N | N | N | N | N | N | N | bar | N | CSS | active |

## 3. Recurring pain points (with evidence)

1. **Backend fragmentation and conflicts.** iwd TUIs require disabling NM/wpa_supplicant (impala/nettui READMEs); Hyprland/Arch users report NM failing where iwd works and vice versa (iwd weak at 802.1X, no Proton VPN without NM) [S: r/archlinux results incl. https://libreddit.winscloud.net/r/archlinux/comments/1nxzxhx/…, https://universal-blue.discourse.group/t/network-and-bluetooth-managers-in-hyprland/6491]. Users must choose a stack before choosing a UI.
2. **Wi-Fi-only scope.** impala issues: WireGuard config request, "what do I use for wired?"; wlctl: VPN, hotspot, Ethernet-visible bugs; Waybar: "wifi chosen while Ethernet is the connection". Nothing unifies Wi-Fi + Ethernet + VPN + hotspot in a themeable, compositor-neutral UI.
3. **Enterprise Wi-Fi (802.1X/eduroam) is brittle.** impala has at least 6 closed EAP bugs (PEAP-MSCHAPv2, TTLS, WPA3-Enterprise, building-specific failure); gazelle-tui exists purely for this; nmrs has open wired-802.1X and multi-secret VPN items.
4. **Secrets/keyring.** nm-applet keyring re-prompt bug family spans 2005-2020 (Launchpad); standalone compositors lack gnome-session to unlock keyring; launcher tools fall back on pinentry or launcher password modes (fuzzel passphrase prompt failures in both iwmenu #? and networkmanager-dmenu 2026-05); impala "passwords not persistent / networks not saved".
5. **Tray fragility on Wayland.** nm-applet icon missing/duplicated depending on Waybar start order, `--indicator` errors, need for sleep hacks; HyprPanel archived; every rice reimplements.
6. **Captive portals are unhandled outside GNOME/KDE.** wlctl open request "auto-login for captive portals"; plasma-nm itself has 3 long-running portal bugs; nm-applet silent; only nmrs exposes the portal URL as an API. Users hand-roll `xdg-open` scripts (e.g. https://github.com/dataforxyz/wireguard-reconnect "isolated captive-portal workflow").
7. **Privilege/polkit friction.** networkmanager-dmenu needs polkit config; impala "assumes sudo"; wlctl "share requires root"; iwmenu "dbus error access denied".
8. **Launcher/terminal portability.** fuzzel/walker/wofi/rofi flag incompatibilities, UWSM quirks, cursor-position timeout in terminals (impala #171, 2026-10), nerd-font dependency and ASCII fallback requests, no mouse (impala request).
9. **Bar widgets are read-only and buggy.** Waybar network module: deadlock, wrong interface selection, stale "connected" state; polling vs D-Bus events (open since 2021).
10. **Fragmented, low-bus-factor ecosystem.** Dozens of 0-star nmcli-wrapper TUIs, one-maintainer launcher tools, HyprPanel archived, hotspot/VPN "can only manage existing".
11. **Weak diagnostics.** wlctl "doctor" request; impala "fails to show connection error"; opaque NM reason codes (nmrs mis-mapped reason codes bug).

## 4. Ranked gaps a best-in-class product could fill

1. **Backend-agnostic core (NM first, iwd second) behind one UI**, auto-detecting what is running and refusing to fight the other stack. Removes pain #1.
2. **A real user-session secret agent** that works without GNOME/KDE (own encrypted store or libsecret/KeePassXC-compatible, pinentry-style prompt in TUI and launcher), plus persistent saved networks. Pain #4. nmrs shows the NM secret-agent API is implementable in Rust.
3. **First-class 802.1X/eduroam/WPA3-Enterprise** with profile templates, CA validation prompts, and clear errors. Pain #3.
4. **Unified connection types**: Wi-Fi + Ethernet + WireGuard (import .conf, per-SSID auto-connect) + OpenVPN/other plugins + hotspot + Bluetooth PAN + static IP/DNS/proxy in one TUI. Pains #2; nobody covers it.
5. **Captive-portal handling in the daemon**: watch NM connectivity state, notify, open the portal in an isolated browser window, track completion. Pain #6; only GNOME/KDE do this.
6. **Daemon + notifications + SNI tray + JSON/Waybar `custom` output + launcher mode** (fuzzel/rofi/walker/dmenu stdin) all driven from one session daemon, so bar widgets read events instead of polling. Pains #5, #8, #9.
7. **Diagnostics view**: why did connect fail (decoded NM reason codes), connectivity checks, DNS/route/rfkill status, `doctor` command. Pain #11.
8. **Privilege model that just works** via polkit rules shipped with the package; no sudo. Pain #7.
9. **Theming** that follows terminal/Hyprland/matugen colours, ASCII fallback, mouse support. Table stakes after impala.
10. Lower: profile export/import, QR share, speed test, per-network metered flags, mobile broadband.

## 5. Caveats
- nmtui, nm-applet, GNOME Settings, ConnMan capability cells are [K] (not fetched). Matrix "?" cells are genuinely unknown.
- Reddit threads were blocked; r/hyprland sentiment is inferred from search snippets and GitHub issue content.
- Star counts/dates from `gh api` on 2026-11-09. iwgtk repo is J-Lentz/iwgtk (stars/date not collected).
