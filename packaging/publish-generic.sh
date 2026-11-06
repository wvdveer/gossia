#!/bin/bash
# Puts one file in the generic package registry of the Gitea instance on acer
# (package gossia, owner ward), under the workspace version, replacing a file
# of the same name and version (the registry does not overwrite):
#
#   publish-generic.sh FILE
#
# Used for the .vsix (packaging/vscode/publish-vsix.sh) and the MSI
# (packaging/windows/publish-msi.sh); the MSIX does the same from
# build-msix.ps1.
#
# GITEA_URL names the server (http://acer.patersonst:3000 when not set).
# PACKAGE_REGISTRY_TOKEN is a Gitea access token of ward with the package
# write permission; when it is not set, the login Git's credential helper
# holds for that server and user ward is used, so it can be run by hand (Git
# Bash on Windows). In CI without the token it says so and publishes nothing.
set -euo pipefail

file=${1:?usage: publish-generic.sh FILE}
root=$(cd "$(dirname "$0")/.." && pwd)
server=${GITEA_URL:-http://acer.patersonst:3000}
owner=ward

token=${PACKAGE_REGISTRY_TOKEN:-}
if [ -z "$token" ] && [ -z "${CI:-}" ]; then
    hostport=${server#*://}
    protocol=${server%%://*}
    token=$(printf 'protocol=%s\nhost=%s\nusername=%s\n\n' "$protocol" "$hostport" "$owner" \
        | GCM_INTERACTIVE=never git credential fill 2>/dev/null | sed -n 's/^password=//p')
fi
if [ -z "$token" ]; then
    echo "PACKAGE_REGISTRY_TOKEN is not set: $(basename "$file") is built but not published."
    exit 0
fi

[ -f "$file" ] || { echo "no such file: $file" >&2; exit 1; }
version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -1)
[ -n "$version" ] || { echo "no version in Cargo.toml" >&2; exit 1; }
name=$(basename "$file")
url=$server/api/packages/$owner/generic/gossia/$version/$name
echo "Publishing $file"

auth="$owner:$token"
code=$(curl -s -o /dev/null -w '%{http_code}' -X DELETE --user "$auth" "$url")
echo "Removed the earlier file of this version, if it was there: HTTP $code"
out=$(mktemp)
code=$(curl -s -o "$out" -w '%{http_code}' -X PUT --user "$auth" --upload-file "$file" "$url")
echo "Upload: HTTP $code"
cat "$out"; rm -f "$out"
echo
[ "$code" = 201 ] || { echo "publishing failed" >&2; exit 1; }
echo "Published $url"
