#!/usr/bin/env bash
# nexus-switch.sh: move this machine from systemd-networkd to NetworkManager
# (with iwd as its Wi-Fi backend), and back. Meant for dogfooding Nexus v2.
#
#   sudo scripts/nexus-switch.sh --dry-run    # print every step, change nothing
#   sudo scripts/nexus-switch.sh              # switch (backs up first)
#   sudo scripts/nexus-switch.sh --revert     # restore the backed-up state
#   sudo scripts/nexus-switch.sh --resume     # finish a switch that stopped half way
#
# Options: --dry-run  --revert  --resume  --yes (skip confirmation)  -h/--help
# Test hooks: NEXUS_SWITCH_ROOT (prefix for every config path; also lifts the
# root requirement), NEXUS_SWITCH_BACKUP (backup dir). systemctl and pacman are
# looked up on PATH, so tests substitute stubs.
set -euo pipefail

ROOT=${NEXUS_SWITCH_ROOT:-}
BACKUP=${NEXUS_SWITCH_BACKUP:-$ROOT/var/lib/nexus-switch}
DRY=0
REVERT=0
RESUME=0
YES=0

# Paths (relative to ROOT) that a switch may change; backed up and restored whole.
TRACKED=(
  etc/systemd/network
  etc/systemd/resolved.conf
  etc/systemd/resolved.conf.d
  etc/resolv.conf
  etc/iwd
  etc/NetworkManager
)
# Everything that can start networkd again. They are stopped in one transaction:
# stopping the service alone lets a trigger socket restart it, and systemd then
# cancels the stop job ("Job for systemd-networkd.socket canceled").
NETWORKD_UNITS=(
  systemd-networkd.socket
  systemd-networkd-varlink.socket
  systemd-networkd-varlink-metrics.socket
  systemd-networkd-resolve-hook.socket
  systemd-networkd-wait-online.service
  systemd-networkd
)
# Units whose enabled/active state is recorded and restored.
UNITS=("${NETWORKD_UNITS[@]}" systemd-resolved iwd NetworkManager)

NM_CONF=$ROOT/etc/NetworkManager/conf.d/10-nexus-iwd.conf
NM_CONN_DIR=$ROOT/etc/NetworkManager/system-connections
IWD_CONF=$ROOT/etc/iwd/main.conf

say() { printf '%s\n' "$*"; }
die() {
  printf 'nexus-switch: %s\n' "$*" >&2
  exit 1
}

usage() { sed -n '2,12p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; }

# Run a mutating command, or only print it under --dry-run.
run() {
  if [ "$DRY" = 1 ]; then
    say "+ $*"
  else
    "$@"
  fi
}

# Write stdin to a file (mode 600 for connection profiles), or print it.
write_file() { # path mode
  local path=$1 mode=$2 content
  content=$(cat)
  if [ "$DRY" = 1 ]; then
    say "+ write $path (mode $mode)"
    printf '%s\n' "$content" | sed 's/^/|   /'
  else
    mkdir -p "$(dirname "$path")"
    printf '%s\n' "$content" >"$path"
    chmod "$mode" "$path"
  fi
}

unit_enabled() { # prints enabled|disabled
  local s
  s=$(systemctl is-enabled "$1" 2>/dev/null || true)
  [ "$s" = enabled ] && echo enabled || echo disabled
}
unit_active() { # prints active|inactive
  local s
  s=$(systemctl is-active "$1" 2>/dev/null || true)
  [ "$s" = active ] && echo active || echo inactive
}

confirm() { # message
  [ "$YES" = 1 ] && return 0
  [ "$DRY" = 1 ] && return 0
  [ -t 0 ] || die "$1 Re-run with --yes to proceed anyway."
  local ans
  read -r -p "$1 Continue? [y/N] " ans
  case $ans in y | Y | yes) ;; *) die "aborted." ;; esac
}

require_root() {
  [ "$DRY" = 1 ] && return 0
  [ -n "$ROOT" ] && return 0
  [ "$(id -u)" = 0 ] || die "must run as root (try: sudo $0 ...). Use --dry-run to preview."
}

detect_partial() { # prints reasons, one per line
  [ "$(unit_enabled NetworkManager)" = enabled ] && say "NetworkManager is already enabled"
  [ "$(unit_active NetworkManager)" = active ] && say "NetworkManager is already running"
  [ -e "$NM_CONF" ] && say "$NM_CONF already exists"
  return 0
}

