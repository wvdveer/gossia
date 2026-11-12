//! Project helper commands, run as `cargo xtask <command>`.
//!
//! This lives in its own workspace (`tools/`) so that it works before `vendor/`
//! has been fetched. See DESIGN.md, "Build and dependencies".

use std::path::{Path, PathBuf};
use std::process::Command;

mod icons;

const HELP: &str = "\
cargo xtask <command>

  clean                          remove the build output and vendor/
  vendor fetch                   fetch any missing or out-of-date vendor item
  vendor status                  show each item, its pin, and its state
  vendor verify                  check every item against vendor.lock
  vendor update <item> <rev>     change a pin in vendor.lock (for review)
  lint                           formatting and clippy
  test                           the default test levels
  e2e                            end-to-end tests on the running application (under xvfb-run on Linux without a display)
  snapshots                      visual snapshot tests (needs a graphics adapter)
  icons <dir>                    the icon as PNG files for the packages (16 to 512 pixels)
  ico <file>                     the icon as a Windows .ico file, for the MSI's shortcut
  web [--test-control]           the wasm version and its pages, in target/web (serve that folder);
                                 with --test-control the test build and the harness, in target/web-test
  web-test                       the test build of the wasm version, driven by the harness page in
                                 headless Chromium (needs Playwright for Python)
  vscode [--test-control]        the wasm version copied into the VS Code extension (vscode/media/pkg)
  vscode-test                    the extension's tests in a downloaded VS Code (needs Python)
  vsix                           the extension packed as target/vsix/gossia-<version>.vsix, with the
                                 release build of the wasm version (needs Python)
  ci                             everything the blocking CI jobs run: lint, test, e2e
";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(message) = run(&args) {
        eprintln!("error: {message}");
        std::process::exit(1);
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let root = repo_root()?;
    match args.first().map(String::as_str) {
        Some("clean") => clean(&root),
        Some("vendor") => fetch_vendor::run_vendor_command(&args[1..]),
        Some("lint") => {
            fetch_vendor::ensure_required(&root)?;
            lint(&root)
        }
        Some("test") => {
            fetch_vendor::ensure_required(&root)?;
            test(&root)
        }
        Some("e2e") => {
            fetch_vendor::ensure_required(&root)?;
            e2e(&root)
        }
        Some("snapshots") => {
            fetch_vendor::ensure_required(&root)?;
            snapshots(&root)
        }
        Some("web") => {
            fetch_vendor::ensure_required(&root)?;
            match args.get(1).map(String::as_str) {
                None => web(&root, false).map(|_| ()),
                Some("--test-control") => web(&root, true).map(|_| ()),
                Some(other) => Err(format!("web takes --test-control, not {other}")),
            }
        }
        Some("web-test") => {
            fetch_vendor::ensure_required(&root)?;
            web_test(&root)
        }
        Some("vscode") => {
            fetch_vendor::ensure_required(&root)?;
            match args.get(1).map(String::as_str) {
                None => vscode(&root, false),
                Some("--test-control") => vscode(&root, true),
                Some(other) => Err(format!("vscode takes --test-control, not {other}")),
            }
        }
        Some("vscode-test") => {
            fetch_vendor::ensure_required(&root)?;
            vscode_test(&root)
        }
        Some("vsix") => {
            fetch_vendor::ensure_required(&root)?;
            vsix(&root).map(|_| ())
        }
        Some("icons") => match args.get(1) {
            Some(dir) => icons::write(Path::new(dir)),
            None => Err("icons needs the folder to write to".to_owned()),
        },
        Some("ico") => match args.get(1) {
            Some(file) => icons::write_ico(Path::new(file)),
            None => Err("ico needs the file to write".to_owned()),
        },
        Some("ci") => {
            fetch_vendor::ensure_required(&root)?;
            lint(&root)?;
            test(&root)?;
            e2e(&root)
        }
        _ => {
            print!("{HELP}");
            Ok(())
        }
    }
}

fn repo_root() -> Result<PathBuf, String> {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    fetch_vendor::find_root(&cwd).ok_or_else(|| "run this from inside the repository".to_owned())
}

/// Runs `cargo <args>` in `dir`, failing if it fails.
fn cargo(dir: &Path, args: &[&str]) -> Result<(), String> {
    let program = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    println!("+ cargo {}", args.join(" "));
    let status = Command::new(program)
        .args(args)
        .current_dir(dir)
        .status()
        .map_err(|e| format!("cannot run cargo: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`cargo {}` failed", args.join(" ")))
    }
}

