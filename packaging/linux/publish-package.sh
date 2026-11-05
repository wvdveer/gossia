#!/bin/bash
# Publishes the package that build-package.sh left in target/packages/ to the
# package registry of the Gitea instance (owner ward), replacing one of the
# same name, version and architecture (the registry does not overwrite):
#
#   publish-package.sh deb|rpm|arch
#
# Needs GITEA_URL (the server, e.g. $GITHUB_SERVER_URL in a workflow) and
# PACKAGE_REGISTRY_TOKEN (a Gitea access token of ward with the package write
# permission). Without the token it says so and publishes nothing.
#
# Where they go: the Debian registry under the distribution of the machine
# that built it (its codename) and component main; the RPM registry in group
# gossia; the Arch registry in repository gossia.
set -euo pipefail

kind=${1:?usage: publish-package.sh deb|rpm|arch}
root=$(cd "$(dirname "$0")/../.." && pwd)
server=${GITEA_URL:?GITEA_URL is not set}
owner=ward
group=gossia

if [ -z "${PACKAGE_REGISTRY_TOKEN:-}" ]; then
    echo "PACKAGE_REGISTRY_TOKEN is not set: the package is built but not published."
    exit 0
fi

case $kind in
    deb) pattern='*.deb' ;;
    rpm) pattern='*.rpm' ;;
    arch) pattern='*.pkg.tar.zst' ;;
    *) echo "unknown package kind: $kind" >&2; exit 2 ;;
esac
file=$(find "$root/target/packages" -maxdepth 1 -name "$pattern" | sort | tail -1)
[ -n "$file" ] || { echo "no $pattern in target/packages" >&2; exit 1; }
echo "Publishing $file"

case $kind in
    deb)
        name=$(dpkg-deb -f "$file" Package)
        version=$(dpkg-deb -f "$file" Version)
        arch=$(dpkg-deb -f "$file" Architecture)
        distribution=$(. /etc/os-release && echo "${VERSION_CODENAME:-stable}")
        remove=$server/api/packages/$owner/debian/pool/$distribution/main/$name/$version/$arch
        upload=$server/api/packages/$owner/debian/pool/$distribution/main/upload
        ;;
    rpm)
        name=$(rpm -qp --qf '%{NAME}' "$file")
        version=$(rpm -qp --qf '%{VERSION}-%{RELEASE}' "$file")
        arch=$(rpm -qp --qf '%{ARCH}' "$file")
        remove=$server/api/packages/$owner/rpm/$group/package/$name/$version/$arch
        upload=$server/api/packages/$owner/rpm/$group/upload
        ;;
    arch)
        info=$(tar -xOf "$file" .PKGINFO)
        field() { sed -n "s/^$1 = //p" <<<"$info" | head -1; }
        name=$(field pkgname)
        version=$(field pkgver)
        arch=$(field arch)
        remove=$server/api/packages/$owner/arch/$group/$name/$version/$arch
        upload=$server/api/packages/$owner/arch/$group
        ;;
esac

auth="$owner:$PACKAGE_REGISTRY_TOKEN"
code=$(curl -s -o /dev/null -w '%{http_code}' -X DELETE --user "$auth" "$remove")
echo "Removed $name $version $arch, if it was there: HTTP $code"
code=$(curl -s -o /tmp/gossia-upload.txt -w '%{http_code}' -X PUT --user "$auth" \
    --upload-file "$file" "$upload")
echo "Upload: HTTP $code"
cat /tmp/gossia-upload.txt
echo
[ "$code" = 201 ] || { echo "publishing failed" >&2; exit 1; }
