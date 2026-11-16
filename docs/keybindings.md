# Keybindings

Every key in the Nexus TUI. The list comes from the keymap in [`crates/nexus/src/tui/verbs.rs`](../crates/nexus/src/tui/verbs.rs), the same table the <kbd>?</kbd> overlay shows. Keys are fixed; there is no remapping.

Any action, keyed or not, is also in the command palette: press <kbd>:</kbd> and type a few words, such as `forget network` or `rescan`.

## Everywhere

| Key | Action |
| --- | --- |
| <kbd>:</kbd> | Open the command palette |
| <kbd>j</kbd> / <kbd>k</kbd>, <kbd>↓</kbd> / <kbd>↑</kbd> | Move in the list |
| <kbd>g</kbd> / <kbd>G</kbd> | Jump to top / bottom |
| <kbd>h</kbd> / <kbd>l</kbd>, <kbd>←</kbd> / <kbd>→</kbd> | Previous / next tab |
| <kbd>Tab</kbd> / <kbd>Shift</kbd>+<kbd>Tab</kbd> | Next / previous tab |
| <kbd>1</kbd> – <kbd>6</kbd> | Jump to a tab: Overview, Wi-Fi, Ethernet, VPN, Hotspot, Settings |
| <kbd>Enter</kbd> | Overview: open the selected card's tab |
| <kbd>d</kbd> | Details sheet (small windows) |
| <kbd>Esc</kbd> | Back out; never quits |
| <kbd>/</kbd> | Filter the Wi-Fi or VPN list by name (<kbd>Esc</kbd> clears) |
| <kbd>w</kbd> | Turn Wi-Fi off or on |
| <kbd>A</kbd> | Airplane mode on or off: every radio off |
| <kbd>u</kbd> | Wi-Fi, Hotspot: undo the change in the toast |
| <kbd>o</kbd> | Open a Portal's login page |
| <kbd>f</kbd> | Overview: run the suggested fix (asks first) |
| <kbd>?</kbd> | Show or hide the help |
| <kbd>q</kbd> | Quit (<kbd>Ctrl</kbd>+<kbd>C</kbd> works anywhere) |

## Wi-Fi

| Key | Action |
| --- | --- |
| <kbd>Enter</kbd> | Join the selected Network |
| <kbd>r</kbd> | Rescan |
| <kbd>x</kbd> | Disconnect |
| <kbd>f</kbd> | Forget (asks first) |
| <kbd>s</kbd> | Share as a QR code (asks first) |
| <kbd>a</kbd> | Toggle autoconnect |
| <kbd>t</kbd> | Mark Trusted or Untrusted |
| <kbd>e</kbd> | Edit the Profile's settings |
| <kbd>n</kbd> | Join a hidden Network by name |
| <kbd>c</kbd> | Pick the Wi-Fi card joins use |
| <kbd>i</kbd> | Access point details of the Network |

## Ethernet

| Key | Action |
| --- | --- |
| <kbd>Enter</kbd> | Join the selected port |
| <kbd>x</kbd> | Drop the port's Link |
| <kbd>s</kbd> | Sign in to an Enterprise (802.1X) port |
| <kbd>e</kbd> | Edit the port's Profile settings |

## VPN

| Key | Action |
| --- | --- |
| <kbd>Enter</kbd> | Bring the VPN up or down |
| <kbd>i</kbd> | Import a WireGuard `.conf` |
| <kbd>c</kbd> | Create a WireGuard VPN with new keys |
| <kbd>p</kbd> | Show a VPN's public key |
| <kbd>s</kbd> | Share a VPN's config as a QR code (asks first) |

## Hotspot

The Hotspot tab also lists the paired phones you can tether through.

| Key | Action |
| --- | --- |
| <kbd>n</kbd> | Share this internet with a made-up password |
| <kbd>c</kbd> | Start a Hotspot with a name and password you choose |
| <kbd>s</kbd> | Show the Hotspot's QR code and password (asks first) |
| <kbd>S</kbd> | Stop the Hotspot |
| <kbd>Enter</kbd> | Use the selected phone's data |
| <kbd>x</kbd> | Drop the phone's tether |

## Settings

| Key | Action |
| --- | --- |
| <kbd>Enter</kbd> | Change the selected setting |
| <kbd>r</kbd> | Put the default back |

## Forms and prompts

| Key | Action |
| --- | --- |
| <kbd>Tab</kbd> | Next field |
| <kbd>Space</kbd>, <kbd>←</kbd> / <kbd>→</kbd> | Change a choice |
| <kbd>Ctrl</kbd>+<kbd>R</kbd> | Show or hide passwords |
| <kbd>Enter</kbd> | Apply, join or send |
| <kbd>Esc</kbd> | Cancel |

While a change to the network you're on counts down, <kbd>y</kbd> keeps it and <kbd>n</kbd> reverts it.

[Back to the README](../README.md)
