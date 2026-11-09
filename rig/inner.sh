#!/usr/bin/env bash
# Root, inside fresh mount+net+pid namespaces (started by rig/host.sh).
# Pid 1 here: when it exits, the kernel kills everything it started.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
work="$1"; testbin="$2"; shift 2
export PATH="$PATH:/usr/sbin:/sbin"

dump_logs() {
    for f in /run/nexus-rig/*.log; do [ -e "$f" ] || continue; echo "::group::rig log $(basename "$f")"; tail -n 150 "$f"; echo "::endgroup::"; done
}
die() { echo "rig(inner): $*" >&2; dump_logs >&2; exit 1; }

# Wait until the host has moved the radios in.
read -r _ <"$work/go"

# /sys shows the namespace it was mounted in: remount it for this one.
mount -t sysfs sysfs /sys

# Safety: this must be an isolated world. Only loopback and wireless
# interfaces may exist; a wired interface means we are on the host.
for dev in /sys/class/net/*; do
    name="$(basename "$dev")"
    [[ "$name" == lo || -d "$dev/phy80211" ]] || die "found non-wireless interface $name: refusing to run against the host's network"
done
[[ -n "$(ls /sys/class/net | grep -v '^lo$' || true)" ]] || die "no virtual radio arrived"

# Private /run, /var/lib and /etc/NetworkManager: nothing of the host's survives.
mount -t tmpfs tmpfs /run
mount -t tmpfs tmpfs /var/lib
mount -t tmpfs tmpfs /etc/NetworkManager 2>/dev/null || { mkdir -p /etc/NetworkManager && mount -t tmpfs tmpfs /etc/NetworkManager; }
mkdir -p /run/nexus-rig /run/NetworkManager /run/wpa_supplicant /var/lib/NetworkManager /etc/NetworkManager/system-connections
chmod 700 /etc/NetworkManager/system-connections
ip link set lo up

# ---- private system bus --------------------------------------------------
bus_dir=/run/nexus-rig
cat >"$bus_dir/bus.conf" <<'EOF'
<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:path=/run/nexus-rig/bus</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
EOF
dbus-daemon --nofork --nopidfile --config-file="$bus_dir/bus.conf" >"$bus_dir/dbus.log" 2>&1 &
for _ in $(seq 100); do [[ -S $bus_dir/bus ]] && break; sleep 0.05; done
[[ -S $bus_dir/bus ]] || die "private bus never came up"
export DBUS_SYSTEM_BUS_ADDRESS="unix:path=$bus_dir/bus"
unset DBUS_SESSION_BUS_ADDRESS

# ---- access points, in their own namespace -------------------------------
apns="$work/apns"
[[ "$(stat -f -c %T "$apns")" == nsfs ]] || die "$apns is not a network namespace"
in_ap() { nsenter --net="$apns" "$@"; }
in_ap ip link set lo up
mapfile -t ap_ifaces < <(in_ap iw dev | awk '$1 == "Interface" {print $2}' | sort)
confs=("$here"/aps.d/*.conf)
(( ${#ap_ifaces[@]} == ${#confs[@]} )) || die "have ${#ap_ifaces[@]} AP radios for ${#confs[@]} AP configs"

dnsmasq_args=(--keep-in-foreground --conf-file=/dev/null --port=0 --bind-interfaces
    --dhcp-leasefile="$bus_dir/leases" --pid-file="$bus_dir/dnsmasq.pid" --dhcp-authoritative)
i=0
for conf in "${confs[@]}"; do
    i=$((i + 1))
    iface="${ap_ifaces[$((i - 1))]}"
    in_ap ip addr add "10.99.$i.1/24" dev "$iface"
    sed "s/@IFACE@/$iface/" "$conf" >"$bus_dir/$(basename "$conf")"
    in_ap hostapd "$bus_dir/$(basename "$conf")" >"$bus_dir/hostapd-$(basename "$conf" .conf).log" 2>&1 &
    dnsmasq_args+=(--interface="$iface" "--dhcp-range=10.99.$i.10,10.99.$i.50,12h")
done
in_ap dnsmasq "${dnsmasq_args[@]}" >"$bus_dir/dnsmasq.log" 2>&1 &

# ---- station side: wpa_supplicant + NetworkManager -----------------------
cat >/etc/NetworkManager/NetworkManager.conf <<EOF
[main]
plugins=keyfile
dhcp=internal
dns=none
rc-manager=unmanaged
no-auto-default=*
auth-polkit=false
[device]
wifi.backend=wpa_supplicant
wifi.scan-rand-mac-address=no
[logging]
level=${NEXUS_RIG_NM_LOG:-INFO}
EOF
# NetworkManager leaves a device unmanaged until udev has initialised it, and
# the host's udev never saw this namespace's devices: run a private udevd.
udevd=""
for c in /usr/lib/systemd/systemd-udevd /lib/systemd/systemd-udevd "$(command -v udevd || true)"; do
    [[ -n "$c" && -x "$c" ]] && { udevd="$c"; break; }
done
[[ -n "$udevd" ]] || die "systemd-udevd not found"
mkdir -p /run/udev
"$udevd" >"$bus_dir/udevd.log" 2>&1 &
for _ in $(seq 100); do [[ -S /run/udev/control ]] && break; sleep 0.05; done
udevadm trigger --action=add --subsystem-match=net --subsystem-match=ieee80211
udevadm settle --timeout=20

nm_up() {
    dbus-send --bus="$DBUS_SYSTEM_BUS_ADDRESS" --print-reply --dest=org.freedesktop.DBus /org/freedesktop/DBus \
        org.freedesktop.DBus.NameHasOwner string:org.freedesktop.NetworkManager 2>/dev/null | grep -q 'boolean true'
}
wpa_supplicant -u -O /run/wpa_supplicant >"$bus_dir/wpa_supplicant.log" 2>&1 &
NetworkManager --no-daemon --debug >"$bus_dir/networkmanager.log" 2>&1 &
nm_pid=$!
for _ in $(seq 200); do
    nm_up && break
    sleep 0.1
done
nm_up \
    || { kill -0 "$nm_pid" 2>/dev/null && echo "NetworkManager still running" >&2 || { wait "$nm_pid"; echo "NetworkManager exited with $?" >&2; }; die "NetworkManager never claimed its name"; }

# ---- run the tests -------------------------------------------------------
status=0
NEXUS_RIG=1 NEXUS_RIG_BUS="$DBUS_SYSTEM_BUS_ADDRESS" "$testbin" --test-threads=1 "$@" || status=$?
(( status == 0 )) || dump_logs
exit "$status"
