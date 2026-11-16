#!/usr/bin/env python3
"""Copies a GitHub release of Gossia to the Gitea mirror: its title, notes
and files, as a release page on Gitea for the same tag.

    python packaging/release-to-gitea.py v0.6.0 [v0.5.0 ...]

The release is made on GitHub first (gh release create); this makes the same
page on Gitea, or brings one made before up to date: the title and notes are
replaced, a file of the same name is replaced, and files the GitHub release
no longer has are removed. The tag has to be on Gitea already (it is pushed
to both). The release marked Latest on GitHub is the newest on Gitea too,
since Gitea orders releases by their tags' dates.

GitHub is read with gh, logged in as for gh release create.
GITEA_URL names the Gitea server; when it is not set, the server of the Git
remote named gitea is used, and the repository is the one that remote names.
GITEA_TOKEN is a Gitea access token with the repository write permission;
when it is not set, the login Git's credential helper holds for that server
is used, as packaging/publish-generic.sh does.
"""

import json
import os
import re
import subprocess
import sys
import tempfile
import urllib.error
import urllib.parse
import urllib.request
import uuid
from base64 import b64encode
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def git(*args, stdin=None):
    return subprocess.run(
        ["git", "-C", str(ROOT), *args],
        input=stdin,
        capture_output=True,
        text=True,
        check=False,
    ).stdout


def gitea_remote():
    """The server, the repository (owner/name) and the user of the remote."""
    url = git("remote", "get-url", "gitea").strip()
    m = re.match(r"^(https?)://(?:([^@/:]+)(?::[^@/]*)?@)?([^/]+)/(.+?)(?:\.git)?/?$", url)
    if not m:
        sys.exit("add the Gitea mirror as the Git remote gitea, or set GITEA_URL")
    protocol, user, host, repo = m.groups()
    server = os.environ.get("GITEA_URL", f"{protocol}://{host}").rstrip("/")
    return server, repo, user


def auth_header(server, user):
    token = os.environ.get("GITEA_TOKEN")
    if token:
        return f"token {token}"
    protocol, hostport = server.split("://", 1)
    query = f"protocol={protocol}\nhost={hostport}\n"
    if user:
        query += f"username={user}\n"
    env = dict(os.environ, GCM_INTERACTIVE="never")
    out = subprocess.run(
        ["git", "credential", "fill"],
        input=query + "\n",
        capture_output=True,
        text=True,
        env=env,
        check=False,
    ).stdout
    fields = dict(line.split("=", 1) for line in out.splitlines() if "=" in line)
    if "password" not in fields:
        sys.exit("set GITEA_TOKEN, or log in to the Gitea server with Git first")
    pair = f"{fields.get('username', user)}:{fields['password']}"
    return "Basic " + b64encode(pair.encode()).decode()


class Gitea:
    def __init__(self):
        self.server, self.repo, user = gitea_remote()
        self.auth = auth_header(self.server, user)

    def call(self, method, path, body=None, data=None, content_type=None):
        url = f"{self.server}/api/v1/repos/{self.repo}{path}"
        headers = {"Authorization": self.auth, "Accept": "application/json"}
        if body is not None:
            data = json.dumps(body).encode()
            content_type = "application/json"
        if content_type:
            headers["Content-Type"] = content_type
        req = urllib.request.Request(url, data=data, method=method, headers=headers)
        try:
            with urllib.request.urlopen(req, timeout=600) as r:
                text = r.read()
                return json.loads(text) if text else None
        except urllib.error.HTTPError as e:
            if e.code == 404 and method == "GET":
                return None
            sys.exit(f"{method} {path}: HTTP {e.code} {e.read().decode(errors='replace')[:400]}")

    def upload(self, release_id, file):
        boundary = uuid.uuid4().hex
        head = (
            f"--{boundary}\r\n"
            f'Content-Disposition: form-data; name="attachment"; filename="{file.name}"\r\n'
            "Content-Type: application/octet-stream\r\n\r\n"
        ).encode()
        data = head + file.read_bytes() + f"\r\n--{boundary}--\r\n".encode()
        name = urllib.parse.quote(file.name)
        return self.call(
            "POST",
            f"/releases/{release_id}/assets?name={name}",
            data=data,
            content_type=f"multipart/form-data; boundary={boundary}",
        )


def github_release(tag):
    out = subprocess.run(
        ["gh", "release", "view", tag, "--json", "name,body,isPrerelease,isDraft,assets"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    if out.returncode != 0:
        sys.exit(f"no GitHub release {tag}: {out.stderr.strip()}")
    return json.loads(out.stdout)


def copy(gitea, tag):
    release = github_release(tag)
    if release["isDraft"]:
        sys.exit(f"{tag} is a draft on GitHub: publish it there first")
    if gitea.call("GET", f"/tags/{urllib.parse.quote(tag)}") is None:
        sys.exit(f"the tag {tag} is not on Gitea: push it there first")
    fields = {
        "tag_name": tag,
        "name": release["name"],
        "body": release["body"],
        "draft": False,
        "prerelease": release["isPrerelease"],
    }
    page = gitea.call("GET", f"/releases/tags/{urllib.parse.quote(tag)}")
    if page is None:
        page = gitea.call("POST", "/releases", body=fields)
        print(f"{tag}: release page made on Gitea")
    else:
        page = gitea.call("PATCH", f"/releases/{page['id']}", body=fields)
        print(f"{tag}: release page on Gitea brought up to date")
    wanted = {a["name"] for a in release["assets"]}
    have = {a["name"]: a for a in page.get("assets", [])}
    for name, asset in have.items():
        if name not in wanted:
            gitea.call("DELETE", f"/releases/{page['id']}/assets/{asset['id']}")
            print(f"  removed {name}")
    with tempfile.TemporaryDirectory() as tmp:
        subprocess.run(
            ["gh", "release", "download", tag, "--dir", tmp],
            cwd=ROOT,
            check=True,
            stdout=subprocess.DEVNULL,
        )
        for file in sorted(Path(tmp).iterdir()):
            old = have.get(file.name)
            if old and old.get("size") == file.stat().st_size:
                print(f"  kept {file.name} (the same size)")
                continue
            if old:
                gitea.call("DELETE", f"/releases/{page['id']}/assets/{old['id']}")
            gitea.upload(page["id"], file)
            print(f"  uploaded {file.name} ({file.stat().st_size:,} bytes)")
    print(f"  {page['html_url']}")


def main():
    tags = sys.argv[1:]
    if not tags:
        sys.exit(__doc__.strip().split("\n\n")[1])
    gitea = Gitea()
    for tag in tags:
        copy(gitea, tag)


if __name__ == "__main__":
    main()
