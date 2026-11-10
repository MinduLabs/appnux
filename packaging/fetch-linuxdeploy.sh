#!/usr/bin/env bash
# Downloads linuxdeploy, its GTK plugin and its AppImage plugin into $LINUXDEPLOY_DIR
# (default ~/.cache/appnux-build), the folder packaging/build-appimage.sh reads.
set -euo pipefail
dir=${LINUXDEPLOY_DIR:-$HOME/.cache/appnux-build}
mkdir -p "$dir"
fetch() {
  # GitHub's download servers sometimes answer 5xx; try a few times.
  for attempt in 1 2 3 4 5; do
    curl -fsSL -o "$dir/$1" "$2" && chmod +x "$dir/$1" && return 0
    echo "retrying $1 ($attempt)"; sleep $((attempt * 5))
  done
  return 1
}
fetch linuxdeploy-x86_64.AppImage https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-x86_64.AppImage
fetch linuxdeploy-plugin-appimage-x86_64.AppImage https://github.com/linuxdeploy/linuxdeploy-plugin-appimage/releases/download/continuous/linuxdeploy-plugin-appimage-x86_64.AppImage
fetch linuxdeploy-plugin-gtk.sh https://raw.githubusercontent.com/linuxdeploy/linuxdeploy-plugin-gtk/master/linuxdeploy-plugin-gtk.sh
echo "Tools in $dir"
