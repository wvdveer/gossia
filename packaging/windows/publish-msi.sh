#!/bin/bash
# Puts the MSI that build-msi.sh left in target/msi/ in the generic package
# registry of the Gitea instance, beside the MSIX and the .vsix
# (packaging/publish-generic.sh says how, and which token):
#
#   publish-msi.sh
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
file=$(find "$root/target/msi" -maxdepth 1 -name 'Gossia-*.msi' | sort | tail -1)
[ -n "$file" ] || { echo "no MSI in target/msi: run packaging/windows/build-msi.sh first" >&2; exit 1; }
exec bash "$root/packaging/publish-generic.sh" "$file"
