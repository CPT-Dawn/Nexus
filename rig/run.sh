#!/usr/bin/env bash
# Real-mode rig (ticket #32): virtual Wi-Fi radios (mac80211_hwsim), hostapd
# access points and a private NetworkManager, then the real-mode S1 tests.
#
#   rig/run.sh [test-name-filter]        (run as your user; it uses sudo)
#
# Everything the tests touch lives in throwaway namespaces: a station
# namespace (own network, mounts, pids) holding NetworkManager, wpa_supplicant
# and a private dbus-daemon, and an AP namespace holding hostapd and dnsmasq.
# The host's network, bus and /etc are never used. See docs/testing.md.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(dirname "$here")"

die() { echo "rig: $*" >&2; exit 1; }

[[ "$(uname -s)" == Linux ]] || die "Linux only"
export PATH="$PATH:/usr/sbin:/sbin"
for tool in unshare iw ip nsenter modprobe hostapd dnsmasq wpa_supplicant NetworkManager dbus-daemon; do
    command -v "$tool" >/dev/null 2>&1 || die "missing tool: $tool (see docs/testing.md)"
done

# Refuse anything that could touch live radios or a live NetworkManager.
if [[ -d /sys/module/mac80211_hwsim ]]; then
    die "mac80211_hwsim is already loaded; unload it (sudo rmmod mac80211_hwsim) so the rig can own its radios"
fi
if [[ "${NEXUS_RIG_ALLOW_HOST_NM:-0}" != 1 ]] && command -v systemctl >/dev/null 2>&1 \
    && systemctl is-active --quiet NetworkManager 2>/dev/null; then
    die "NetworkManager is running on this host. New virtual radios may briefly appear to it before the rig moves them away (harmless: it has no profile for them). Re-run with NEXUS_RIG_ALLOW_HOST_NM=1 to accept that. Nothing else of the host is touched."
fi

# Build the tests as the calling user, then do the privileged part.
log="$(cd "$repo" && CARGO_TERM_COLOR=never cargo test -p nexus --test real_mode --no-run --locked 2>&1)" || { echo "$log" >&2; die "build failed"; }
testbin="$(sed -n 's/.*Executable .*(\(.*\)).*/\1/p' <<<"$log" | tail -n1)"
[[ -n "$testbin" ]] || die "could not find the test binary"
testbin="$(cd "$repo" && realpath "$testbin")"

if [[ $EUID -eq 0 ]]; then
    exec bash "$here/host.sh" "$testbin" "$@"
fi
exec sudo --preserve-env=RUST_LOG,RUST_BACKTRACE env "PATH=$PATH" bash "$here/host.sh" "$testbin" "$@"
