#!/bin/bash
# Readies a fresh GitHub-hosted Ubuntu runner for the build
# (.github/workflows/ci.yml): the libraries the application links against,
# the build folder on the second disk, and the toolchain pinned in
# rust-toolchain.toml.
set -euo pipefail

sudo apt-get update -qq
sudo apt-get install -y -qq --no-install-recommends \
    pkg-config libssl-dev libfontconfig1-dev libxkbcommon-dev libwayland-dev \
    libx11-dev libxcursor-dev libxrandr-dev libxi-dev libgl1-mesa-dev

sudo mkdir -p "${CARGO_TARGET_DIR:-/mnt/target}"
sudo chown "$(id -u):$(id -g)" "${CARGO_TARGET_DIR:-/mnt/target}"

rustup show active-toolchain || rustup toolchain install
