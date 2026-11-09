#!/usr/bin/env bash
# Tests for nexus-switch.sh against a fake root and stub systemctl/pacman.
# Run: scripts/test-nexus-switch.sh   (never touches the real system)
set -u
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
SCRIPT=$HERE/nexus-switch.sh
fails=0
total=0

write_systemctl_stub() {
  cat >"$1" <<'S'
#!/usr/bin/env bash
echo "systemctl $*" >>"$STUB_LOG"
verb=$1
shift
case $verb in
  is-enabled)
    f=$STUB_STATE/$1.enabled
    if [ -f "$f" ]; then cat "$f"; [ "$(cat "$f")" = enabled ]; else echo not-found; exit 1; fi ;;
  is-active)
    f=$STUB_STATE/$1.active
    if [ -f "$f" ]; then cat "$f"; [ "$(cat "$f")" = active ]; else echo inactive; exit 3; fi ;;
  enable | disable | start | stop)
    now=0
    units=()
    for a in "$@"; do
      case $a in --now) now=1 ;; -*) ;; *) units+=("$a") ;; esac
    done
    stops=0
    if [ $now = 1 ] || [ "$verb" = stop ]; then
      case $verb in disable | stop) stops=1 ;; esac
    fi
    if [ $stops = 1 ] && printf '%s\n' "${units[@]}" | grep -qx 'systemd-networkd'; then
      for trig in systemd-networkd.socket systemd-networkd-varlink.socket systemd-networkd-varlink-metrics.socket systemd-networkd-resolve-hook.socket; do
        if [ "$(cat "$STUB_STATE/$trig.active" 2>/dev/null)" = active ] && ! printf '%s\n' "${units[@]}" | grep -qx "$trig"; then
          for u in "${units[@]}"; do [ "$verb" = disable ] && echo disabled >"$STUB_STATE/$u.enabled"; done
          echo "Job for $trig canceled." >&2
          exit 1
        fi
      done
    fi
    for u in "${units[@]}"; do
      case $verb in
        enable) echo enabled >"$STUB_STATE/$u.enabled" ;;
        disable) echo disabled >"$STUB_STATE/$u.enabled" ;;
      esac
      if [ $now = 1 ] || [ "$verb" = start ] || [ "$verb" = stop ]; then
        case $verb in
          enable | start) echo active >"$STUB_STATE/$u.active" ;;
          disable | stop) echo inactive >"$STUB_STATE/$u.active" ;;
        esac
      fi
    done ;;
esac
exit 0
S
}

write_pacman_stub() {
  cat >"$1" <<'S'
#!/usr/bin/env bash
echo "pacman $*" >>"$STUB_LOG"
case $1 in
  -Q) [ -f "$STUB_STATE/pkg.$2" ] ;;
  -S*) for a in "$@"; do case $a in -*) ;; *) touch "$STUB_STATE/pkg.$a" ;; esac; done ;;
esac
exit 0
S
}

setup() {
  T=$(mktemp -d)
  export NEXUS_SWITCH_ROOT=$T/root
  mkdir -p "$T/bin" "$NEXUS_SWITCH_ROOT/etc/systemd/network" "$NEXUS_SWITCH_ROOT/etc/iwd"
  printf '[Match]\nName=enp110s0\n[Network]\nDHCP=yes\n' >"$NEXUS_SWITCH_ROOT/etc/systemd/network/20-wired.network"
  printf '[General]\nEnableNetworkConfiguration=true\n' >"$NEXUS_SWITCH_ROOT/etc/iwd/main.conf"
  ln -s ../run/systemd/resolve/stub-resolv.conf "$NEXUS_SWITCH_ROOT/etc/resolv.conf"
  export STUB_STATE=$T/units STUB_LOG=$T/calls.log
  mkdir -p "$STUB_STATE"
  echo enabled >"$STUB_STATE/systemd-networkd.enabled"
  echo active >"$STUB_STATE/systemd-networkd.active"
  for trig in systemd-networkd.socket systemd-networkd-varlink.socket systemd-networkd-varlink-metrics.socket systemd-networkd-resolve-hook.socket; do
    echo enabled >"$STUB_STATE/$trig.enabled"
    echo active >"$STUB_STATE/$trig.active"
  done
  echo enabled >"$STUB_STATE/systemd-resolved.enabled"
  echo active >"$STUB_STATE/systemd-resolved.active"
  echo disabled >"$STUB_STATE/iwd.enabled"
  echo inactive >"$STUB_STATE/iwd.active"
  : >"$STUB_LOG"
  write_systemctl_stub "$T/bin/systemctl"
  write_pacman_stub "$T/bin/pacman"
  chmod +x "$T/bin/systemctl" "$T/bin/pacman"
  export PATH=$T/bin:$PATH
}
teardown() { rm -rf "$T"; }

