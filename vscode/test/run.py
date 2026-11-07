"""Runs the extension's tests in a VS Code downloaded for them (DESIGN.md, Web
port, 0.3.0, Testing), without Node.js: VS Code runs test/suite.js in its own
extension host.

Insiders by default, which runs beside an open VS Code (stable refuses to run
tests while one is open); GOSSIA_VSCODE_VERSION names another (`stable`, or a
version such as `1.105.1`). The download is kept in target/vscode-test and
used again. media/pkg must hold the test build (cargo xtask vscode
--test-control). On Linux without a display, run it under `xvfb-run -a`.

    python vscode/test/run.py
"""

import json
import os
import platform
import shutil
import subprocess
import sys
import tarfile
import tempfile
import urllib.request
import zipfile
from pathlib import Path

EXTENSION = Path(__file__).resolve().parent.parent
ROOT = EXTENSION.parent
CACHE = ROOT / "target" / "vscode-test"
UPDATES = "https://update.code.visualstudio.com"


def download_platform():
    """The name the update server gives this system's archive."""
    machine = platform.machine().lower()
    if sys.platform == "win32":
        return "win32-arm64-archive" if machine == "arm64" else "win32-x64-archive"
    if sys.platform == "darwin":
        return "darwin-universal"
    if sys.platform.startswith("linux"):
        return "linux-arm64" if machine in ("aarch64", "arm64") else "linux-x64"
    sys.exit(f"no VS Code download for {sys.platform}")


def executable(folder, insiders):
    """The program in an unpacked VS Code, or None."""
    if sys.platform == "win32":
        names = ["Code - Insiders.exe" if insiders else "Code.exe"]
    elif sys.platform == "darwin":
        app = "Visual Studio Code - Insiders.app" if insiders else "Visual Studio Code.app"
        names = [f"{app}/Contents/MacOS/Electron"]
    else:
        program = "code-insiders" if insiders else "code"
        names = [program, f"VSCode-linux-x64/{program}", f"VSCode-linux-arm64/{program}"]
    for name in names:
        if (folder / name).is_file():
            return folder / name
    return None


def latest_stable():
    """The newest stable version, or None when the server cannot be asked."""
    try:
        with urllib.request.urlopen(f"{UPDATES}/api/releases/stable", timeout=30) as r:
            return json.load(r)[0]
    except Exception as e:  # noqa: BLE001: offline is fine when one is cached
        print(f"cannot ask for the latest VS Code: {e}")
        return None


def unpack(archive, folder):
    """Unpacks a .zip or .tar.gz into a new folder, keeping the programs'
    permissions."""
    tmp = folder.with_name(folder.name + ".tmp")
    shutil.rmtree(tmp, ignore_errors=True)
    tmp.mkdir(parents=True)
    if zipfile.is_zipfile(archive):
        with zipfile.ZipFile(archive) as z:
            for info in z.infolist():
                path = Path(z.extract(info, tmp))
                mode = info.external_attr >> 16
                if mode and not info.is_dir():
                    path.chmod(mode & 0o777)
    else:
        with tarfile.open(archive) as t:
            if hasattr(tarfile, "tar_filter"):
                t.extractall(tmp, filter="tar")
            else:
                t.extractall(tmp)
    tmp.rename(folder)


def vscode():
    """The program of the VS Code to test in, downloaded the first time."""
    wanted = os.environ.get("GOSSIA_VSCODE_VERSION") or "insiders"
    insiders = wanted == "insiders"
    where = download_platform()
    quality = "insider" if insiders else "stable"
    if wanted == "stable":
        version = latest_stable()
        if version is None:
            cached = sorted(CACHE.glob(f"vscode-{where}-[0-9]*"))
            if not cached:
                sys.exit("no VS Code is cached and the server cannot be reached")
            return executable(cached[-1], False)
    else:
        version = wanted
    label = "insiders" if insiders else version
    folder = CACHE / f"vscode-{where}-{label}"
    found = executable(folder, insiders)
    if found:
        return found
    url = f"{UPDATES}/{'latest' if insiders else version}/{where}/{quality}"
    CACHE.mkdir(parents=True, exist_ok=True)
    archive = CACHE / f"{folder.name}.download"
    print(f"downloading VS Code {label} from {url}")
    with urllib.request.urlopen(url, timeout=600) as r, open(archive, "wb") as f:
        shutil.copyfileobj(r, f)
    shutil.rmtree(folder, ignore_errors=True)
    unpack(archive, folder)
    archive.unlink()
    found = executable(folder, insiders)
    if not found:
        sys.exit(f"the download in {folder} has no VS Code program in it")
    return found


def main():
    if not (EXTENSION / "media" / "pkg" / "gossia_web_bg.wasm").is_file():
        sys.exit("media/pkg is empty: run cargo xtask vscode --test-control first")
    program = vscode()
    # A folder of its own for the workbooks, and a fresh profile, so that
    # nothing is kept from an earlier run.
    work = Path(tempfile.mkdtemp(prefix="gossia-vscode-"))
    out = work / "results.json"
    (work / "workspace").mkdir()
    env = dict(os.environ)
    env.pop("ELECTRON_RUN_AS_NODE", None)
    env.update(GOSSIA_VSCODE_TEST="1", GOSSIA_TEST_DIR=str(work), GOSSIA_TEST_OUT=str(out))
    args = [
        str(program),
        "--no-sandbox",
        "--disable-gpu-sandbox",
        "--disable-updates",
        "--skip-welcome",
        "--skip-release-notes",
        "--disable-workspace-trust",
        "--disable-extensions",
        f"--extensionDevelopmentPath={EXTENSION}",
        f"--extensionTestsPath={EXTENSION / 'test' / 'suite.js'}",
        "--user-data-dir",
        str(work / "user"),
        "--extensions-dir",
        str(work / "extensions"),
        str(work / "workspace"),
    ]
    if sys.platform.startswith("linux"):
        # WebGL without a graphics adapter (the CI runner under xvfb-run), as
        # the web harness gives Chromium: without it the panel cannot start.
        args[1:1] = ["--use-angle=swiftshader", "--enable-unsafe-swiftshader", "--ignore-gpu-blocklist"]
    print("+ " + " ".join(args))
    try:
        code = subprocess.call(args, env=env)
    finally:
        if out.is_file():
            results = json.loads(out.read_text(encoding="utf-8"))
            failed = sum(1 for r in results if not r["ok"])
            print(f"== Summary: {len(results) - failed} passed, {failed} failed ==")
        shutil.rmtree(work, ignore_errors=True)
    if code != 0:
        sys.exit(f"the tests failed (VS Code exited with {code})")


if __name__ == "__main__":
    main()
