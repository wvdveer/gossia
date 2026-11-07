#!/bin/bash
# Readies a fresh GitHub-hosted Ubuntu runner for the build
# (.github/workflows/ci.yml): the libraries the application links against,
# the build folder, and the toolchain pinned in rust-toolchain.toml.
set -euo pipefail

# The runner's one disk is nearly full of toolkits the build never uses
# (.NET, Android, Haskell, CodeQL), and the test binaries need the room.
sudo rm -rf /usr/share/dotnet /usr/local/lib/android /opt/ghc \
    /usr/local/.ghcup /opt/hostedtoolcache/CodeQL /usr/local/share/boost

sudo apt-get update -qq
sudo apt-get install -y -qq --no-install-recommends \
    pkg-config libssl-dev libfontconfig1-dev libxkbcommon-dev libwayland-dev \
    libx11-dev libxcursor-dev libxrandr-dev libxi-dev libgl1-mesa-dev

sudo mkdir -p "${CARGO_TARGET_DIR:-/mnt/target}"
sudo chown "$(id -u):$(id -g)" "${CARGO_TARGET_DIR:-/mnt/target}"

rustup show active-toolchain || rustup toolchain install
