#!/bin/bash
# Puts one file in the generic package registry of the Gitea mirror (package
# gossia, owner ward), under the workspace version, replacing a file of the
# same name and version (the registry does not overwrite):
#
#   publish-generic.sh FILE
#
# Used for the .vsix (packaging/vscode/publish-vsix.sh), the MSI
# (packaging/windows/publish-msi.sh) and the files CI keeps of a failed run
# (packaging/keep-ci-files.sh); the MSIX does the same from build-msix.ps1.
# PACKAGE_NAME and PACKAGE_VERSION put it under another package or version.
#
# GITEA_URL names the server; when it is not set, the server of the Git
# remote named gitea is used.
# PACKAGE_REGISTRY_TOKEN is a Gitea access token of ward with the package
# write permission; when it is not set, the login Git's credential helper
# holds for that server and user ward is used, so it can be run by hand (Git
# Bash on Windows). In CI without the token it says so and publishes nothing.
set -euo pipefail

file=${1:?usage: publish-generic.sh FILE}
root=$(cd "$(dirname "$0")/.." && pwd)
server=${GITEA_URL:-}
if [ -z "$server" ]; then
    remote=$(git -C "$root" remote get-url gitea 2>/dev/null || true)
    # http://user@host:port/owner/repo.git -> http://host:port
    server=$(printf '%s' "$remote" | sed -E 's#^(https?://)([^@/]*@)?([^/]+).*#\1\3#')
fi
case $server in
    http://*|https://*) ;;
    *) echo "set GITEA_URL, or add the Gitea mirror as the Git remote gitea" >&2; exit 1 ;;
esac
server=${server%/}
owner=ward
package=${PACKAGE_NAME:-gossia}

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
version=${PACKAGE_VERSION:-$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -1)}
[ -n "$version" ] || { echo "no version in Cargo.toml" >&2; exit 1; }
name=$(basename "$file")
url=$server/api/packages/$owner/generic/$package/$version/$name
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