# Everything under the fake root plus unit state, as one comparable string.
snapshot() {
  (
    cd "$NEXUS_SWITCH_ROOT" || exit 1
    find etc -type f -exec sha256sum {} + | sort
    find etc -type l -exec sh -c 'echo "$1 -> $(readlink "$1")"' _ {} \; | sort
    find etc -type d | sort
    grep -r . "$STUB_STATE" --include='*.enabled' --include='*.active' | sed "s|$STUB_STATE/||" | grep -v -e ':disabled$' -e ':inactive$' | sort
  )
}

check() {
  local name=$1
  shift
  total=$((total + 1))
  if "$@"; then echo "ok   $name"; else echo "FAIL $name"; fails=$((fails + 1)); fi
}

t_dry_run_changes_nothing() {
  setup
  local before after out rc ok=1
  before=$(snapshot)
  out=$("$SCRIPT" --dry-run 2>&1)
  rc=$?
  after=$(snapshot)
  [ $rc = 0 ] || ok=0
  [ "$before" = "$after" ] || ok=0
  grep -q 'pacman -S' <<<"$out" || ok=0
  grep -q 'wifi.backend=iwd' <<<"$out" || ok=0
  grep -q 'stop systemd-networkd.socket systemd-networkd-varlink.socket' <<<"$out" || ok=0
  grep -q 'EnableNetworkConfiguration=false' <<<"$out" || ok=0
  grep -q 'enable --now NetworkManager' <<<"$out" || ok=0
  if grep -qE '^(pacman -S|systemctl (enable|disable|start|stop))' "$STUB_LOG"; then ok=0; fi
  teardown
  [ $ok = 1 ]
}

