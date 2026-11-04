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
        Some("icons") => match args.get(1) {
            Some(dir) => icons::write(Path::new(dir)),
            None => Err("icons needs the folder to write to".to_owned()),
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
    if std::env::var_os("CLIOTREE_VENDOR_DIR").is_some_and(|v| !v.is_empty()) {
        println!("CLIOTREE_VENDOR_DIR is set; that directory was left alone");
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
            "cliotree-app",
            "--all-targets",
            "--features",
            "test-control",
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
        "cliotree-app",
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
            "cliotree-app",
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
    let args = ["test", "-p", "cliotree-app", "--test", "snapshots"];
    println!("+ CLIOTREE_SNAPSHOTS=1 cargo {}", args.join(" "));
    let status = Command::new(program)
        .args(args)
        .env("CLIOTREE_SNAPSHOTS", "1")
        .current_dir(root)
        .status()
        .map_err(|e| format!("cannot run cargo: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("the snapshot tests failed".to_owned())
    }
}
