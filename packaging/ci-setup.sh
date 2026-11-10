#!/usr/bin/env bash
# Prepares a bare ubuntu:22.04 container (the oldest base Appnux supports) to build and check
# the app: system packages, then the Rust toolchain from rust-toolchain.toml.
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq --no-install-recommends \
  build-essential pkg-config curl ca-certificates git file \
  libgtk-4-dev libadwaita-1-dev \
  libglib2.0-bin libgdk-pixbuf2.0-bin librsvg2-common >/dev/null
# rustup reads the channel and components (rustfmt, clippy) from rust-toolchain.toml.
curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain none
echo "$HOME/.cargo/bin" >> "${GITHUB_PATH:-/dev/null}"
# With no name, `toolchain install` installs what rust-toolchain.toml asks for.
"$HOME/.cargo/bin/rustup" toolchain install
"$HOME/.cargo/bin/rustup" show active-toolchain