t_switch_then_revert_restores() {
  setup
  local before ok=1 conf=
  before=$(snapshot)
  "$SCRIPT" --yes >/dev/null 2>&1 || ok=0
  conf=$NEXUS_SWITCH_ROOT/etc/NetworkManager/conf.d/10-nexus-iwd.conf
  grep -q 'wifi.backend=iwd' "$conf" || ok=0
  [ "$(cat "$STUB_STATE/NetworkManager.enabled")" = enabled ] || ok=0
  [ "$(cat "$STUB_STATE/iwd.enabled")" = enabled ] || ok=0
  [ "$(cat "$STUB_STATE/systemd-networkd.enabled")" = disabled ] || ok=0
  [ "$(cat "$STUB_STATE/systemd-networkd.active")" = inactive ] || ok=0
  [ -f "$NEXUS_SWITCH_ROOT/etc/systemd/network/20-wired.network" ] || ok=0
  grep -q 'interface-name=enp110s0' "$NEXUS_SWITCH_ROOT"/etc/NetworkManager/system-connections/*.nmconnection || ok=0
  grep -q '^EnableNetworkConfiguration=false' "$NEXUS_SWITCH_ROOT/etc/iwd/main.conf" || ok=0
  [ "$(grep -c '^EnableNetworkConfiguration=' "$NEXUS_SWITCH_ROOT/etc/iwd/main.conf")" = 1 ] || ok=0
  [ "$(cat "$STUB_STATE/systemd-networkd-varlink.socket.active")" = inactive ] || ok=0
  [ "$(cat "$STUB_STATE/NetworkManager.active")" = active ] || ok=0
  "$SCRIPT" --revert --yes >/dev/null 2>&1 || ok=0
  [ "$before" = "$(snapshot)" ] || ok=0
  [ "$(cat "$STUB_STATE/NetworkManager.enabled")" = disabled ] || ok=0
  teardown
  [ $ok = 1 ]
}

t_refuses_partial_state_without_confirmation() {
  setup
  local out rc ok=1
  echo enabled >"$STUB_STATE/NetworkManager.enabled"
  echo active >"$STUB_STATE/NetworkManager.active"
  out=$("$SCRIPT" </dev/null 2>&1)
  rc=$?
  [ $rc != 0 ] || ok=0
  grep -qi 'partial' <<<"$out" || ok=0
  if grep -qE '^(pacman -S|systemctl (enable|disable))' "$STUB_LOG"; then ok=0; fi
  "$SCRIPT" --yes >/dev/null 2>&1 || ok=0
  teardown
  [ $ok = 1 ]
}

t_revert_without_backup_fails() {
  setup
  local rc
  "$SCRIPT" --revert --yes >/dev/null 2>&1
  rc=$?
  teardown
  [ $rc != 0 ]
}

t_second_switch_refused() {
  setup
  local ok=1
  "$SCRIPT" --yes >/dev/null 2>&1 || ok=0
  "$SCRIPT" </dev/null >/dev/null 2>&1 && ok=0
  teardown
  [ $ok = 1 ]
}

# A switch interrupted after the backup (the 2026-11-10 failure: networkd's
# varlink sockets cancelled the stop job) can be finished with --resume.
t_resume_finishes_an_interrupted_switch() {
  setup
  local ok=1
  mkdir -p "$T/realbin"
  cp "$T/bin/systemctl" "$T/realbin/systemctl"
  cat >"$T/bin/systemctl" <<S
#!/usr/bin/env bash
case "\$*" in *NetworkManager*) [ "\$1" = enable ] && exit 1 ;; esac
exec "$T/realbin/systemctl" "\$@"
S
  "$SCRIPT" --yes >/dev/null 2>&1 && ok=0
  cp "$T/realbin/systemctl" "$T/bin/systemctl"
  [ "$(cat "$STUB_STATE/NetworkManager.enabled" 2>/dev/null)" = enabled ] && ok=0
  "$SCRIPT" --resume --yes >/dev/null 2>&1 || ok=0
  [ "$(cat "$STUB_STATE/NetworkManager.enabled")" = enabled ] || ok=0
  [ "$(cat "$STUB_STATE/NetworkManager.active")" = active ] || ok=0
  [ "$(cat "$STUB_STATE/systemd-networkd.active")" = inactive ] || ok=0
  grep -q '^EnableNetworkConfiguration=false' "$NEXUS_SWITCH_ROOT/etc/iwd/main.conf" || ok=0
  "$SCRIPT" --revert --yes >/dev/null 2>&1 || ok=0
  grep -q '^EnableNetworkConfiguration=true' "$NEXUS_SWITCH_ROOT/etc/iwd/main.conf" || ok=0
  [ "$(cat "$STUB_STATE/systemd-networkd.active")" = active ] || ok=0
  teardown
  [ $ok = 1 ]
}

t_resume_without_backup_fails() {
  setup
  local rc
  "$SCRIPT" --resume --yes >/dev/null 2>&1
  rc=$?
  teardown
  [ $rc != 0 ]
}

t_real_root_needs_root() {
  [ "$(id -u)" = 0 ] && return 0
  setup
  local out rc
  unset NEXUS_SWITCH_ROOT
  out=$("$SCRIPT" --yes 2>&1)
  rc=$?
  teardown
  [ $rc != 0 ] && grep -qi 'root' <<<"$out"
}

# Real use leaves NEXUS_SWITCH_ROOT empty; the revert must still reach the end.
t_real_root_revert_dry_run() {
  setup
  local out rc
  unset NEXUS_SWITCH_ROOT
  mkdir -p "$T/backup"
  : >"$T/backup/units"
  out=$(NEXUS_SWITCH_BACKUP=$T/backup "$SCRIPT" --dry-run --revert 2>&1)
  rc=$?
  teardown
  [ $rc = 0 ] && grep -q '^+ rm -rf /etc/iwd$' <<<"$out" && grep -q '== Done' <<<"$out"
}

# Static networkd config: every Address=/DNS= line counts, "DNS=a b" is two
# servers, IPv6 values belong under [ipv6], and the Gateway goes on address1.
t_static_wired_translation() {
  setup
  local ok=1 prof
  printf '%s\n' '[Match]' 'Name=enp5s0' '[Network]' 'DHCP=no' \
    'Address=192.168.1.10/24' 'Address=192.168.1.11/24' 'Address=fd00::10/64' \
    'Gateway=192.168.1.1' 'Gateway=fd00::1' \
    'DNS=1.1.1.1 9.9.9.9' 'DNS=2606:4700:4700::1111#cloudflare-dns.com' \
    >"$NEXUS_SWITCH_ROOT/etc/systemd/network/30-static.network"
  "$SCRIPT" --yes >/dev/null 2>&1 || ok=0
  prof=$NEXUS_SWITCH_ROOT/etc/NetworkManager/system-connections/nexus-wired-enp5s0.nmconnection
  [ "$(sed -n '/^\[ipv4\]/,/^\[ipv6\]/p' "$prof")" = "$(printf '%s\n' '[ipv4]' 'method=manual' \
    'address1=192.168.1.10/24,192.168.1.1' 'address2=192.168.1.11/24' 'dns=1.1.1.1;9.9.9.9;' '' '[ipv6]')" ] || ok=0
  [ "$(sed -n '/^\[ipv6\]/,$p' "$prof")" = "$(printf '%s\n' '[ipv6]' 'method=manual' \
    'address1=fd00::10/64,fd00::1' 'dns=2606:4700:4700::1111;')" ] || ok=0
  [ "$(stat -c %a "$prof")" = 600 ] || ok=0
  teardown
  [ $ok = 1 ]
}

# A backup interrupted before it finished is redone from scratch on the next run:
# no copies nested inside half-written directories, and revert restores the original.
t_interrupted_backup_reruns_cleanly() {
  setup
  local ok=1 b
  before=$(snapshot)
  b=$NEXUS_SWITCH_ROOT/var/lib/nexus-switch
  mkdir -p "$b/files/etc/iwd"
  echo stale >"$b/files/etc/iwd/main.conf"
  "$SCRIPT" --yes >/dev/null 2>&1 || ok=0
  [ ! -e "$b/files/etc/iwd/iwd" ] || ok=0
  [ ! -e "$b/files/etc/systemd/network/network" ] || ok=0
  grep -q '^EnableNetworkConfiguration=true' "$b/files/etc/iwd/main.conf" || ok=0
  "$SCRIPT" --revert --yes >/dev/null 2>&1 || ok=0
  grep -q '^EnableNetworkConfiguration=true' "$NEXUS_SWITCH_ROOT/etc/iwd/main.conf" || ok=0
  teardown
  [ $ok = 1 ]
}

check "dry-run prints every step and changes nothing" t_dry_run_changes_nothing
check "switch then revert restores previous state exactly" t_switch_then_revert_restores
check "refuses partially configured state without confirmation" t_refuses_partial_state_without_confirmation
check "revert without a backup fails" t_revert_without_backup_fails
check "second switch refused without confirmation" t_second_switch_refused
check "resume finishes an interrupted switch" t_resume_finishes_an_interrupted_switch
check "resume without a backup fails" t_resume_without_backup_fails
check "refuses to touch the real system unless root" t_real_root_needs_root
check "revert dry-run works on the real root" t_real_root_revert_dry_run
check "static wired config keeps every address and DNS server" t_static_wired_translation
check "an interrupted backup reruns without nesting copies" t_interrupted_backup_reruns_cleanly
echo "$((total - fails))/$total passed"
[ $fails = 0 ]
