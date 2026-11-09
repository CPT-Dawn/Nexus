#!/usr/bin/env bash
# Fill a PKGBUILD.in and write PKGBUILD + .SRCINFO into an output directory.
# Usage: packaging/aur/render.sh <nexus-nm|nexus-nm-bin> <pkgver> <out-dir>
# Checksums come from the environment:
#   nexus-nm:     SHA256 (source tarball of the tag)
#   nexus-nm-bin: SHA256_X86_64, SHA256_AARCH64 (release tarballs)
# makepkg refuses to run as root, so run this as an ordinary user.
set -euo pipefail

pkg=${1:?package}
pkgver=${2:?pkgver}
out=${3:?out dir}
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)

case "$pkgver" in
  *-*) echo "pkgver '$pkgver' is a prerelease; the AUR only gets final releases" >&2; exit 1 ;;
esac

sed_args=(-e "s|@PKGVER@|$pkgver|g")
case "$pkg" in
  nexus-nm)
    sed_args+=(-e "s|@SHA256@|${SHA256:?SHA256 is required}|g") ;;
  nexus-nm-bin)
    sed_args+=(-e "s|@SHA256_X86_64@|${SHA256_X86_64:?SHA256_X86_64 is required}|g"
               -e "s|@SHA256_AARCH64@|${SHA256_AARCH64:?SHA256_AARCH64 is required}|g") ;;
  *) echo "unknown package: $pkg" >&2; exit 1 ;;
esac

mkdir -p "$out"
sed "${sed_args[@]}" "$here/$pkg/PKGBUILD.in" > "$out/PKGBUILD"
(cd "$out" && makepkg --printsrcinfo > .SRCINFO)