fn clean(root: &Path) -> Result<(), String> {
    cargo(root, &["clean"])?;
    let vendor = root.join("vendor");
    if vendor.exists() {
        fetch_vendor::remove_dir_all_force(&vendor)?;
        println!("removed {}", vendor.display());
    }
    if std::env::var_os("GOSSIA_VENDOR_DIR").is_some_and(|v| !v.is_empty()) {
        println!("GOSSIA_VENDOR_DIR is set; that directory was left alone");
    }
    Ok(())
}

fn lint(root: &Path) -> Result<(), String> {
    // Not `--all`: that would also format the vendored path dependencies.
    cargo(root, &["fmt", "--", "--check"])?;
    cargo(
        root,
        &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ],
    )?;
    // The test control channel is compiled only with its feature.
    cargo(
        root,
        &[
            "clippy",
            "-p",
            "gossia-app",
            "--all-targets",
            "--features",
            "test-control",
            "--",
            "-D",
            "warnings",
        ],
    )?;
    // The wasm version (DESIGN.md, Web port, 0.3.0): the crates that are built
    // for wasm, with the code that only that target compiles.
    cargo(
        root,
        &[
            "clippy",
            "-p",
            "gossia-calc",
            "-p",
            "gossia-user",
            "-p",
            "gossia-app",
            "-p",
            "gossia-web",
            "--lib",
            "--target",
            WASM_TARGET,
            // The test build's control() too.
            "--features",
            "gossia-web/test-control",
            "--",
            "-D",
            "warnings",
        ],
    )?;
    cargo(
        root,
        &[
            "fmt",
            "--manifest-path",
            "tools/Cargo.toml",
            "--all",
            "--",
            "--check",
        ],
    )?;
    cargo(
        root,
        &[
            "clippy",
            "--manifest-path",
            "tools/Cargo.toml",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ],
    )
}

const WASM_TARGET: &str = "wasm32-unknown-unknown";

