# Risky Profile edits run under a NetworkManager checkpoint, confirmed by the person

An edit to the Profile that carries the current Link (static IP, DNS, MAC) can cut the machine off from the network, and a person locked out cannot click "undo". So the Daemon creates a NetworkManager checkpoint with a rollback timeout (15 s by default) before it writes, and the change stays only if a Front end confirms in time. Edits to a Profile that carries no Link are written at once, with no checkpoint.

The rollback is NetworkManager's own timer, not the Daemon's: if the Daemon, the TUI or the whole session dies mid-countdown, NetworkManager still puts the old settings back. The Daemon keeps a mirror timer for the same instant only to tell Front ends how it ended (`profile_settled`) and to roll back by itself if NetworkManager somehow has not.

## Consequences

- One change may wait for confirmation at a time. A second edit while one waits is refused with `conflict`; it is settled by `profile_confirm`, `profile_revert` or the timeout. This keeps the countdown a single, unambiguous thing on every Front end.
- Every write uses `Update2` with the Profile's `version-id`, so an edit made meanwhile by `nmcli` or another program is reported as `conflict` instead of being overwritten.
- Settings Nexus cannot express (an IPv4 method of `link-local`, a fixed MAC address) are read as `unknown` and written back untouched; only fields the person changed are rewritten.
- The countdown length is a request parameter (`rollback_secs`), not a hidden test switch, so tests run the real timer for a second instead of fifteen.
- A checkpoint rolls back only what NetworkManager manages. It cannot undo a firewall rule, an rfkill switch or what a VPN server did.
- A wired Profile has no SSID, so profile ops address it by the Profile's uuid: `profile_get` and `profile_update` take `profile` (an `EthernetProfile.uuid`) instead of `network`. Everything else is shared: the same validation, the same `Gate` (one pending change across Wi-Fi and Ethernet), the same checkpoint and countdown. A wired Profile is active when one of its Devices has it as the active connection. The MAC option lives in `802-3-ethernet`, where "per network" maps to NetworkManager's `stable` (there is no SSID to be stable per).
