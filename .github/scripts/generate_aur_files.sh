#!/usr/bin/env bash
set -euo pipefail

if [[ -z "${GITHUB_REF_NAME:-}" ]]; then
  echo "GITHUB_REF_NAME is required (example: v1.2.3)." >&2
  exit 1
fi

release_tag="${GITHUB_REF_NAME}"
if [[ "$release_tag" != v* ]]; then
  echo "Release tag must start with 'v' (received: $release_tag)." >&2
  exit 1
fi

pkgname="nexus-nm"
pkgver="${release_tag#v}"
pkgrel="${PKGREL:-1}"
maintainer="${AUR_MAINTAINER:-CPT-Dawn <dawnsp0456@gmail.com>}"
repo="${GITHUB_REPOSITORY:-CPT-Dawn/Nexus}"
server_url="${GITHUB_SERVER_URL:-https://github.com}"

tarball_url="${server_url}/${repo}/archive/refs/tags/${release_tag}.tar.gz"
source_name="${pkgname}-${pkgver}.tar.gz"

tmp_tarball="$(mktemp)"
trap 'rm -f "$tmp_tarball"' EXIT

curl -fsSL "$tarball_url" -o "$tmp_tarball"
sha256="$(sha256sum "$tmp_tarball" | awk '{print $1}')"

cat > PKGBUILD <<EOF
# Maintainer: ${maintainer}
pkgname=${pkgname}
pkgver=${pkgver}
pkgrel=${pkgrel}
pkgdesc="A beautiful modern TUI WiFi manager for Arch Linux"
arch=('x86_64' 'aarch64')
url="${server_url}/${repo}"
license=('MIT')
depends=('glibc' 'networkmanager' 'dbus')
makedepends=('cargo' 'rust')
source=("${source_name}::${tarball_url}")
sha256sums=('${sha256}')

build() {
  cd "Nexus-${pkgver}"
  export CARGO_TARGET_DIR="\${srcdir}/target"
  cargo build --release --locked
}

package() {
  cd "Nexus-${pkgver}"
  install -Dm755 "\${srcdir}/target/release/nexus" "\${pkgdir}/usr/bin/nexus"
  install -Dm644 LICENSE "\${pkgdir}/usr/share/licenses/${pkgname}/LICENSE"
  install -Dm644 README.md "\${pkgdir}/usr/share/doc/${pkgname}/README.md"
}
EOF

cat > .SRCINFO <<EOF
pkgbase = ${pkgname}
  pkgdesc = A beautiful modern TUI WiFi manager for Arch Linux
  pkgver = ${pkgver}
  pkgrel = ${pkgrel}
  url = ${server_url}/${repo}
  arch = x86_64
  arch = aarch64
  license = MIT
  makedepends = cargo
  makedepends = rust
  depends = glibc
  depends = networkmanager
  depends = dbus
  source = ${source_name}::${tarball_url}
  sha256sums = ${sha256}

pkgname = ${pkgname}
EOF