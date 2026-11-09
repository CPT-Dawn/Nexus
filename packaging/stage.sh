#!/usr/bin/env bash
# Lay out a /usr tree for Nexus from a built binary.
# Usage: packaging/stage.sh <path-to-nexus-binary> <dest-dir>
# Completions and the man page come from the binary itself.
set -euo pipefail

bin=${1:?usage: stage.sh <nexus-binary> <dest-dir>}
dest=${2:?usage: stage.sh <nexus-binary> <dest-dir>}
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)

install -Dm755 "$bin" "$dest/usr/bin/nexus"
install -Dm644 "$here/systemd/nexus.service" "$dest/usr/lib/systemd/user/nexus.service"
install -Dm644 "$here/systemd/nexus.socket" "$dest/usr/lib/systemd/user/nexus.socket"
install -Dm644 "$here/systemd/90-nexus.preset" "$dest/usr/lib/systemd/user-preset/90-nexus.preset"
install -Dm644 "$here/polkit/50-nexus-wheel.rules.example" \
  "$dest/usr/share/doc/nexus/examples/50-nexus-wheel.rules.example"

"$bin" completions bash | install -Dm644 /dev/stdin "$dest/usr/share/bash-completion/completions/nexus"
"$bin" completions zsh | install -Dm644 /dev/stdin "$dest/usr/share/zsh/site-functions/_nexus"
"$bin" completions fish | install -Dm644 /dev/stdin "$dest/usr/share/fish/vendor_completions.d/nexus.fish"
mkdir -p "$dest/usr/share/man/man1"
"$bin" man --dir "$dest/usr/share/man/man1"
chmod 644 "$dest"/usr/share/man/man1/*.1