backup_state() {
  say "== Backing up current state to $BACKUP"
  # No "units" file yet means an earlier backup was interrupted. Start it over:
  # copying into half-written directories would nest copies inside them.
  run rm -rf "${BACKUP:?}/files" "${BACKUP:?}/absent"
  run mkdir -p "$BACKUP/files"
  local p units_tmp=""
  for p in "${TRACKED[@]}"; do
    if [ -e "$ROOT/$p" ] || [ -L "$ROOT/$p" ]; then
      run mkdir -p "$BACKUP/files/$(dirname "$p")"
      run cp -a -P "$ROOT/$p" "$BACKUP/files/$p"
    else
      say "  (absent: /$p)"
      if [ "$DRY" = 0 ]; then echo "$p" >>"$BACKUP/absent"; fi
    fi
  done
  local u
  for u in "${UNITS[@]}"; do
    units_tmp+="$u $(unit_enabled "$u") $(unit_active "$u")"$'\n'
  done
  if [ "$DRY" = 1 ]; then
    say "+ record unit states in $BACKUP/units:"
    printf '%s' "$units_tmp" | sed 's/^/|   /'
  else
    printf '%s' "$units_tmp" >"$BACKUP/units"
    : >>"$BACKUP/absent"
  fi
}

# Every value of a (possibly repeated, possibly space-separated) networkd key,
# one token per line. A trailing "#server-name" (DNS=) is dropped.
network_values() { # file key
  sed -n "s/^[[:space:]]*$2=//p" "$1" | tr -s ' \t\r' '\n' | sed 's/#.*$//' | sed '/^$/d'
}

