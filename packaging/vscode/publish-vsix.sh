#!/bin/bash
# Puts the .vsix that cargo xtask vsix left in target/vsix/ in the generic
# package registry of the Gitea instance (package gossia, owner ward), as the
# MSIX is, replacing a file of the same version (the registry does not
# overwrite):
#
#   publish-vsix.sh
#
# GITEA_URL names the server (http://acer.patersonst:3000 when not set).
# PACKAGE_REGISTRY_TOKEN is a Gitea access token of ward with the package
# write permission; when it is not set, the login Git's credential helper
# holds for that server and user ward is used, so it can be run by hand (Git
# Bash on Windows). In CI without the token it says so and publishes nothing.
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
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
    echo "PACKAGE_REGISTRY_TOKEN is not set: the .vsix is built but not published."
    exit 0
fi

file=$(find "$root/target/vsix" -maxdepth 1 -name 'gossia-*.vsix' | sort | tail -1)
[ -n "$file" ] || { echo "no .vsix in target/vsix: run cargo xtask vsix first" >&2; exit 1; }
name=$(basename "$file")
version=${name#gossia-}
version=${version%.vsix}
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
