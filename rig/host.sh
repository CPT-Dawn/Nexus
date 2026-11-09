#!/usr/bin/env bash
# Root, host namespace: load the virtual radios, hand them to two fresh
# namespaces, run inner.sh in one of them, and clean everything up.
# Called by rig/run.sh; do not run directly.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
testbin="$1"; shift
[[ $EUID -eq 0 ]] || { echo "rig: host.sh needs root" >&2; exit 1; }

aps=("$here"/aps.d/*.conf)
radios=$(( ${#aps[@]} + 1 ))   # one station radio plus one per AP

work="$(mktemp -d /tmp/nexus-rig.XXXXXX)"
loaded=0 inner_pid="" ap_pid=""
cleanup() {
    set +e
    [[ -n "$inner_pid" ]] && kill -KILL "$inner_pid" 2>/dev/null   # pid namespace init: takes its children along
    [[ -n "$ap_pid" ]] && kill -KILL "$ap_pid" 2>/dev/null
    wait 2>/dev/null
    umount "$work/apns" 2>/dev/null
    (( loaded )) && rmmod mac80211_hwsim 2>/dev/null
    rm -rf "$work"
}
trap cleanup EXIT INT TERM

before="$(iw dev 2>/dev/null | awk '/^phy#/{print $1}' | sort || true)"
modprobe mac80211_hwsim radios="$radios" || {
    echo "rig: cannot load mac80211_hwsim (on Debian/Ubuntu: apt install linux-modules-extra-\$(uname -r))" >&2
    exit 1
}
loaded=1
# Only phys born from our modprobe are ever moved.
after="$(ls /sys/class/ieee80211 | sort)"
mapfile -t phys < <(comm -13 <(sed 's/^phy#/phy/' <<<"$before") <<<"$after")
(( ${#phys[@]} == radios )) || { echo "rig: expected $radios new radios, got ${#phys[@]}" >&2; exit 1; }

# AP namespace: a bare network namespace kept alive by a sleeper.
touch "$work/apns"
unshare --net="$work/apns" sleep infinity &
ap_pid=$!
# Station namespace: its init (pid 1 of a new pid namespace) waits for "go".
mkfifo "$work/go"
unshare --mount --net --pid --fork --kill-child --mount-proc --propagation private \
    bash "$here/inner.sh" "$work" "$testbin" "$@" &
inner_pid=$!

# The sleeper and the unshare parent are already in their new network namespaces.
for _ in $(seq 50); do
    [[ "$(readlink /proc/$ap_pid/ns/net)" != "$(readlink /proc/$$/ns/net)" ]] && break
    sleep 0.1
done
iw phy "${phys[0]}" set netns "$inner_pid"
for p in "${phys[@]:1}"; do iw phy "$p" set netns "$ap_pid"; done
echo go >"$work/go"

status=0
wait "$inner_pid" || status=$?
inner_pid=""
exit "$status"
