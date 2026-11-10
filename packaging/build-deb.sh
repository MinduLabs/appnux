#!/usr/bin/env bash
# Builds dist/appnux_<version>_amd64.deb from a release build. Needs dpkg-deb.
set -euo pipefail
cd "$(dirname "$0")/.."
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
cargo build --release -p appnux

stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
chmod 755 "$stage"
install -Dm755 target/release/appnux "$stage/usr/bin/appnux"
install -Dm644 packaging/linux/io.github.mindulabs.Appnux.desktop "$stage/usr/share/applications/io.github.mindulabs.Appnux.desktop"
install -Dm644 packaging/linux/io.github.mindulabs.appnux.policy "$stage/usr/share/polkit-1/actions/io.github.mindulabs.appnux.policy"
install -Dm644 NOTICE "$stage/usr/share/doc/appnux/copyright"
install -Dm644 packaging/linux/appnux.svg "$stage/usr/share/icons/hicolor/scalable/apps/io.github.mindulabs.Appnux.svg"
install -Dm644 packaging/linux/icons/32x32.png "$stage/usr/share/icons/hicolor/32x32/apps/io.github.mindulabs.Appnux.png"
install -Dm644 packaging/linux/icons/128x128.png "$stage/usr/share/icons/hicolor/128x128/apps/io.github.mindulabs.Appnux.png"
install -Dm644 packaging/linux/icons/128x128@2x.png "$stage/usr/share/icons/hicolor/256x256/apps/io.github.mindulabs.Appnux.png"

size=$(du -sk "$stage/usr" | cut -f1)
mkdir -p "$stage/DEBIAN"
cat > "$stage/DEBIAN/control" <<CONTROL
Package: appnux
Version: $version
Section: utils
Priority: optional
Architecture: amd64
Installed-Size: $size
Depends: libgtk-4-1 (>= 4.6), libadwaita-1-0 (>= 1.1), libc6 (>= 2.35), pkexec | policykit-1
Maintainer: Appnux contributors <noreply@appnux.invalid>
Homepage: https://github.com/MinduLabs/Appnux
Description: See, uninstall and clean every app on Linux
 One window for system packages, Flatpak, Snap and AppImage apps: what each
 takes on disk, when it was last used, and a safe cleaner for caches.
CONTROL

mkdir -p dist
out="dist/appnux_${version}_amd64.deb"
dpkg-deb --root-owner-group --build "$stage" "$out" >/dev/null
echo "Built: $(pwd)/$out"