# Print the [ipv4]/[ipv6] sections for one .network file (stdin: none).
# Static Address= lines become addressN= (the first Gateway of the family goes on
# address1), DNS= servers go to the section of their family as "dns=a;b;".
wired_ip_sections() { # file
  local f=$1 dhcp v token n
  local -a a4=() a6=() g4=() g6=() d4=() d6=()
  while read -r token; do
    [ -n "$token" ] || continue
    case $token in *:*) a6+=("$token") ;; *) a4+=("$token") ;; esac
  done < <(network_values "$f" Address)
  while read -r token; do
    [ -n "$token" ] || continue
    case $token in *:*) g6+=("$token") ;; *) g4+=("$token") ;; esac
  done < <(network_values "$f" Gateway)
  while read -r token; do
    [ -n "$token" ] || continue
    case $token in *:*) d6+=("$token") ;; *) d4+=("$token") ;; esac
  done < <(network_values "$f" DNS)
  dhcp=$(sed -n 's/^[[:space:]]*DHCP=//p' "$f" | head -n1)
  [ ${#a4[@]} -gt 0 ] || case $dhcp in yes | ipv4 | "") ;; *) say "  note: $f has DHCP=$dhcp; using DHCP4 in NM" >&2 ;; esac

  printf '[ipv4]\n'
  if [ ${#a4[@]} -gt 0 ]; then
    printf 'method=manual\n'
    n=0
    for v in "${a4[@]}"; do
      n=$((n + 1))
      printf 'address%d=%s%s\n' "$n" "$v" "$([ $n = 1 ] && [ ${#g4[@]} -gt 0 ] && echo ",${g4[0]}")"
    done
  else
    printf 'method=auto\n'
  fi
  if [ ${#d4[@]} -gt 0 ]; then printf 'dns=%s;\n' "$(IFS=';'; echo "${d4[*]}")"; fi

  printf '\n[ipv6]\n'
  if [ ${#a6[@]} -gt 0 ]; then
    printf 'method=manual\n'
    n=0
    for v in "${a6[@]}"; do
      n=$((n + 1))
      printf 'address%d=%s%s\n' "$n" "$v" "$([ $n = 1 ] && [ ${#g6[@]} -gt 0 ] && echo ",${g6[0]}")"
    done
  else
    printf 'method=auto\n'
  fi
  if [ ${#d6[@]} -gt 0 ]; then printf 'dns=%s;\n' "$(IFS=';'; echo "${d6[*]}")"; fi
}

# Translate the networkd .network files that match a named interface into NM profiles.
write_wired_profiles() {
  local dir=$ROOT/etc/systemd/network f name slug iface
  [ -d "$dir" ] || return 0
  for f in "$dir"/*.network; do
    [ -f "$f" ] || continue
    name=$(sed -n 's/^Name=\([A-Za-z0-9_.:*-]*\)$/\1/p' "$f" | head -n1)
    [ -n "$name" ] || { say "  skip $f (no plain Name= match)"; continue; }
    case $name in wl*) say "  skip $name (Wi-Fi is handled by iwd)"; continue ;; esac
    slug=${name//\*/any}
    iface="interface-name=$name"
    case $name in *\**) iface='' ;; esac
    {
      printf '[connection]\nid=nexus-wired-%s\ntype=ethernet\n%sautoconnect=true\n\n' "$slug" "${iface:+$iface$'\n'}"
      wired_ip_sections "$f"
    } | write_file "$NM_CONN_DIR/nexus-wired-$slug.nmconnection" 600
  done
}

# NetworkManager does IP configuration itself; iwd's own DHCP must be off or
# both run on the same link. /etc/iwd is backed up, so --revert restores it.
iwd_hand_over_ip() {
  local content='' has=0
  [ -f "$IWD_CONF" ] && content=$(cat "$IWD_CONF")
  grep -q '^EnableNetworkConfiguration=' <<<"$content" && has=1
  printf '%s\n' "$content" | awk -v has="$has" '
    /^EnableNetworkConfiguration=/ { print "EnableNetworkConfiguration=false"; next }
    { print }
    /^\[General\]$/ && !has { print "EnableNetworkConfiguration=false"; has = 1 }
    END { if (!has) printf "\n[General]\nEnableNetworkConfiguration=false\n" }
  ' | write_file "$IWD_CONF" 644
}

do_switch() {
  local reasons
  if [ "$RESUME" = 1 ]; then
    [ -e "$BACKUP/units" ] || die "no backup found at $BACKUP; nothing to resume. Run without --resume."
    say "== Resuming a switch (backup at $BACKUP is kept as it is)"
  else
    if [ -e "$BACKUP/units" ]; then
      die "already switched (backup exists at $BACKUP). Finish an interrupted switch with --resume, undo with --revert."
    fi
    reasons=$(detect_partial)
    if [ -n "$reasons" ]; then
      say "WARNING: partially configured state detected:"
      printf '%s\n' "$reasons" | sed 's/^/  - /'
      confirm "Machine looks partially configured."
    fi
    backup_state
  fi
  say "== Installing packages"
  run pacman -S --needed --noconfirm networkmanager iwd
  say "== Configuring NetworkManager (iwd backend, resolved for DNS)"
  write_file "$NM_CONF" 644 <<'EOF'
[main]
dns=systemd-resolved

[device]
wifi.backend=iwd
EOF
  say "== Preserving the wired setup as NetworkManager profiles"
  write_wired_profiles
  say "== Handing IP configuration from iwd to NetworkManager"
  iwd_hand_over_ip
  say "== Switching services (wired connectivity drops briefly)"
  run systemctl disable "${NETWORKD_UNITS[@]}"
  run systemctl stop "${NETWORKD_UNITS[@]}"
  run systemctl enable --now systemd-resolved
  run systemctl enable --now iwd
  run systemctl enable --now NetworkManager
  say "== Done. Undo with: $0 --revert"
}

do_revert() {
  [ -e "$BACKUP/units" ] || die "no backup found at $BACKUP; nothing to revert."
  say "== Reverting from $BACKUP"
  confirm "This restores the backed-up networkd/resolved/iwd/NetworkManager state."
  local u en ac p want_on=()
  while read -r u en ac; do
    [ -n "$u" ] || continue
    if [ "$en" = disabled ] && [ "$(unit_enabled "$u")" = enabled ]; then run systemctl disable --now "$u"; fi
    if [ "$ac" = inactive ] && [ "$(unit_active "$u")" = active ]; then run systemctl stop "$u"; fi
    if [ "$en" = enabled ]; then want_on+=("$u $en $ac"); fi
    if [ "$en" = disabled ] && [ "$ac" = active ]; then want_on+=("$u $en $ac"); fi
  done <"$BACKUP/units"
  say "== Restoring files"
  for p in "${TRACKED[@]}"; do
    run rm -rf "$ROOT/${p:?}"
    if [ -e "$BACKUP/files/$p" ] || [ -L "$BACKUP/files/$p" ]; then
      run mkdir -p "$ROOT/$(dirname "$p")"
      run cp -a -P "$BACKUP/files/$p" "$ROOT/$p"
    fi
  done
  say "== Restoring services"
  local entry
  for entry in "${want_on[@]}"; do
    read -r u en ac <<<"$entry"
    if [ "$en" = enabled ] && [ "$(unit_enabled "$u")" != enabled ]; then run systemctl enable "$u"; fi
    if [ "$ac" = active ] && [ "$(unit_active "$u")" != active ]; then run systemctl start "$u"; fi
  done
  run rm -rf "${BACKUP:?}"
  say "== Done. Packages were left installed:"
  say "   remove with 'pacman -Rns networkmanager' if you do not want them."
}

while [ $# -gt 0 ]; do
  case $1 in
    --dry-run) DRY=1 ;;
    --revert) REVERT=1 ;;
    --resume) RESUME=1 ;;
    --yes | -y) YES=1 ;;
    -h | --help) usage; exit 0 ;;
    *) die "unknown option: $1 (see --help)" ;;
  esac
  shift
done

require_root
[ "$DRY" = 1 ] && say "DRY RUN: nothing will be changed."
if [ "$REVERT" = 1 ]; then do_revert; else do_switch; fi
