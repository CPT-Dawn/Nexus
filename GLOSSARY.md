# Nexus

A network manager for Wayland desktops: one Daemon and its Front ends let a single user join, configure and diagnose every kind of network their machine can reach.

## Language

### Networking

**Network**:
Something the user can join: a Wi-Fi SSID, an Ethernet port, a VPN, a hotspot, a tethered phone.
_Avoid_: connection, access point, SSID (as the user-facing name)

**Profile**:
The saved settings Nexus uses to join one Network. A Network can be joined without a Profile until the first join succeeds.
_Avoid_: connection, saved connection, configuration

**Device**:
A hardware or virtual interface that can carry a Network, such as a Wi-Fi card, an Ethernet port or a WireGuard tunnel.
_Avoid_: adapter, interface (in the UI), NIC

**Link**:
A Device's live state while it carries a Network: the joined Network, addresses, speed and traffic.
_Avoid_: active connection, session

**Forget**:
Deleting a Profile, so Nexus no longer joins its Network automatically.
_Avoid_: delete connection, remove

### System

**Backend**:
The system network service Nexus drives to do the actual networking; NetworkManager in v2.
_Avoid_: provider, driver

**Daemon**:
The single long-running Nexus process in the user's session that owns all state and talks to the Backend.
_Avoid_: server, service (alone), agent

**Front end**:
Any program that shows or changes Nexus state only through the Daemon: the TUI, the CLI, a status-bar module.
_Avoid_: client, UI