/// Builds the wasm version (`gossia-web`) in release, makes its JavaScript
/// glue with `wasm-bindgen`, and puts it beside its pages in `target/web`.
/// The `wasm-bindgen` command-line tool must be the version the lock file
/// has, so that version is installed into `target/wasm-tools` the first time.
///
/// `test_control` builds the test build instead (the `test-control`
/// feature, with `control()` on the page), with the harness page beside it,
/// in `target/web-test`. Gives the folder.
fn web(root: &Path, test_control: bool) -> Result<PathBuf, String> {
    let mut build = vec![
        "build",
        "-p",
        "gossia-web",
        "--release",
        "--target",
        WASM_TARGET,
    ];
    if test_control {
        // Its own target folder, so the two builds do not replace each other.
        build.extend([
            "--features",
            "test-control",
            "--target-dir",
            "target/web-test-build",
        ]);
    }
    cargo(root, &build)?;
    let version = locked_version(root, "wasm-bindgen")?;
    let tools = root.join("target").join("wasm-tools");
    let bindgen = tools
        .join("bin")
        .join(format!("wasm-bindgen{}", std::env::consts::EXE_SUFFIX));
    let installed = Command::new(&bindgen)
        .arg("--version")
        .output()
        .ok()
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned());
    if installed.as_deref() != Some(&format!("wasm-bindgen {version}")) {
        let root_arg = tools.to_string_lossy().into_owned();
        cargo(
            root,
            &[
                "install",
                "wasm-bindgen-cli",
                "--version",
                &version,
                "--locked",
                "--root",
                &root_arg,
            ],
        )?;
    }
    let out = root
        .join("target")
        .join(if test_control { "web-test" } else { "web" });
    let wasm = root
        .join("target")
        .join(if test_control { "web-test-build" } else { "" })
        .join(WASM_TARGET)
        .join("release")
        .join("gossia_web.wasm");
    println!("+ wasm-bindgen {}", wasm.display());
    let mut command = Command::new(&bindgen);
    command.args(["--target", "web", "--no-typescript"]);
    // The function names are a fifth of the module (DESIGN.md, step 13g);
    // the test build keeps them, for stack traces that say where.
    if !test_control {
        command.arg("--remove-name-section");
    }
    let status = command
        .arg("--out-dir")
        .arg(out.join("pkg"))
        .arg(&wasm)
        .status()
        .map_err(|e| format!("cannot run wasm-bindgen: {e}"))?;
    if !status.success() {
        return Err("wasm-bindgen failed".to_owned());
    }
    // The pages and scripts beside the glue: the application's page with its
    // test host, the worker, and the worker protocol's probe.
    let web = root.join("crates").join("gossia-web");
    copy_files(&web.join("web"), &out)?;
    // The icons of the installed web app and the page (DESIGN.md, 17c),
    // drawn as the window's.
    let icons = out.join("icons");
    std::fs::create_dir_all(&icons).map_err(|e| format!("{}: {e}", icons.display()))?;
    for size in WEB_ICON_SIZES {
        icons::write_one(*size, &icons.join(format!("{size}.png")))?;
    }
    // The files the service worker keeps (the page's, not the harness's),
    // and the build they make, named by their contents, so that a new build
    // replaces the cached one (17c).
    let mut files: Vec<String> = std::fs::read_dir(web.join("web"))
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .filter(|e| e.path().is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name != "sw.js")
        .collect();
    files.extend([
        "pkg/gossia_web.js".to_owned(),
        "pkg/gossia_web_bg.wasm".to_owned(),
    ]);
    files.extend(
        WEB_ICON_SIZES
            .iter()
            .map(|size| format!("icons/{size}.png")),
    );
    files.sort();
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for file in &files {
        let bytes = std::fs::read(out.join(file)).map_err(|e| format!("{file}: {e}"))?;
        for byte in file.bytes().chain(bytes) {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
        }
    }
    let build = format!("{hash:016x}");
    // The module's size, for the loading screen's progress (the length a
    // server gives is the compressed one where it compresses; DESIGN.md,
    // 17b), the version and the build.
    let wasm_bytes = std::fs::metadata(out.join("pkg").join("gossia_web_bg.wasm"))
        .map_err(|e| format!("gossia_web_bg.wasm: {e}"))?
        .len();
    let version = locked_version(root, "gossia-web")?;
    std::fs::write(
        out.join("build.json"),
        format!(
            "{{\"version\": \"{version}\", \"build\": \"{build}\", \"wasmBytes\": {wasm_bytes}}}\n"
        ),
    )
    .map_err(|e| format!("cannot write build.json: {e}"))?;
    files.extend(["./".to_owned(), "build.json".to_owned()]);
    let list = files
        .iter()
        .map(|f| format!("\"{f}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let worker = std::fs::read_to_string(web.join("web").join("sw.js"))
        .map_err(|e| format!("sw.js: {e}"))?
        .replace("'__GOSSIA_BUILD__'", &format!("'{build}'"))
        .replace("__GOSSIA_FILES__", &format!("[{list}]"));
    std::fs::write(out.join("sw.js"), worker).map_err(|e| format!("cannot write sw.js: {e}"))?;
    if test_control {
        // The harness page and its scripts; the Playwright script stays.
        copy_files(&web.join("harness"), &out)?;
        let _ = std::fs::remove_file(out.join("run.py"));
    }
    println!("built {}", out.display());
    Ok(out)
}

/// The sizes of the web version's icons: the page's, Apple's home screen's,
/// and the web app manifest's.
const WEB_ICON_SIZES: &[u32] = &[32, 180, 192, 512];

/// Copies the files (not the folders) of `from` into `to`.
fn copy_files(from: &Path, to: &Path) -> Result<(), String> {
    let entries = std::fs::read_dir(from).map_err(|e| format!("{}: {e}", from.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        if entry.path().is_file() {
            let name = entry.file_name();
            std::fs::copy(entry.path(), to.join(&name))
                .map_err(|e| format!("cannot copy {}: {e}", name.to_string_lossy()))?;
        }
    }
    Ok(())
}

/// Builds the wasm version (or its test build) and copies the module and its
/// glue into the VS Code extension, `vscode/media/pkg`.
fn vscode(root: &Path, test_control: bool) -> Result<(), String> {
    let out = web(root, test_control)?;
    let pkg = root.join("vscode").join("media").join("pkg");
    std::fs::create_dir_all(&pkg).map_err(|e| format!("{}: {e}", pkg.display()))?;
    for name in ["gossia_web.js", "gossia_web_bg.wasm"] {
        std::fs::copy(out.join("pkg").join(name), pkg.join(name))
            .map_err(|e| format!("cannot copy {name}: {e}"))?;
    }
    println!("copied the wasm version into {}", pkg.display());
    cjk_fonts(root)
}

/// The fonts for Japanese and Chinese that the extension loads when one of
/// them is the interface's language (DESIGN.md, the decisions for the
/// interface's languages): Noto Sans JP and SC from the Google Fonts
/// repository, at the commit of the bundled fonts, each with its SHA-256.
const CJK_FONTS: &[(&str, &str, &str)] = &[
    (
        "NotoSansJP.ttf",
        "ofl/notosansjp/NotoSansJP%5Bwght%5D.ttf",
        "c2f3b4d463500a2ddcd3849cded1fceeb9fd6d1c32e6cbecd568453ba50fc68f",
    ),
    (
        "NotoSansSC.ttf",
        "ofl/notosanssc/NotoSansSC%5Bwght%5D.ttf",
        "a3041811a78c361b1de50f953c805e0244951c21c5bd412f7232ef0d899af0da",
    ),
    // One licence for both (the files are the same).
    (
        "OFL.txt",
        "ofl/notosansjp/OFL.txt",
        "1c05c68c34f9708415aada51f17e1b0092d2cea709bf4a94cd38114f9e73d7d9",
    ),
];

/// The commit of the Google Fonts repository the fonts come from, as in
/// `crates/gossia-layout/assets/fonts/FONTS.md`.
const GOOGLE_FONTS_COMMIT: &str = "e7db6ca603fd4862010383ff17ffd6cc4ffac1c0";

/// Puts the fonts for Japanese and Chinese in `vscode/media/fonts`,
/// fetched with `curl` the first time and kept in `target/fonts`. A file
/// whose SHA-256 is not the one pinned is refused.
fn cjk_fonts(root: &Path) -> Result<(), String> {
    use sha2::Digest;
    let cache = root.join("target").join("fonts");
    let dest = root.join("vscode").join("media").join("fonts");
    for dir in [&cache, &dest] {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let digest = |bytes: &[u8]| {
        sha2::Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    };
    for (name, source, sha) in CJK_FONTS {
        let cached = cache.join(name);
        let good = |path: &Path| std::fs::read(path).is_ok_and(|b| digest(&b) == *sha);
        if !good(&cached) {
            let url = format!(
                "https://raw.githubusercontent.com/google/fonts/{GOOGLE_FONTS_COMMIT}/{source}"
            );
            let path = cached.to_string_lossy().into_owned();
            run_in(root, "curl", &["-sSfL", "-o", &path, &url])?;
            if !good(&cached) {
                let _ = std::fs::remove_file(&cached);
                return Err(format!("{url} is not the file pinned (SHA-256 {sha})"));
            }
        }
        let target = dest.join(name);
        if !good(&target) {
            std::fs::copy(&cached, &target).map_err(|e| format!("cannot copy {name}: {e}"))?;
        }
    }
    println!(
        "put the fonts for Japanese and Chinese in {}",
        dest.display()
    );
    Ok(())
}

/// Builds the test build into the extension and runs the extension's tests
/// in a VS Code downloaded for them (`vscode/test/run.py`, with the Python
/// `GOSSIA_PYTHON` names). No Node.js is needed: VS Code runs the suite in its
/// own extension host.
fn vscode_test(root: &Path) -> Result<(), String> {
    vscode(root, true)?;
    let script = root.join("vscode").join("test").join("run.py");
    run_in(root, &python(), &[&script.to_string_lossy()])
}

/// The Python to run the helper scripts with: `GOSSIA_PYTHON`, or the usual
/// name of the system's.
fn python() -> String {
    std::env::var("GOSSIA_PYTHON")
        .unwrap_or_else(|_| if cfg!(windows) { "python" } else { "python3" }.to_owned())
}

/// Packs the extension with `packaging/vscode/pack-vsix.py` (DESIGN.md, Web
/// port, 0.3.0, In VS Code), which writes the zip as `vsce` would:
/// the release build of the wasm version copied in, the licence and the icon
/// (drawn as the window's is) put beside `package.json`, and the result in
/// `target/vsix`. The extension's version must be the workspace's. Gives the
/// file.
fn vsix(root: &Path) -> Result<PathBuf, String> {
    let dir = root.join("vscode");
    let manifest = std::fs::read_to_string(dir.join("package.json"))
        .map_err(|e| format!("cannot read vscode/package.json: {e}"))?;
    let version = locked_version(root, "gossia-app")?;
    if !manifest.contains(&format!("\"version\": \"{version}\"")) {
        return Err(format!(
            "vscode/package.json must have the workspace's version, {version}"
        ));
    }
    vscode(root, false)?;
    std::fs::copy(root.join("LICENSE"), dir.join("LICENSE"))
        .map_err(|e| format!("cannot copy LICENSE: {e}"))?;
    icons::write_one(128, &dir.join("media").join("icon.png"))?;
    let out = root.join("target").join("vsix");
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let file = out.join(format!("gossia-{version}.vsix"));
    let script = root.join("packaging").join("vscode").join("pack-vsix.py");
    run_in(
        root,
        &python(),
        &[
            &script.to_string_lossy(),
            &dir.to_string_lossy(),
            &file.to_string_lossy(),
        ],
    )?;
    println!("packed {}", file.display());
    Ok(file)
}

/// Runs a program in a folder, failing when it fails.
fn run_in(dir: &Path, program: &str, args: &[&str]) -> Result<(), String> {
    println!("+ {program} {}", args.join(" "));
    let status = Command::new(program)
        .args(args)
        .current_dir(dir)
        .status()
        .map_err(|e| format!("cannot run {program}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{program} {} failed", args.join(" ")))
    }
}

/// Builds the test build of the wasm version and runs the harness page in
/// headless Chromium with the Playwright script (DESIGN.md, Web port, 0.3.0,
/// Testing). `GOSSIA_PYTHON` names the Python to use.
fn web_test(root: &Path) -> Result<(), String> {
    let out = web(root, true)?;
    let python = python();
    let script = root
        .join("crates")
        .join("gossia-web")
        .join("harness")
        .join("run.py");
    println!("+ {python} {} {}", script.display(), out.display());
    let status = Command::new(&python)
        .arg(&script)
        .arg(&out)
        .status()
        .map_err(|e| format!("cannot run {python}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("the web harness failed".to_owned())
    }
}

/// The version of a package in the workspace's `Cargo.lock`.
fn locked_version(root: &Path, package: &str) -> Result<String, String> {
    let lock = std::fs::read_to_string(root.join("Cargo.lock"))
        .map_err(|e| format!("cannot read Cargo.lock: {e}"))?;
    let mut lines = lock.lines();
    while let Some(line) = lines.next() {
        if line.trim() == format!("name = \"{package}\"")
            && let Some(version) = lines
                .next()
                .and_then(|l| l.trim().strip_prefix("version = \""))
                .and_then(|v| v.strip_suffix('"'))
        {
            return Ok(version.to_owned());
        }
    }
    Err(format!("{package} is not in Cargo.lock"))
}

fn test(root: &Path) -> Result<(), String> {
    cargo(root, &["test", "--workspace"])?;
    cargo(
        root,
        &["test", "--manifest-path", "tools/Cargo.toml", "--workspace"],
    )
}

/// The tests that start the real application, which needs a display. On Linux
/// without one they run under `xvfb-run`.
fn e2e(root: &Path) -> Result<(), String> {
    let args = [
        "test",
        "-p",
        "gossia-app",
        "--features",
        "test-control",
        "--test",
        "e2e",
    ];
    let headless = cfg!(target_os = "linux")
        && std::env::var_os("DISPLAY").is_none_or(|d| d.is_empty())
        && std::env::var_os("WAYLAND_DISPLAY").is_none_or(|d| d.is_empty());
    if !headless {
        return cargo(root, &args);
    }
    // Build first, so that the virtual display is not up while compiling.
    cargo(
        root,
        &[
            "test",
            "--no-run",
            "-p",
            "gossia-app",
            "--features",
            "test-control",
        ],
    )?;
    let program = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    println!("+ xvfb-run -a cargo {}", args.join(" "));
    let status = Command::new("xvfb-run")
        .arg("-a")
        .arg(program)
        .args(args)
        .current_dir(root)
        .status()
        .map_err(|e| format!("cannot run xvfb-run (is it installed?): {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("the end-to-end tests failed".to_owned())
    }
}

/// The screenshot comparisons. They render with wgpu, so they need a graphics
/// adapter (on Linux CI, Mesa's software Vulkan driver, lavapipe).
fn snapshots(root: &Path) -> Result<(), String> {
    let program = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let args = ["test", "-p", "gossia-app", "--test", "snapshots"];
    println!("+ GOSSIA_SNAPSHOTS=1 cargo {}", args.join(" "));
    let status = Command::new(program)
        .args(args)
        .env("GOSSIA_SNAPSHOTS", "1")
        .current_dir(root)
        .status()
        .map_err(|e| format!("cannot run cargo: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("the snapshot tests failed".to_owned())
    }
}
