#!/bin/bash
# Puts the .vsix that cargo xtask vsix left in target/vsix/ in the generic
# package registry of the Gitea instance, beside the MSI and the MSIX
# (packaging/publish-generic.sh says how, and which token):
#
#   publish-vsix.sh
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
file=$(find "$root/target/vsix" -maxdepth 1 -name 'gossia-*.vsix' | sort | tail -1)
[ -n "$file" ] || { echo "no .vsix in target/vsix: run cargo xtask vsix first" >&2; exit 1; }
exec bash "$root/packaging/publish-generic.sh" "$file"
