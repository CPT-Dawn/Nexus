# The Nexus socket API

Every Front end (the TUI, the CLI, `nexus menu`, `nexus status --waybar`, the Zenith `nexus` module) talks to the Daemon through one protocol: JSON lines on a Unix socket. This page is its contract. It is **version 1 and frozen** (see [Compatibility](#compatibility)): the types live in the `nexus-ipc` crate, and ADR-0002 records why it is a socket and not D-Bus.

Everything below is checked by the conformance suite (`crates/nexus/tests/ipc_conformance.rs`): every request example is sent to a real Daemon on a fake NetworkManager and its answer must have the shape shown, every event example must decode, and every op, event and error code the crate knows must be documented here. If this page and the Daemon disagree, CI fails.

## Connecting

```sh
SOCK="$XDG_RUNTIME_DIR/nexus/nexus.sock"
```

- The directory is mode `0700` and the Daemon checks every peer's user id (`SO_PEERCRED`): only you can talk to your Daemon.
- systemd socket activation starts the Daemon on the first connection, so a client never needs to launch it. `nexus daemon` runs it in the foreground.
- A connection stays open until either side closes it. A join waits until the Network is up or has failed before it answers, so a Front end gives it a connection of its own.

The examples use `socat` and `jq`. `-t1` makes `socat` wait one second for the answer after its input ends:

```sh
echo '{"v":1,"id":1,"op":"status"}' | socat -t1 - UNIX-CONNECT:"$SOCK" | jq .
```

Arrays in the examples are cut to their first element, and ids that NetworkManager makes up (`profile`, `uuid`, VPN `id`) differ on your machine.

## Messages

One JSON object per `\n`-terminated line, UTF-8, no pretty-printing on the wire.

| Direction | Shape |
|---|---|
| Request | `{"v":1,"id":7,"op":"wifi_list", ...arguments}` |
| Success | `{"v":1,"id":7,"ok":{...result}}` |
| Failure | `{"v":1,"id":7,"err":{"code":"...","message":"..."}}` |
| Event | `{"v":1,"event":"status_changed", ...payload}` (after `subscribe`) |

- `v` is the protocol version, `id` a number the client picks and the response repeats. A request too broken to carry an `id` is answered without one.
- Names are `snake_case`. A Network is named by its `id`, the SSID's bytes in lower-case hex, so SSIDs that are not UTF-8 work. `wifi_list` also returns the plain `ssid` (sanitised for display).
- Secrets (a password, a private key) travel as plain strings in the few requests and results that need them, are never logged, and are never part of an event. The ops that reveal a stored secret (`wifi_reveal`, `hotspot_reveal`, `vpn_reveal`) make NetworkManager ask polkit first, so call them only after the person asked.

## Compatibility

`nexus-ipc` v1 is a promise to everything that speaks to the Daemon, Zenith included.

- **Additive changes keep `v: 1`:** a new op, a new event, a new error code, a new optional field in a result or request, a new value in an enum a client already tolerates. A client must therefore ignore fields it does not know, and treat an unknown event name, error code or enum value as "something I do not understand", not as a failure of the connection. (`nexus-ipc` decodes these as `Unknown`.)
- **Breaking changes bump `v`:** removing or renaming an op, event, field or code, changing a type, or making an optional field required. The Daemon accepts the versions it lists (`nexus_ipc::SUPPORTED`); `unsupported_version` tells a client it is too new or too old.
- Documentation, messages and the order of keys may change at any time. Match on `code`, never on `message`.

## Requests


### Status

Overall state, and the primary Link's traffic.

#### `status`

Overall network state and the primary Link.

Command line: `nexus status [--json]`.

```sh
echo '{"v":1,"id":1,"op":"status"}' | socat -t1 - UNIX-CONNECT:"$SOCK" | jq .
```

```json
{
  "v": 1,
  "id": 1,
  "ok": {
    "backend": {
      "name": "networkmanager",
      "version": "1.58.1"
    },
    "connectivity": "full",
    "link": {
      "kind": "wifi",
      "name": "Home-5G"
    },
    "radios": {
      "airplane": false,
      "wifi": {
        "enabled": true,
        "hardware_blocked": false
      }
    },
    "state": "online"
  }
}
```

#### `traffic`

Download and upload rates of the primary Link, in bytes per second. Zero without a Link; the first call only sets a baseline.

Command line: none: a rate needs two samples a second apart, so only the TUI Overview polls it.

```sh
echo '{"v":1,"id":1,"op":"traffic"}' | socat -t1 - UNIX-CONNECT:"$SOCK" | jq .
```

```json
{
  "v": 1,
  "id": 1,
  "ok": {
    "rx_bps": 0,
    "tx_bps": 0
  }
}
```

### Wi-Fi

Nearby and saved Networks, joining, forgetting, radios. A Network is named by its `id`: the SSID's bytes in hex (so names that are not UTF-8 work too).

#### `wifi_list`

Nearby Wi-Fi Networks, one per SSID, plus saved out-of-range ones.

Command line: `nexus wifi list [--json]`.

```sh
echo '{"v":1,"id":1,"op":"wifi_list"}' | socat -t1 - UNIX-CONNECT:"$SOCK" | jq .
```

```json
{
  "v": 1,
  "id": 1,
  "ok": {
    "devices": [
      {
        "id": "/org/freedesktop/NetworkManager/Devices/0",
        "name": "wlan0"
      }
    ],
    "networks": [
      {
        "access_points": 1,
        "autoconnect": true,
        "band": "5",
        "device": "/org/freedesktop/NetworkManager/Devices/0",
        "id": "486f6d652d3547",
        "joined": true,
        "profile": "731c9f8c-e411-4629-816c-e74b33c6fdb8",
        "security": "wpa2",
        "signal": 82,
        "ssid": "Home-5G",
        "trust": "trusted"
      }
    ],
    "saved": [
      {
        "autoconnect": true,
        "id": "Parents House",
        "ssid": "Parents House",
        "uuid": "uuid-parents"
      }
    ]
  }
}
```

#### `wifi_scan`

Ask the Backend to refresh the Wi-Fi list. Results arrive as `networks_changed` events; the response only says what was done.

Command line: `nexus wifi scan`.

```sh
echo '{"v":1,"id":1,"op":"wifi_scan"}' | socat -t1 - UNIX-CONNECT:"$SOCK" | jq .
```

```json
{
  "v": 1,
  "id": 1,
  "ok": {
    "requested": 1,
    "skipped": 0
  }
}
```

#### `wifi_aps`

The access points behind a Network (`WifiNetwork.id`): address, channel, frequency, band and link rate of each. Result: `ApList`; `out_of_range` when none is heard.

Command line: `nexus wifi aps <ssid>`.

```sh
echo '{"v":1,"id":1,"network":"486f6d652d3547","op":"wifi_aps"}' | socat -t1 - UNIX-CONNECT:"$SOCK" | jq .
```

```json
{
  "v": 1,
  "id": 1,
  "ok": {
    "aps": [
      {
        "band": "5",
        "bssid": "AA:00:00:00:00:01",
        "channel": 36,
        "device": "wlan0",
        "frequency_mhz": 5180,
        "joined": true,
        "rate_mbps": 130,
        "signal": 82
      }
    ],
    "ssid": "Home-5G"
  }
}
```

#### `wifi_join`

Join a Wi-Fi Network, waiting until it is up or has failed. `network` is a `WifiNetwork.id`. A Profile is saved only if the join succeeds. Result: `Joined`. Failures carry a human sentence and one of the codes `wrong_password`, `out_of_range`, `no_address`, `join_failed`, `password_required`.

Command line: `nexus wifi join <ssid> [--hidden SEC] [--device IF]`.

```sh
echo '{"v":1,"id":1,"network":"43616665204775657374","op":"wifi_join"}' | socat -t1 - UNIX-CONNECT:"$SOCK" | jq .
```

```json
{
  "v": 1,
  "id": 1,
  "ok": {
    "profile": "e9e34a3a-12d5-451c-b0f7-04af884c4046",
    "ssid": "Cafe Guest"
  }
}
```

#### `wifi_disconnect`

Drop the current Wi-Fi Link, keeping its Profile. Result: `Disconnected`; `not_connected` when there is nothing to drop.

Command line: `nexus wifi disconnect`.

```sh
echo '{"v":1,"id":1,"op":"wifi_disconnect"}' | socat -t1 - UNIX-CONNECT:"$SOCK" | jq .
```

```json
{
  "v": 1,
  "id": 1,
  "ok": {
    "ssid": "Cafe Guest"
  }
}
```

#### `wifi_join_eap`

Join a WPA-Enterprise (802.1X) Network such as eduroam, with the settings in `eap`. Like `wifi_join` otherwise (result `Joined`); `invalid_settings` when `eap` fails its validation, `wrong_password` when the server refuses the sign-in.

Command line: `nexus wifi join <ssid> --eap METHOD ...`.

```sh
echo '{"v":1,"id":1,"eap":{"domain_suffix":"uni.example","identity":"you@uni.example","method":"peap","password":"correct-horse","phase2":"mschapv2"},"network":"656475726f616d","op":"wifi_join_eap"}' | socat -t1 - UNIX-CONNECT:"$SOCK" | jq .
```

```json
{
  "v": 1,
  "id": 1,
  "ok": {
    "profile": "d500d454-5742-4ab0-9b31-0e55a1a09a29",
    "ssid": "eduroam"
  }
}
```

#### `wifi_autoconnect`

Turn automatic joining of a saved Network on or off, by `WifiNetwork.id`. Result: `AutoconnectSet`; `unknown_network` when nothing is saved for it.

Command line: `nexus wifi autoconnect <ssid> on|off`.

```sh
echo '{"v":1,"id":1,"enabled":false,"network":"506172656e747320486f757365","op":"wifi_autoconnect"}' | socat -t1 - UNIX-CONNECT:"$SOCK" | jq .
```

```json
{
  "v": 1,
  "id": 1,
  "ok": {
    "enabled": false,
    "ssid": "Parents House"
  }
}
```

#### `trust_set`

Marks a nearby or joined Wi-Fi Network (`WifiNetwork.id`) Trusted or Untrusted. Result: `TrustSet`; `unknown_network` when it is not in the Wi-Fi list.

Command line: `nexus trust set <ssid> trusted|untrusted`.

```sh
echo '{"v":1,"id":1,"network":"43616665204775657374","op":"trust_set","trust":"untrusted"}' | socat -t1 - UNIX-CONNECT:"$SOCK" | jq .
```

```json
{
  "v": 1,
  "id": 1,
  "ok": {
    "ssid": "Cafe Guest",
    "trust": "untrusted"
  }
}
```

#### `wifi_reveal`

Reveal what a phone needs to join a saved Network, by `WifiNetwork.id`: reads the stored password, which needs the Backend's authorization (NetworkManager asks polkit). Result: `WifiShare`; `unknown_network` when nothing is saved for it, `reveal_denied` when authorization was refused. Front ends call it only after the person pressed a reveal key.

Command line: `nexus wifi share <ssid> [--qr]`.

```sh
echo '{"v":1,"id":1,"network":"506172656e747320486f757365","op":"wifi_reveal"}' | socat -t1 - UNIX-CONNECT:"$SOCK" | jq .
```

```json
{
  "v": 1,
  "id": 1,
  "ok": {
    "auth": "wpa",
    "password": "parents-wifi-2024",
    "ssid": "Parents House"
  }
}
```

#### `wifi_forget`

Forget (delete the Profile of) a Wi-Fi Network by `WifiNetwork.id`. Result: `Forgotten`; `unknown_network` when nothing is saved for it.

Command line: `nexus wifi forget <ssid>`.

```sh
echo '{"v":1,"id":1,"network":"43616665204775657374","op":"wifi_forget"}' | socat -t1 - UNIX-CONNECT:"$SOCK" | jq .
```

```json
{
  "v": 1,
  "id": 1,
  "ok": {
    "ssid": "Cafe Guest"
  }
}
```

#### `wifi_power`

Switch Wi-Fi on or off. Result: `Radios`. `hard_blocked` while a hardware switch holds the radio off.

Command line: `nexus wifi on|off`.

```sh
echo '{"v":1,"id":1,"on":true,"op":"wifi_power"}' | socat -t1 - UNIX-CONNECT:"$SOCK" | jq .
```

```json
{
  "v": 1,
  "id": 1,
  "ok": {
    "airplane": false,
    "wifi": {
      "enabled": true,
      "hardware_blocked": false
    }
  }
}
```

#### `airplane`

Airplane mode: every radio the Backend controls off (`on`) or back on: Wi-Fi, mobile broadband and Bluetooth. Bluetooth is powered off through BlueZ and, on leaving, back to what it was before. Result: `Radios`. `hard_blocked` when a hardware switch prevents it.

Command line: `nexus airplane on|off`.

```sh
echo '{"v":1,"id":1,"on":false,"op":"airplane"}' | socat -t1 - UNIX-CONNECT:"$SOCK" | jq .
```

```json
{
  "v": 1,
  "id": 1,
  "ok": {
    "airplane": false,
    "wifi": {
      "enabled": true,
      "hardware_blocked": false
    }
  }
}
```

### Profile settings

IP, DNS, MAC, metered and priority settings of a saved Network. A change to the Profile that carries the current Link is checkpointed: it rolls back on its own unless `profile_confirm` arrives in time.

#### `profile_get`

Read the settings of a saved Profile: a Wi-Fi Network's (`network` is a `WifiNetwork.id`) or, with `profile` (a Profile uuid, e.g. `EthernetProfile.uuid`), any Ethernet Profile. `profile` wins when both are given. Result: `ProfileView`; `unknown_network` when nothing is saved for it.

Command line: `nexus profile show <ssid>`.

```sh
echo '{"v":1,"id":1,"network":"656475726f616d","op":"profile_get"}' | socat -t1 - UNIX-CONNECT:"$SOCK" | jq .
```

```json
{
  "v": 1,
  "id": 1,
  "ok": {
    "active": true,
    "settings": {
      "dns": [],
      "ipv4": {
        "addresses": [],
        "method": "auto"
      },
      "ipv6": {
        "addresses": [],
        "method": "auto"
      },
      "keyring": false,
      "mac": "permanent",
      "metered": false,
      "priority": 0
    },
    "ssid": "eduroam",
    "uuid": "d500d454-5742-4ab0-9b31-0e55a1a09a29",
    "version": 1
  }
}
```

#### `profile_update`

Change the settings of a saved Profile. `version` is the one `profile_get` returned: a Profile that changed since fails with `conflict`. Invalid settings fail with `invalid_settings` before anything is written. When the Profile carries the current Link the change is checkpointed: the result has `pending`, and unless `profile_confirm` follows within `rollback_secs` (default 15) the Backend puts the old settings back. Result: `ProfileApplied`.

Command line: `nexus profile set <ssid> ...`.

```sh
echo '{"v":1,"id":1,"network":"656475726f616d","op":"profile_update","rollback_secs":30,"settings":{"dns":["9.9.9.9"],"ipv4":{"addresses":[],"method":"auto"},"ipv6":{"addresses":[],"method":"auto"},"keyring":false,"mac":"permanent","metered":true,"priority":0},"version":1}' | socat -t1 - UNIX-CONNECT:"$SOCK" | jq .
```

```json
{
  "v": 1,
  "id": 1,
  "ok": {
    "pending": {
      "id": 1,
      "rollback_secs": 30,
      "ssid": "eduroam"
    },
    "ssid": "eduroam",
    "version": 2
  }
}
```

#### `profile_confirm`

Keep a pending change (`pending` is `PendingChange.id`). Result `{}`; `nothing_pending` when it was confirmed, reverted or timed out already.

Command line: `nexus profile set ... --keep (or answer y)`.

```sh
echo '{"v":1,"id":1,"op":"profile_confirm","pending":1}' | socat -t1 - UNIX-CONNECT:"$SOCK" | jq .
```

```json
{
  "v": 1,
  "id": 1,
  "ok": {}
}
```

The same op again, to be rolled back:

```sh
echo '{"v":1,"id":1,"network":"656475726f616d","op":"profile_update","rollback_secs":30,"settings":{"dns":[],"ipv4":{"addresses":[],"method":"auto"},"ipv6":{"addresses":[],"method":"auto"},"keyring":false,"mac":"permanent","metered":false,"priority":0},"version":2}' | socat -t1 - UNIX-CONNECT:"$SOCK" | jq .
```

```json
{
  "v": 1,
  "id": 1,
  "ok": {
    "pending": {
      "id": 2,
      "rollback_secs": 30,
      "ssid": "eduroam"
    },
    "ssid": "eduroam",
    "version": 3
  }
}
```

#### `profile_revert`

Revert a pending change now instead of waiting. Result `{}`; `nothing_pending` as for `profile_confirm`.

Command line: `nexus profile set ... (answer n at the prompt)`.

```sh
echo '{"v":1,"id":1,"op":"profile_revert","pending":2}' | socat -t1 - UNIX-CONNECT:"$SOCK" | jq .
```

```json
{
  "v": 1,
  "id": 1,
  "ok": {}
}
```

### Activity

The recent-activity log shown on the Overview.

#### `activity`

The recent activity log, newest first.

Command line: `nexus activity [--json]`.

```sh
echo '{"v":1,"id":1,"op":"activity"}' | socat -t1 - UNIX-CONNECT:"$SOCK" | jq .
```

```json
{
  "v": 1,
  "id": 1,
  "ok": {
    "entries": [
      {
        "at": 1794265055,
        "kind": "vpn_up",
        "text": "VPN Work is up"
      }
    ]
  }
}
```

### Secret prompts

A join that needs a password the request did not carry opens a Secret prompt: the `secret_prompt_opened` event tells every subscribed Front end, and the first to answer or cancel wins. Both ops fail with `unknown_prompt` once the prompt is closed.

#### `secret_answer`

Answer an open Secret prompt (`prompt` is the id from its `secret_prompt_opened` event). The only request besides `wifi_join` that carries a secret. Result `{}`; `unknown_prompt` when it is closed already.

Command line: the masked prompt of nexus wifi join and nexus vpn up.

```sh
echo '{"v":1,"id":1,"op":"secret_answer","prompt":99,"secret":"hunter2"}' | socat -t1 - UNIX-CONNECT:"$SOCK" | jq .
```

```json
{
  "v": 1,
  "id": 1,
  "err": {
    "code": "unknown_prompt",
    "message": "That prompt is closed already (answered, cancelled or expired)."
  }
}
```

#### `secret_cancel`

Decline an open Secret prompt. The join waiting on it fails with `password_required`. Result `{}`; `unknown_prompt` when closed already.

Command line: an empty answer at that prompt.

```sh
echo '{"v":1,"id":1,"op":"secret_cancel","prompt":99}' | socat -t1 - UNIX-CONNECT:"$SOCK" | jq .
```

```json
{
  "v": 1,
  "id": 1,
  "err": {
    "code": "unknown_prompt",
    "message": "That prompt is closed already (answered, cancelled or expired)."
  }
}
```

### Ethernet

Ports, their Link details, and why a plugged-in one does not work.

#### `ethernet_list`

The Ethernet ports, their Link details and why a plugged-in one does not work. Result: `EthernetList`.

Command line: `nexus ethernet list [--json]`.

```sh
echo '{"v":1,"id":1,"op":"ethernet_list"}' | socat -t1 - UNIX-CONNECT:"$SOCK" | jq .
```

```json
{
  "v": 1,
  "id": 1,
  "ok": {
    "ports": [
      {
        "id": "enp3s0",
        "mac": "52:54:00:00:00:03",
        "speed_mbps": 1000,
        "state": "disconnected"
      }
    ]
  }
}
```

#### `ethernet_connect`

Bring an Ethernet port up with its saved Profile (a plain DHCP Profile is made and saved when it has none), waiting until it is up or has failed. `port` may be left out when the machine has one port. Result: `EthernetJoined`; `unplugged`, `unknown_port`, `wrong_password` (the port refused the 802.1X sign-in), `no_address`, `join_failed`.

Command line: `nexus ethernet connect [port]`.

```sh
echo '{"v":1,"id":1,"op":"ethernet_connect","port":"enp3s0"}' | socat -t1 - UNIX-CONNECT:"$SOCK" | jq .
```

```json
{
  "v": 1,
  "id": 1,
  "ok": {
    "port": "enp3s0",
    "profile": "4e086de9-1f3a-439a-9335-5a16b596902f"
  }
}
```

#### `ethernet_disconnect`

Drop an Ethernet Link, keeping its Profile. Result: `EthernetDropped`; `not_connected`, `unknown_port`.

Command line: `nexus ethernet disconnect [port]`.

```sh
echo '{"v":1,"id":1,"op":"ethernet_disconnect","port":"enp3s0"}' | socat -t1 - UNIX-CONNECT:"$SOCK" | jq .
```

```json
{
  "v": 1,
  "id": 1,
  "ok": {
    "port": "enp3s0"
  }
}
```

#### `ethernet_join_eap`

Sign in to an 802.1X (wired Enterprise Network) port with the settings in `eap`: the port's Profile gets them (one is made when there is none) and is brought up. Like `ethernet_connect` otherwise; `invalid_settings` when `eap` fails its validation.

Command line: `nexus ethernet connect [port] --eap METHOD ...`.

```sh
echo '{"v":1,"id":1,"eap":{"domain_suffix":"radius.office.example","identity":"alice@office.example","method":"peap","password":"hunter2","phase2":"mschapv2"},"op":"ethernet_join_eap","port":"enp3s0"}' | socat -t1 - UNIX-CONNECT:"$SOCK" | jq .
```

```json
{
  "v": 1,
  "id": 1,
  "ok": {
    "port": "enp3s0",
    "profile": "4e086de9-1f3a-439a-9335-5a16b596902f"
  }
}
```

### VPN

WireGuard and plugin VPN Profiles. A VPN is named by its `id` from `vpn_list`; the examples below show `WORK_VPN_ID`, which you replace with the real one.

#### `vpn_list`

The VPN Profiles and their state. Result: `VpnList`.

Command line: `nexus vpn list [--json]`.

```sh
echo '{"v":1,"id":1,"op":"vpn_list"}' | socat -t1 - UNIX-CONNECT:"$SOCK" | jq .
```

```json
{
  "v": 1,
  "id": 1,
  "ok": {
    "vpns": [
      {
        "addresses": [
          "10.9.0.2/32"
        ],
        "endpoint": "lab.example.org:51820",
        "id": "b87a9cef-ad68-4a8b-8944-d1b0f6ae098f",
        "kind": "wireguard",
        "name": "Home Lab",
        "state": "down"
      }
    ]
  }
}
```

#### `vpn_down`

Bring a VPN down. Result: `VpnToggled`; `unknown_vpn`, `not_connected` when it is not up.

Command line: `nexus vpn down <name>`.

```sh
echo '{"v":1,"id":1,"op":"vpn_down","vpn":"WORK_VPN_ID"}' | socat -t1 - UNIX-CONNECT:"$SOCK" | jq .
```

```json
{
  "v": 1,
  "id": 1,
  "ok": {
    "id": "1bbdded0-b780-4866-9281-57d42b3d79d5",
    "name": "Work",
    "state": "down"
  }
}
```

#### `vpn_up`

Bring a VPN up, waiting until it is up or has failed (`vpn` is a `Vpn.id`). Result: `VpnToggled`; `unknown_vpn`, `vpn_failed`.

Command line: `nexus vpn up <name>`.

```sh
echo '{"v":1,"id":1,"op":"vpn_up","vpn":"WORK_VPN_ID"}' | socat -t1 - UNIX-CONNECT:"$SOCK" | jq .
```

```json
{
  "v": 1,
  "id": 1,
  "ok": {
    "id": "1bbdded0-b780-4866-9281-57d42b3d79d5",
    "name": "Work",
    "state": "up"
  }
}
```

#### `vpn_key`

The public key of a WireGuard VPN Profile. Result: `VpnKey`.

Command line: `nexus vpn key <name>`.

```sh
echo '{"v":1,"id":1,"op":"vpn_key","vpn":"WORK_VPN_ID"}' | socat -t1 - UNIX-CONNECT:"$SOCK" | jq .
```

```json
{
  "v": 1,
  "id": 1,
  "ok": {
    "name": "Work",
    "public_key": "HIgo9xNzJMWLKASShiTqIybxZ0U3wGLiUeJ1PKf8ykw="
  }
}
```

#### `vpn_reveal`

Reveal a WireGuard Profile's whole config, private key included, so it can be shown as a QR code. Needs the Backend's authorization. Result: `VpnShare`; `reveal_denied` when refused. Front ends call it only after the person pressed a reveal key.

Command line: `nexus vpn share <name> [--qr]`.

```sh
echo '{"v":1,"id":1,"op":"vpn_reveal","vpn":"WORK_VPN_ID"}' | socat -t1 - UNIX-CONNECT:"$SOCK" | jq .
```

```json
{
  "v": 1,
  "id": 1,
  "ok": {
    "config": "[Interface]\nPrivateKey = yAnz5TF+lXXJte14tji3zlMNq+hd2rYUIgJBgB3fBmk=\nAddress = 10.8.0.2/32\nDNS = 10.8.0.1\n\n[Peer]\nPublicKey = xTIBA5rboUvnH4htodjb6e697QjLERt1NAB4mZqp8Dg=\nAllowedIPs = 0.0.0.0/0, ::/0\nEndpoint = vpn.work.example:51820\nPersistentKeepalive = 25\n",
    "name": "Work",
    "public_key": "HIgo9xNzJMWLKASShiTqIybxZ0U3wGLiUeJ1PKf8ykw="
  }
}
```

#### `vpn_import`

Import a WireGuard `.conf` (its text, private key included) as a VPN Profile. Result: `VpnAdded`; `invalid_config` with the line at fault.

Command line: `nexus vpn import <file>`.

```sh
echo '{"v":1,"id":1,"config":"[Interface]\nPrivateKey = yAnz5TF+lXXJte14tji3zlMNq+hd2rYUIgJBgB3fBmk=\nAddress = 10.8.1.2/32\n\n[Peer]\nPublicKey = xTIBA5rboUvnH4htodjb6e697QjLERt1NAB4mZqp8Dg=\nAllowedIPs = 10.8.1.0/24\nEndpoint = vpn.branch.example:51820\n","name":"Branch","op":"vpn_import"}' | socat -t1 - UNIX-CONNECT:"$SOCK" | jq .
```

```json
{
  "v": 1,
  "id": 1,
  "ok": {
    "id": "22b4224e-e618-4fa0-a61b-81242ead0bfd",
    "name": "Branch",
    "public_key": "HIgo9xNzJMWLKASShiTqIybxZ0U3wGLiUeJ1PKf8ykw="
  }
}
```

#### `vpn_create`

Create a WireGuard VPN Profile from scratch with a fresh key pair. Result: `VpnAdded` (carries the public key); `invalid_config`.

Command line: `nexus vpn create <name> --address ...`.

```sh
echo '{"v":1,"id":1,"addresses":["10.9.1.2/32"],"name":"Lab","op":"vpn_create"}' | socat -t1 - UNIX-CONNECT:"$SOCK" | jq .
```

```json
{
  "v": 1,
  "id": 1,
  "ok": {
    "id": "1f205a6f-1cb1-4e75-a67d-39a1245e0c91",
    "name": "Lab",
    "public_key": "3rg8wDEGOc64SDh4LooxoMn35kn9pBTwwhwgaK29Ano="
  }
}
```

### Tethering

Get online through a paired phone over Bluetooth.

#### `tethers`

Paired phones that offer network access over Bluetooth, and whether Bluetooth is on. Result: `Tethers`.

Command line: `nexus tether list [--json]`.

```sh
echo '{"v":1,"id":1,"op":"tethers"}' | socat -t1 - UNIX-CONNECT:"$SOCK" | jq .
```

```json
{
  "v": 1,
  "id": 1,
  "ok": {
    "bluetooth": "on",
    "phones": [
      {
        "connected": false,
        "connecting": false,
        "id": "AA:BB:CC:DD:EE:01",
        "name": "Pixel 8",
        "saved": false
      }
    ]
  }
}
```

#### `tether_connect`

Tether through a phone, waiting until the Link is up or has failed. `phone` is its Bluetooth address or name; it may be left out when only one phone is paired. Result: `Tethered`; `bluetooth_off`, `no_phone`, `unknown_phone` or `join_failed` on failure.

Command line: `nexus tether connect [phone]`.

```sh
echo '{"v":1,"id":1,"op":"tether_connect","phone":"Pixel 8"}' | socat -t1 - UNIX-CONNECT:"$SOCK" | jq .
```

```json
{
  "v": 1,
  "id": 1,
  "ok": {
    "id": "AA:BB:CC:DD:EE:01",
    "phone": "Pixel 8"
  }
}
```

#### `tether_disconnect`

Drop the Tethering Link, keeping the phone paired. Result: `Untethered`; `not_connected` when nothing is tethered.

Command line: `nexus tether disconnect`.

```sh
echo '{"v":1,"id":1,"op":"tether_disconnect"}' | socat -t1 - UNIX-CONNECT:"$SOCK" | jq .
```

```json
{
  "v": 1,
  "id": 1,
  "ok": {
    "phone": "Pixel 8"
  }
}
```

### Hotspot

Share this machine's connection as a Wi-Fi Hotspot.

#### `hotspot`

Whether this machine can host a Hotspot, and the Hotspot that runs. Result: `Hotspot`.

Command line: `nexus hotspot status [--json]`.

```sh
echo '{"v":1,"id":1,"op":"hotspot"}' | socat -t1 - UNIX-CONNECT:"$SOCK" | jq .
```

```json
{
  "v": 1,
  "id": 1,
  "ok": {
    "available": true,
    "on": false,
    "starting": false
  }
}
```

#### `hotspot_start`

Start a Hotspot sharing the internet connection, waiting until it is up or has failed. Without `ssid` and `password` Nexus makes both up (a strong password). Result: `HotspotStarted`; `hotspot_unsupported` (the card or the radio cannot host one), `join_failed`, `invalid_settings` (the name or password is not usable).

Command line: `nexus hotspot start`.

```sh
echo '{"v":1,"id":1,"op":"hotspot_start","password":"open sesame 42","ssid":"Garage"}' | socat -t1 - UNIX-CONNECT:"$SOCK" | jq .
```

```json
{
  "v": 1,
  "id": 1,
  "ok": {
    "generated": false,
    "password": "open sesame 42",
    "ssid": "Garage"
  }
}
```

#### `hotspot_reveal`

The join details of the running Hotspot, for a QR code. Result: `WifiShare`; `not_connected` when none runs, `reveal_denied` when authorization was refused. Front ends call it only after the person pressed a reveal key.

Command line: `nexus hotspot qr`.

```sh
echo '{"v":1,"id":1,"op":"hotspot_reveal"}' | socat -t1 - UNIX-CONNECT:"$SOCK" | jq .
```

```json
{
  "v": 1,
  "id": 1,
  "ok": {
    "auth": "wpa",
    "password": "open sesame 42",
    "ssid": "Garage"
  }
}
```

#### `hotspot_stop`

Stop the Hotspot and forget its Profile. Result: `HotspotStopped`; `not_connected` when none runs.

Command line: `nexus hotspot stop`.

```sh
echo '{"v":1,"id":1,"op":"hotspot_stop"}' | socat -t1 - UNIX-CONNECT:"$SOCK" | jq .
```

```json
{
  "v": 1,
  "id": 1,
  "ok": {
    "ssid": "Garage"
  }
}
```

### Portal

Captive-portal logins.

#### `portal`

The Portal the Link is behind, if any. Result: `PortalState`.

Command line: `nexus portal [status]`.

```sh
echo '{"v":1,"id":1,"op":"portal"}' | socat -t1 - UNIX-CONNECT:"$SOCK" | jq .
```

```json
{
  "v": 1,
  "id": 1,
  "ok": {
    "portal": null
  }
}
```

#### `portal_open`

Open the Portal's login page in the user's browser (the Daemon runs `xdg-open`). Result: `PortalOpened`; `no_portal` when no Network is asking for a login right now; `open_failed` when no browser started.

Command line: `nexus portal open`.

```sh
echo '{"v":1,"id":1,"op":"portal_open"}' | socat -t1 - UNIX-CONNECT:"$SOCK" | jq .
```

```json
{
  "v": 1,
  "id": 1,
  "err": {
    "code": "no_portal",
    "message": "No network is asking for a login right now."
  }
}
```

### Doctor

Why the machine is or is not online, and the fixes.

#### `doctor`

Run the Doctor's checks. Result: `DoctorReport`, in check order (sort by `severity` to lead with the worst). Contacts nothing outside beyond the configured connectivity check's host name lookup.

Command line: `nexus doctor [--json]`.

```sh
echo '{"v":1,"id":1,"op":"doctor"}' | socat -t1 - UNIX-CONNECT:"$SOCK" | jq .
```

```json
{
  "v": 1,
  "id": 1,
  "ok": {
    "findings": [
      {
        "cause": "NetworkManager manages your connections, which is what Nexus drives.",
        "id": "networkmanager",
        "severity": "ok",
        "title": "NetworkManager 1.58.1 is running"
      }
    ]
  }
}
```

#### `doctor_fix`

Run the fix of one finding (`finding` is a `Finding.id`) through `pkexec` when it needs privileges. Refused with `not_confirmed` unless `confirm` is true: a Front end sets it only after the person said yes. Result: `DoctorFixed`; `unknown_finding` when the check has nothing to fix any more; `fix_failed` when a command failed (the message says which).

Command line: `nexus doctor --fix`.

```sh
echo '{"v":1,"id":1,"confirm":true,"finding":"no-such-finding","op":"doctor_fix"}' | socat -t1 - UNIX-CONNECT:"$SOCK" | jq .
```

```json
{
  "v": 1,
  "id": 1,
  "err": {
    "code": "unknown_finding",
    "message": "There is nothing to fix for `no-such-finding` right now."
  }
}
```

### Config and events

The effective config, and the event stream.

#### `config_get`

Read the effective config, or one value by dotted key (`theme`, `connectivity.url`). The result is `{"value": ..., "warning"?: ...}`; `warning` is present while the file on disk is being ignored. While the Backend reports the address it probes for connectivity (NetworkManager's `ConnectivityCheckUri`), the result also carries `connectivity_uri`: read-only, set in `NetworkManager.conf`, not by Nexus. `connectivity.enabled` switches the Backend's own check (NetworkManager's `ConnectivityCheckEnabled`, which NetworkManager remembers) on or off when it changes; `connectivity.url` is only what the Doctor looks up and the Portal login opens.

Command line: `nexus config get [key] [--json]`.

```sh
echo '{"v":1,"id":1,"key":"theme","op":"config_get"}' | socat -t1 - UNIX-CONNECT:"$SOCK" | jq .
```

```json
{
  "v": 1,
  "id": 1,
  "ok": {
    "value": "midnight-bloom"
  }
}
```

#### `config_set`

Set one value by dotted key, editing `config.toml` in place so the user's comments and ordering survive. The result is as for `config_get`.

Command line: `nexus config set <key> <value>`.

```sh
echo '{"v":1,"id":1,"key":"theme","op":"config_set","value":"nord"}' | socat -t1 - UNIX-CONNECT:"$SOCK" | jq .
```

```json
{
  "v": 1,
  "id": 1,
  "ok": {
    "value": "nord"
  }
}
```

#### `subscribe`

Start receiving event lines on this connection. Answered with `{"ok": {}}`; events follow, interleaved with later responses.

Command line: `nexus status --waybar --follow (and the TUI)`.

```sh
(echo '{"v":1,"id":1,"op":"subscribe"}'; sleep 600) | socat - UNIX-CONNECT:"$SOCK" | jq -c .
```

```json
{
  "v": 1,
  "id": 1,
  "ok": {}
}
```

## Events

After `subscribe` the connection also receives event lines, interleaved with the responses to later requests. An event carries `v`, `event` and its payload. Events are pushed after a change settles (a burst of Backend changes yields one event per Daemon tick), so a Front end can keep its picture current without polling.

A subscriber that stops reading is not buffered forever: the Daemon queues up to 1024 lines per connection, and a connection that falls further behind is closed. A Front end that sees the stream end re-connects, re-subscribes and re-reads what it shows.

### `status_changed`

The overall state or the primary Link changed. Carries the whole new status (what `status` would answer).

```json event
{
  "v": 1,
  "event": "status_changed",
  "status": {
    "backend": {
      "name": "networkmanager",
      "version": "1.58.1"
    },
    "connectivity": "full",
    "link": {
      "kind": "wifi",
      "name": "Home-5G"
    },
    "radios": {
      "airplane": false,
      "wifi": {
        "enabled": true,
        "hardware_blocked": false
      }
    },
    "state": "online"
  }
}
```

### `networks_changed`

The Wi-Fi list changed. Carries the whole new list; a burst of Backend changes yields one event per Daemon tick.

```json event
{
  "v": 1,
  "event": "networks_changed",
  "wifi": {
    "devices": [
      {
        "id": "/org/freedesktop/NetworkManager/Devices/0",
        "name": "wlan0"
      }
    ],
    "networks": [
      {
        "access_points": 1,
        "autoconnect": true,
        "band": "5",
        "device": "/org/freedesktop/NetworkManager/Devices/0",
        "id": "486f6d652d3547",
        "joined": true,
        "profile": "731c9f8c-e411-4629-816c-e74b33c6fdb8",
        "security": "wpa2",
        "signal": 82,
        "ssid": "Home-5G",
        "trust": "trusted"
      }
    ],
    "saved": [
      {
        "autoconnect": true,
        "id": "Parents House",
        "ssid": "Parents House",
        "uuid": "uuid-parents"
      }
    ]
  }
}
```

### `activity`

Something worth a line in the activity strip just happened.

```json event
{
  "v": 1,
  "entry": {
    "at": 1794265055,
    "kind": "joined",
    "text": "Joined Home-5G (Wi-Fi)"
  },
  "event": "activity"
}
```

### `config_reloaded`

`config.toml` changed (or was fixed, or broke). `config` is what is now in force; `warning` says why the file is being ignored, if it is.

```json event
{
  "v": 1,
  "config": {
    "connectivity": {
      "enabled": true,
      "url": "https://ping.archlinux.org/nm-check.txt"
    },
    "doctor": {
      "gateway": "auto"
    },
    "hotspot": {
      "poll_secs": 5
    },
    "menu": {
      "launcher": null
    },
    "notifications": {
      "joined": true,
      "lost": true,
      "policy": true,
      "portal": true,
      "secret_prompt": true,
      "vpn_dropped": true,
      "wrong_password": true
    },
    "prompts": {
      "timeout_secs": 120
    },
    "theme": "nord",
    "themes": {},
    "trust": {
      "auto_vpn": "",
      "metered": false
    },
    "ui": {
      "nerd_font": false,
      "reduce_motion": false
    }
  },
  "event": "config_reloaded"
}
```

### `secret_prompt_opened`

The Backend needs a secret from the user. Prompts still open when a Front end subscribes are replayed to it, so one that was not running when the prompt opened still sees it.

```json event
{
  "v": 1,
  "event": "secret_prompt_opened",
  "prompt": {
    "expires_in": 120,
    "field": "password",
    "id": 1,
    "kind": "wifi_password",
    "message": "Password for Work-Secure",
    "network": "Work-Secure",
    "network_id": "576f726b2d536563757265",
    "retry": false
  }
}
```

### `secret_prompt_closed`

A Secret prompt is gone: answer it no more.

```json event
{
  "v": 1,
  "id": 1,
  "event": "secret_prompt_closed",
  "reason": "cancelled"
}
```

### `doctor_findings`

The Doctor's findings changed since it last ran (any Front end asked it to run). Carries all of them.

```json event
{
  "v": 1,
  "event": "doctor_findings",
  "findings": [
    {
      "cause": "NetworkManager manages your connections, which is what Nexus drives.",
      "id": "networkmanager",
      "severity": "ok",
      "title": "NetworkManager 1.58.1 is running"
    }
  ]
}
```

### `vpns_changed`

The VPN list changed (a Profile added, a VPN up or down).

```json event
{
  "v": 1,
  "event": "vpns_changed",
  "vpns": {
    "vpns": [
      {
        "addresses": [
          "10.9.0.2/32"
        ],
        "endpoint": "lab.example.org:51820",
        "id": "b87a9cef-ad68-4a8b-8944-d1b0f6ae098f",
        "kind": "wireguard",
        "name": "Home Lab",
        "state": "down"
      }
    ]
  }
}
```

### `tethers_changed`

The Tethering picture changed (Bluetooth switched, a phone paired, tethered or dropped). Carries the whole new picture.

```json event
{
  "v": 1,
  "event": "tethers_changed",
  "tethers": {
    "bluetooth": "on",
    "phones": [
      {
        "connected": false,
        "connecting": false,
        "id": "AA:BB:CC:DD:EE:01",
        "name": "Pixel 8",
        "saved": false
      }
    ]
  }
}
```

### `hotspot_changed`

The Hotspot started, stopped or its device count changed.

```json event
{
  "v": 1,
  "event": "hotspot_changed",
  "hotspot": {
    "available": true,
    "on": true,
    "ssid": "Garage",
    "starting": true
  }
}
```

### `portal_changed`

The Network is now behind a Portal (`Some`), or no longer is (`None`: signed in, or the Link went away).

```json event
{
  "v": 1,
  "event": "portal_changed",
  "portal": {
    "network": "CafeGuest",
    "url": "http://ping.archlinux.org/nm-check.txt"
  }
}
```

### `ethernet_changed`

The Ethernet picture changed (a cable plugged or pulled, a port joined, dropped or failed). Carries the whole new picture.

```json event
{
  "v": 1,
  "ethernet": {
    "ports": [
      {
        "id": "enp3s0",
        "mac": "52:54:00:00:00:03",
        "speed_mbps": 1000,
        "state": "disconnected"
      }
    ]
  },
  "event": "ethernet_changed"
}
```

### `profile_pending`

A change to the Profile carrying the Link waits for confirmation: it rolls back in `rollback_secs` unless somebody confirms it.

```json event
{
  "v": 1,
  "change": {
    "id": 1,
    "rollback_secs": 30,
    "ssid": "eduroam"
  },
  "event": "profile_pending"
}
```

### `profile_settled`

A pending change was confirmed or rolled back.

```json event
{
  "v": 1,
  "id": 1,
  "event": "profile_settled",
  "outcome": "confirmed",
  "ssid": "eduroam"
}
```

## Errors

A failed request answers `{"v":1,"id":N,"err":{"code":"...","message":"..."}}`. The `code` is stable and meant for programs; the `message` is a sentence for people and may change. Codes a client does not know decode as `unknown`: treat them as a plain failure. Each code has a CLI exit code (see [Exit codes](#exit-codes)).

| Code | Exit | Meaning |
|---|---|---|
| `bad_request` | 4 | The line is not JSON, or not a request this daemon can read (a missing or wrongly typed field). The message says what. |
| `unsupported_version` | 4 | The request's `v` is not one the daemon speaks; the message lists them. |
| `unknown_op` | 4 | No op has that name. |
| `internal` | 4 | The daemon failed in a way that is not the request's fault (a NetworkManager read that errored, say); the message says what. |
| `backend_unavailable` | 4 | The Backend (NetworkManager) is not running or not reachable. |
| `wrong_password` | 5 | The password was rejected. |
| `out_of_range` | 6 | The Network is not (or no longer) in range. |
| `no_address` | 7 | Joined the Network but never got an address. |
| `join_failed` | 8 | The join failed for another reason; the message says which. |
| `password_required` | 10 | The Network needs a password and none was given. |
| `unknown_network` | 9 | No Network by that name is saved or nearby. |
| `not_connected` | 11 | There is no Wi-Fi Link to disconnect. |
| `hard_blocked` | 12 | A hardware switch (not software) holds the radio off. |
| `unknown_prompt` | 4 | No Secret prompt with that id is open (it was answered, cancelled or expired). |
| `not_confirmed` | 4 | A fix needs `confirm: true` and did not get it. |
| `unknown_finding` | 4 | No finding with that id has a fix to run. |
| `fix_failed` | 4 | A fix command failed or was not authorized. |
| `bluetooth_off` | 18 | Bluetooth is switched off, or this machine has none. |
| `no_phone` | 19 | No paired phone offers network access. |
| `unknown_phone` | 20 | No paired phone matches the name given, or several do and none was named. |
| `no_portal` | 33 | No Network is asking for a login right now. |
| `open_failed` | 34 | The browser could not be started. |
| `invalid_settings` | 15 | Settings are not valid (Profile settings, 802.1X sign-in); the message says which and why. |
| `conflict` | 16 | The Profile changed since it was read (or a change still waits for confirmation); read it again. |
| `nothing_pending` | 17 | No change is waiting for confirmation under that id. |
| `reveal_denied` | 14 | The Backend refused to hand over a stored secret (authorization denied). |
| `unknown_vpn` | 21 | No VPN Profile has that id. |
| `invalid_config` | 22 | A WireGuard config could not be read; the message names the line. |
| `vpn_failed` | 23 | The VPN did not come up; the message says why. |
| `unplugged` | 27 | No cable is plugged into the Ethernet port. |
| `unknown_port` | 28 | No Ethernet port has that name (or several exist and none was named). |
| `missing_plugin` | 24 | A plugin VPN's NetworkManager plugin is not installed; the message names the package. |
| `keyring_unavailable` | 25 | No Secret Service (keyring) answers, or it is locked or refused: a Profile's secrets cannot be moved into or out of it. |
| `unknown_device` | 32 | No Wi-Fi Device has that name or id. |
| `hotspot_unsupported` | 26 | This machine's Wi-Fi cannot host a Hotspot; the message says why. |

An error needs no subscription and never closes the connection. Examples:

A line that is not JSON:

```sh
echo 'not json' | socat -t1 - UNIX-CONNECT:"$SOCK" | jq .
```

```json
{
  "v": 1,
  "err": {
    "code": "bad_request",
    "message": "expected ident at line 1 column 2"
  }
}
```

A protocol version the daemon does not speak:

```sh
echo '{"v":2,"id":1,"op":"status"}' | socat -t1 - UNIX-CONNECT:"$SOCK" | jq .
```

```json
{
  "v": 1,
  "id": 1,
  "err": {
    "code": "unsupported_version",
    "message": "protocol version 2 is not supported (supported: [1])"
  }
}
```

An op the daemon does not know:

```sh
echo '{"v":1,"id":1,"op":"teleport"}' | socat -t1 - UNIX-CONNECT:"$SOCK" | jq .
```

```json
{
  "v": 1,
  "id": 1,
  "err": {
    "code": "unknown_op",
    "message": "unknown op `teleport`"
  }
}
```

A Network that is neither saved nor nearby:

```sh
echo '{"v":1,"id":1,"op":"wifi_forget","network":"4e6f7768657265"}' | socat -t1 - UNIX-CONNECT:"$SOCK" | jq .
```

```json
{
  "v": 1,
  "id": 1,
  "err": {
    "code": "unknown_network",
    "message": "No saved network called Nowhere."
  }
}
```

## Exit codes

Every `nexus` command exits with one of these. The man page (`man nexus`) carries the same table.

| Exit | Meaning |
|---|---|
| 0 | Success. |
| 1 | Another failure (the message says which). |
| 2 | Usage error: a flag or argument was not understood. |
| 3 | The Daemon cannot be reached. |
| 4 | The Daemon refused for a reason with no code of its own (backend_unavailable, bad_request, internal, ...). |
| 5 | wrong_password: the password was rejected. |
| 6 | out_of_range: the Network is not in range. |
| 7 | no_address: joined, but no address was handed out. |
| 8 | join_failed: the join failed; the message says why. |
| 9 | unknown_network: nothing is saved or nearby by that name. |
| 10 | password_required: the Network needs a password and none was given. |
| 11 | not_connected: there is nothing to disconnect or stop. |
| 12 | hard_blocked: a hardware switch holds the radio off. |
| 13 | `nexus doctor` found a problem. |
| 14 | reveal_denied: authorization to read a stored secret was refused. |
| 15 | invalid_settings: the settings are not valid. |
| 16 | conflict: the Profile changed since it was read. |
| 17 | nothing_pending: no change is waiting for confirmation. |
| 18 | bluetooth_off: Bluetooth is off or missing. |
| 19 | no_phone: no paired phone offers network access. |
| 20 | unknown_phone: no paired phone matches, or several do. |
| 21 | unknown_vpn: no VPN has that name. |
| 22 | invalid_config: a VPN config could not be read. |
| 23 | vpn_failed: the VPN did not come up. |
| 24 | missing_plugin: the VPN's NetworkManager plugin is not installed. |
| 25 | keyring_unavailable: no Secret Service answers, or it is locked. |
| 26 | hotspot_unsupported: this Wi-Fi card cannot host a Hotspot. |
| 27 | unplugged: no cable in the Ethernet port. |
| 28 | unknown_port: no Ethernet port by that name. |
| 29 | `nexus menu`: no launcher found. |
| 30 | `nexus menu`: the launcher would not run. |
| 31 | `nexus menu`: the launcher named in the config is not one Nexus knows. |
| 32 | unknown_device: no Wi-Fi device by that name. |
| 33 | no_portal: no Network is asking for a login. |
| 34 | open_failed: the browser could not be started. |
