//! Writes the main third-party libraries, with the versions Cargo.lock has, for
//! the About page of the Options dialog (`options.rs`). Reading them from the
//! lock file means the page cannot fall behind an update.
//!
//! For Windows (with the MSVC linker, or lld-link when cross-compiling) it
//! also links in the executable's icon and version details, as a `.res` file
//! written here: no resource compiler is needed.

use std::{env, fs, path::Path};

#[path = "src/app_icon.rs"]
#[allow(dead_code)]
mod app_icon;

/// The libraries shown: the package in Cargo.lock, the name shown, and its
/// licence (from the package's own Cargo.toml).
const LIBRARIES: &[(&str, &str, &str)] = &[
    ("egui", "egui", "MIT OR Apache-2.0"),
    ("eframe", "eframe", "MIT OR Apache-2.0"),
    ("egui_extras", "egui_extras", "MIT OR Apache-2.0"),
    ("egui-file-dialog", "egui-file-dialog", "MIT"),
    ("winit", "winit", "Apache-2.0"),
    ("rustpython", "RustPython", "MIT"),
    ("rustpython-ruff_python_parser", "Ruff Python parser", "MIT"),
    ("serde_json", "serde_json", "MIT OR Apache-2.0"),
    ("quick-xml", "quick-xml", "MIT"),
    ("regex", "regex", "MIT OR Apache-2.0"),
    ("chrono", "chrono", "MIT OR Apache-2.0"),
    ("indexmap", "indexmap", "Apache-2.0 OR MIT"),
    ("zip", "zip", "MIT"),
    ("sha2", "sha2", "MIT OR Apache-2.0"),
    ("skrifa", "skrifa", "MIT OR Apache-2.0"),
];

/// A package of Cargo.lock: its name, version and dependencies as written there
/// (`"name"`, or `"name version"` when the lock has more than one version).
struct Package<'a> {
    name: &'a str,
    version: &'a str,
    dependencies: Vec<&'a str>,
}

fn quoted(line: &str) -> Option<&str> {
    let start = line.find('"')? + 1;
    let end = start + line[start..].find('"')?;
    Some(&line[start..end])
}

fn packages(lock: &str) -> Vec<Package<'_>> {
    lock.split("[[package]]")
        .skip(1)
        .filter_map(|block| {
            let mut name = None;
            let mut version = None;
            let mut dependencies = Vec::new();
            let mut in_dependencies = false;
            for line in block.lines() {
                let line = line.trim();
                if in_dependencies {
                    if line.starts_with(']') {
                        in_dependencies = false;
                    } else if let Some(d) = quoted(line) {
                        dependencies.push(d);
                    }
                } else if line.starts_with("name = ") {
                    name = quoted(line);
                } else if line.starts_with("version = ") {
                    version = quoted(line);
                } else if line.starts_with("dependencies = [") {
                    in_dependencies = true;
                }
            }
            Some(Package {
                name: name?,
                version: version?,
                dependencies,
            })
        })
        .collect()
}

/// The version of `name` that Gossia uses: the only one in the lock, or, when
/// there are several, the one a Gossia crate names.
fn version<'a>(packages: &[Package<'a>], name: &str) -> &'a str {
    let versions: Vec<&str> = packages
        .iter()
        .filter(|p| p.name == name)
        .map(|p| p.version)
        .collect();
    match versions.as_slice() {
        [] => panic!("{name} is not in Cargo.lock: update LIBRARIES in build.rs"),
        [only] => only,
        _ => packages
            .iter()
            .filter(|p| p.name.starts_with("gossia"))
            .flat_map(|p| &p.dependencies)
            .find_map(|d| {
                let (n, v) = d.split_once(' ')?;
                (n == name).then(|| v.split(' ').next().unwrap_or(v))
            })
            .unwrap_or_else(|| panic!("no Gossia crate names a version of {name}")),
    }
}

fn main() {
    let manifest = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let lock = Path::new(&manifest).join("../../Cargo.lock");
    println!("cargo:rerun-if-changed={}", lock.display());
    println!("cargo:rerun-if-changed=build.rs");
    let text = fs::read_to_string(&lock).expect("Cargo.lock of the workspace");
    let packages = packages(&text);
    let mut out = String::from(
        "/// The main third-party libraries: name, version and licence.\n\
         pub const LIBRARIES: &[(&str, &str, &str)] = &[\n",
    );
    for (package, shown, licence) in LIBRARIES {
        let version = version(&packages, package);
        out += &format!("    ({shown:?}, {version:?}, {licence:?}),\n");
    }
    out += "];\n";
    let out_dir = env::var("OUT_DIR").expect("OUT_DIR");
    fs::write(Path::new(&out_dir).join("libraries.rs"), out).expect("write libraries.rs");

    println!("cargo:rerun-if-changed=src/app_icon.rs");
    let windows = env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");
    let msvc = env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    if windows && msvc {
        let version = env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION");
        let res = Path::new(&out_dir).join("gossia.res");
        fs::write(&res, app_icon::windows_resources(&version)).expect("write gossia.res");
        println!("cargo:rustc-link-arg-bins={}", res.display());
    }
}
