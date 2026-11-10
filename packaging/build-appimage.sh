#!/usr/bin/env bash
# Builds dist/Appnux-<version>-x86_64.AppImage with linuxdeploy and its GTK plugin, which
# bundles GTK 4 and libadwaita. Tools are looked up in $LINUXDEPLOY_DIR (default
# ~/.cache/appnux-build); fetch them once from github.com/linuxdeploy if missing.
set -euo pipefail
cd "$(dirname "$0")/.."
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
tools=${LINUXDEPLOY_DIR:-$HOME/.cache/appnux-build}
for tool in linuxdeploy-x86_64.AppImage linuxdeploy-plugin-gtk.sh linuxdeploy-plugin-appimage-x86_64.AppImage; do
  [ -e "$tools/$tool" ] || { echo "Missing $tools/$tool (see the comment at the top of this script)"; exit 1; }
done
cargo build --release -p appnux

appdir=$(mktemp -d)/Appnux.AppDir
trap 'rm -rf "$(dirname "$appdir")"' EXIT
mkdir -p dist
export PATH="$tools:$PATH" DEPLOY_GTK_VERSION=4 OUTPUT="dist/Appnux-${version}-x86_64.AppImage" ARCH=x86_64
"$tools/linuxdeploy-x86_64.AppImage" --appdir "$appdir" \
  --executable target/release/appnux \
  --desktop-file packaging/linux/io.github.mindulabs.Appnux.desktop \
  --icon-file packaging/linux/icons/128x128@2x.png --icon-filename io.github.mindulabs.Appnux \
  --plugin gtk >/dev/null
# The GTK plugin forces GTK_THEME=Adwaita, which replaces libadwaita's own stylesheet with
# GTK's built-in theme: boxed lists, dim labels and row spacing all lose their style.
sed -i '/^export GTK_THEME=/d' "$appdir/apprun-hooks/linuxdeploy-plugin-gtk.sh"
"$tools/linuxdeploy-x86_64.AppImage" --appdir "$appdir" --output appimage >/dev/null
echo "Built: $(pwd)/$OUTPUT"
