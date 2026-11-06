#!/bin/bash
# Keeps the files of a failed CI step (the snapshot images that differ, the
# web harness's screenshots) in the Gitea package registry, since the Gitea
# runners have no Node.js for upload-artifact: the folder is zipped with
# Python and put in the generic package gossia-ci under the commit's short
# hash (packaging/publish-generic.sh says which token):
#
#   keep-ci-files.sh NAME FOLDER
#
# leaves gossia-ci/<commit>/NAME.zip. Gitea's package cleanup rules can
# remove the old ones.
set -euo pipefail

name=${1:?usage: keep-ci-files.sh NAME FOLDER}
folder=${2:?usage: keep-ci-files.sh NAME FOLDER}
root=$(cd "$(dirname "$0")/.." && pwd)
if [ ! -d "$folder" ]; then
    echo "nothing to keep: there is no $folder"
    exit 0
fi
commit=$(git -C "$root" rev-parse --short=12 HEAD)
zip="$root/target/ci-files/$name.zip"
mkdir -p "$(dirname "$zip")"
rm -f "$zip"
python3 -m zipfile -c "$zip" "$folder"
PACKAGE_NAME=gossia-ci PACKAGE_VERSION=$commit exec bash "$root/packaging/publish-generic.sh" "$zip"
