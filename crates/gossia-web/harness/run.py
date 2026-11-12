"""Runs the harness page headless, for CI (DESIGN.md, Web port, 0.3.0, Testing).

Usage: python run.py <folder> [--headed] [--timeout SECONDS]

<folder> is the test build of the wasm version with the harness beside it,
as `cargo xtask web --test-control` makes it (target/web-test). The folder is
served on a loopback port, Chromium opens harness.html?auto, and the harness
drives the application in other tabs. The harness's log is printed, and the
exit code is 0 only when every check passed. On a failure, a screenshot of
each tab still open is saved in <folder>/screenshots.

Needs Playwright for Python and its Chromium:
    python -m pip install playwright
    python -m playwright install chromium
"""

import argparse
import functools
import http.server
import pathlib
import shutil
import sys
import threading


class Handler(http.server.SimpleHTTPRequestHandler):
    """Serves the folder, with the types a module and a wasm file need."""

    extensions_map = {
        **http.server.SimpleHTTPRequestHandler.extensions_map,
        ".wasm": "application/wasm",
        ".js": "text/javascript",
        ".json": "application/json",
        ".html": "text/html",
    }

    def end_headers(self):
        self.send_header("Cache-Control", "no-store")
        super().end_headers()

    def log_message(self, format, *args):
        pass


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("folder", type=pathlib.Path)
    parser.add_argument("--headed", action="store_true", help="show the browser")
    parser.add_argument("--timeout", type=float, default=600, help="seconds for the whole run")
    args = parser.parse_args()
    folder = args.folder.resolve()
    if not (folder / "harness.html").is_file() or not (folder / "pkg").is_dir():
        sys.exit(f"{folder} has no harness.html and pkg/: run cargo xtask web --test-control")
    # Screenshots of an earlier run would be taken for this one's.
    shutil.rmtree(folder / "screenshots", ignore_errors=True)

    try:
        from playwright.sync_api import sync_playwright
    except ImportError:
        sys.exit("Playwright for Python is missing: python -m pip install playwright, "
                 "then python -m playwright install chromium")

    server = http.server.ThreadingHTTPServer(
        ("127.0.0.1", 0), functools.partial(Handler, directory=str(folder)))
    threading.Thread(target=server.serve_forever, daemon=True).start()
    # Another origin (another port) serving the same files without allowing
    # this one, for the browser host's CORS error.
    other = http.server.ThreadingHTTPServer(
        ("127.0.0.1", 0), functools.partial(Handler, directory=str(folder)))
    threading.Thread(target=other.serve_forever, daemon=True).start()
    url = (f"http://127.0.0.1:{server.server_address[1]}/harness.html?auto"
           f"&other={other.server_address[1]}")

    with sync_playwright() as p:
        # WebGL without a graphics adapter, as on the runner.
        browser = p.chromium.launch(
            headless=not args.headed,
            args=["--use-angle=swiftshader", "--enable-unsafe-swiftshader"])
        context = browser.new_context(viewport={"width": 1280, "height": 800})

        def watch(page):
            # Errors in the application's tabs, which the harness cannot see.
            page.on("console", lambda m: m.type == "error" and print(f"[{page.url}] {m.text}"))
            page.on("pageerror", lambda e: print(f"[{page.url}] {e}"))

        context.on("page", watch)
        page = context.new_page()
        page.goto(url)
        try:
            page.wait_for_function("window.harnessDone", timeout=args.timeout * 1000, polling=500)
        except Exception:
            print(page.evaluate("window.harness ? window.harness.logText() : ''"))
            shoot(context, folder)
            sys.exit(f"the harness did not finish within {args.timeout:.0f} s")
        done = page.evaluate("window.harnessDone")
        print(done["log"])
        ok = done["fail"] == 0 and done["pass"] > 0
        if not ok:
            shoot(context, folder)
        browser.close()
    server.shutdown()
    other.shutdown()
    sys.exit(0 if ok else 1)


def shoot(context, folder):
    out = folder / "screenshots"
    out.mkdir(exist_ok=True)
    for i, page in enumerate(context.pages):
        path = out / f"tab-{i}.png"
        try:
            page.screenshot(path=str(path))
            print(f"screenshot of {page.url}: {path}")
        except Exception as e:
            print(f"no screenshot of {page.url}: {e}")


if __name__ == "__main__":
    main()
