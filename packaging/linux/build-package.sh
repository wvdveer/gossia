#!/bin/bash
# Builds a Linux package of ClioTree on a machine of that family, so that it
# links against that family's libraries (DESIGN.md, Testing and CI):
#
#   build-package.sh deb    Debian and Ubuntu, with cargo-deb
#   build-package.sh rpm    Fedora and openSUSE, with cargo-generate-rpm
#   build-package.sh arch   Arch, with makepkg (packaging/linux/arch/PKGBUILD)
#
# Each holds cliotree and cliotree-cli in /usr/bin, the licence, a .desktop
# file, the icon and the MIME type of .clio files. The package is left in
# target/packages/. The packaging tools are installed under
# $CLIOTREE_PACKAGE_TOOLS (default ~/.cache/cliotree-packages), pinned, and
# reused; nothing is installed system-wide.
set -euo pipefail

kind=${1:?usage: build-package.sh deb|rpm|arch}
root=$(cd "$(dirname "$0")/../.." && pwd)
tools=${CLIOTREE_PACKAGE_TOOLS:-$HOME/.cache/cliotree-packages}
cd "$root"

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
[ -n "$version" ] || { echo "no version in Cargo.toml" >&2; exit 1; }

export PATH=$tools/cargo/bin:$HOME/.cargo/bin:$PATH
rustup show active-toolchain >/dev/null 2>&1 || rustup toolchain install
cargo xtask vendor fetch

# Only the package of this run is left there, for publish-package.sh.
out=$root/target/packages
mkdir -p "$out"
rm -f "$out"/*.deb "$out"/*.rpm "$out"/*.pkg.tar.zst

build() {
    cargo build --release --locked -p cliotree-app -p cliotree-cli --bin cliotree --bin cliotree-cli
    cargo xtask icons target/release/icons
}

# Installs a cargo subcommand under $tools, at a fixed version.
tool() {
    local crate=$1 version=$2
    if ! "$tools/cargo/bin/$crate" --version 2>/dev/null | grep -q "$version"; then
        cargo install "$crate" --locked --version "$version" --root "$tools/cargo"
    fi
}

case $kind in
    deb)
        build
        tool cargo-deb 3.8.0
        cargo deb -p cliotree-app --no-build --output "$out/"
        ;;
    rpm)
        build
        tool cargo-generate-rpm 0.21.1
        cargo generate-rpm -p crates/cliotree-app -o "$out/"
        ;;
    arch)
        rm -f packaging/linux/arch/*.pkg.tar.zst
        (cd packaging/linux/arch && CLIOTREE_VERSION=$version makepkg -f --noconfirm)
        mv packaging/linux/arch/*.pkg.tar.zst "$out/"
        rm -rf packaging/linux/arch/src packaging/linux/arch/pkg
        ;;
    *)
        echo "unknown package kind: $kind (deb, rpm or arch)" >&2
        exit 2
        ;;
esac
ls -la "$out"
