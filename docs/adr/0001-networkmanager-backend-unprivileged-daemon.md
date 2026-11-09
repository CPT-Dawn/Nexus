# NetworkManager is the only Backend, driven by an unprivileged session Daemon

Nexus v2 does no networking itself: it drives NetworkManager over the system D-Bus from a Daemon that runs as a `systemd --user` service and never as root, leaving authorization to NetworkManager's polkit rules. NetworkManager already solves the cases an all-in-one product needs (secret agents, 802.1X, VPN plugins, hotspot, WireGuard, modems, Bluetooth tethering, connectivity and captive-portal checks, checkpoints with rollback); reimplementing them would take years and a root process.

## Considered Options

- **iwd + systemd-networkd + resolved as a second Backend**: iwd's D-Bus works without root, but networkd is configured through root-owned files in `/etc`, which would need a privileged helper, and the stack has no VPN plugins, captive-portal detection, metered state or modem support. Rejected for v2.0; Daemon code depends on a domain-level Backend interface with capability flags so an iwd Wi-Fi Backend can be added later.
- **Own root daemon over netlink, wpa_supplicant and a DHCP client**: rejected as unsafe and unbounded.

## Consequences

- Users on iwd-only or networkd setups must switch to NetworkManager (it can keep iwd as its Wi-Fi backend). First run must detect this and explain the fix.
- Importing `.ovpn` and similar VPN files is not available over NetworkManager's D-Bus API; Nexus shells out to `nmcli connection import` for plugin VPNs and parses WireGuard `.conf` files itself.
