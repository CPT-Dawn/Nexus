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

**Enterprise Network**:
A Network that makes each person sign in with their own identity and a password or certificate, such as eduroam or an office Wi-Fi or Ethernet port. Joined through a guided sign-in form.
_Avoid_: WPA-Enterprise, 802.1X, EAP (in the UI; fine in code and the CLI flags)
**Pending change**:
An edit to the Profile carrying the current Link that has been applied but not yet kept. It reverts by itself unless the person confirms within the countdown (ADR-0004).
_Avoid_: draft, transaction, trial

**Forget**:
Deleting a Profile, so Nexus no longer joins its Network automatically.
_Avoid_: delete connection, remove

**Trust**:
The user's judgement of a Network, either Trusted or Untrusted, which decides policies such as bringing up a VPN on join. Unknown open Networks start Untrusted.
_Avoid_: security level, zone, public/private

**Portal**:
A login or terms page a Network demands before it gives real internet access.
_Avoid_: captive page, splash page, hotspot login

**Hotspot**:
A Network this machine creates so other devices can share its internet connection. Started with a generated strong password unless the person chooses one, shown as a QR code only after an explicit reveal, and forgotten when stopped: its Profile lives in memory, not on disk. Needs a Wi-Fi card whose driver offers access point mode; on a single-radio card the Wi-Fi Link usually drops while it runs. Devices that joined it are counted, not listed.
_Avoid_: access point, AP mode, tethering (that is joining a phone's Network)

**Tethering**:
Joining the internet through a phone, over Bluetooth or USB.
_Avoid_: hotspot, PAN

**Phone**:
A paired Bluetooth phone that offers network access, which Tethering joins. Listed by name and Bluetooth address; a phone that offers only dial-up is not a Phone to Nexus.
_Avoid_: device (that is a hardware interface), handset, PAN client

**VPN**:
A Network that tunnels all or some traffic through another machine: a WireGuard tunnel or a plugin VPN such as OpenVPN. Each VPN has a Profile, is up or down independently of the Link that carries it, and a VPN that goes down without being asked to is a drop, which Nexus announces.
_Avoid_: tunnel (in the UI; fine for what a WireGuard peer pair is), connection

**VPN plugin**:
The NetworkManager add-on (its package, e.g. `networkmanager-openvpn`) that knows how to read a provider file such as `.ovpn` and run that kind of tunnel. A plugin VPN's Profile names its plugin; if the plugin is not installed Nexus says which package to install instead of failing obscurely.
_Avoid_: driver, module

**Public key**:
The half of a WireGuard key pair that is safe to hand out: the other side of a tunnel needs it. Nexus generates the pair when it creates a VPN, and the private half never leaves NetworkManager except through a Reveal.
_Avoid_: key (alone, ambiguous with the private key)

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

**Secret prompt**:
A request from the Backend for something only the user knows, such as a Wi-Fi password or a VPN one-time code, answered through a Front end.
_Avoid_: password dialog, auth request

**Keyring**:
The user's Secret Service (gnome-keyring, KWallet, KeePassXC) as an opt-in home for one Profile's Wi-Fi password. A Profile "kept in the keyring" has its password marked agent-owned, so NetworkManager never stores it and asks the Daemon, which answers from the keyring. The system store (NetworkManager's own) stays the default.
_Avoid_: wallet, vault, password manager

**Reveal**:
Reading a stored secret back out of the Backend, such as a saved Wi-Fi password, so it can be shown or turned into a QR code. Always an explicit act of the user, and the Backend must authorize it.
_Avoid_: unlock, decrypt, export

**Doctor**:
The set of checks that explains, in plain language, why the machine is or is not online and what to do about it.
_Avoid_: diagnostics, troubleshooter, health check

**Verb**:
One thing the TUI can do, named in plain English ("Forget network") with its key. The single table of Verbs drives the command palette and the `?` help, so a key is documented and findable by being listed once.
_Avoid_: command, action, menu item

**Effect**:
A short animation the TUI plays because something happened: a tab glides in, a joined Network blooms, the focal element pulses while connecting, an error toast shakes once, a toast slides in. Every Effect starts from an event and ends by itself; nothing animates on its own, so an idle TUI draws no frames.
_Avoid_: animation, transition, ambient motion

**Reduced motion**:
The mode (`NEXUS_REDUCE_MOTION=1` or `REDUCE_MOTION=1`) in which nothing moves: tabs change at once, toasts appear in place, no pulse, no shake. The colour changes that say what happened stay. Terminals with 16 colours or none play no Effects at all.
_Avoid_: no-animation mode, calm mode
