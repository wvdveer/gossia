#!/bin/bash
# Builds the Windows installer (an x64 MSI) on Linux: cross-compiles the release
# application for x86_64-pc-windows-msvc with cargo-xwin, then packages it with wixl
# (msitools). Nothing is installed system-wide: the tools it needs and does not find
# are put under $CLIOTREE_MSI_TOOLS (default ~/.cache/cliotree-msi) and reused.
#
# The first run downloads the Microsoft CRT and Windows SDK (cargo-xwin, accepting
# their licence terms: XWIN_ACCEPT_LICENSE) and takes several minutes; later runs
# reuse the cache. The result is target/msi/ClioTree-<version>-x64.msi.
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
tools=${CLIOTREE_MSI_TOOLS:-$HOME/.cache/cliotree-msi}
target=x86_64-pc-windows-msvc
mkdir -p "$tools"

version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -1)
[ -n "$version" ] || { echo "no version in Cargo.toml" >&2; exit 1; }

# wixl: from the system if there, else the Ubuntu packages unpacked under $tools
# (no root needed). Their names are those of Ubuntu 24.04.
if command -v wixl >/dev/null; then
    wixl=wixl
else
    wixl=$tools/root/usr/bin/wixl
    if [ ! -x "$wixl" ]; then
        mkdir -p "$tools/debs" "$tools/root"
        (cd "$tools/debs" && apt-get download wixl wixl-data msitools libmsi-1.0-0 libgcab-1.0-0)
        for deb in "$tools"/debs/*.deb; do dpkg -x "$deb" "$tools/root"; done
    fi
    export LD_LIBRARY_PATH=$tools/root/usr/lib/x86_64-linux-gnu${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}
fi

# cargo-xwin, pinned so that a run does not change under us.
export PATH=$tools/cargo/bin:$PATH
command -v cargo-xwin >/dev/null || cargo install cargo-xwin --locked --version 0.23.1 --root "$tools/cargo"
rustup target add "$target"

export XWIN_ACCEPT_LICENSE=1 XWIN_CACHE_DIR=$tools/xwin-cache
cd "$root"
cargo xwin build --release -p cliotree-app --bin cliotree --target "$target"

out=$root/target/msi
mkdir -p "$out"
"$wixl" -a x64 -o "$out/ClioTree-$version-x64.msi" \
    -D "Version=$version" \
    -D "Exe=$root/target/$target/release/cliotree.exe" \
    -D "License=$root/LICENSE" \
    "$root/packaging/windows/cliotree.wxs"
ls -la "$out"
