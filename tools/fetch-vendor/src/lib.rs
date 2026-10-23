//! Fetches and verifies the pinned dependencies in `vendor/`.
//!
//! The pins are in `vendor.lock` at the repository root. Each item is fetched
//! at an exact commit, optionally with a sparse checkout, and is marked complete
//! by a `.complete` stamp that is written only after everything succeeded. See
//! DESIGN.md, "Build and dependencies".

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

/// The name of the stamp file written into an item's directory once it is complete.
pub const STAMP: &str = ".complete";

/// The parsed contents of `vendor.lock`.
#[derive(Debug, Deserialize)]
pub struct Lock {
    pub version: u32,
    #[serde(rename = "item")]
    pub items: Vec<Item>,
}

/// One pinned dependency.
#[derive(Debug, Clone, Deserialize)]
pub struct Item {
    pub name: String,
    pub url: String,
    pub rev: String,
    #[serde(default)]
    pub sparse: Vec<String>,
    /// A patch file, relative to the repository root, applied to the checkout
    /// after it has been verified (see DESIGN.md, "Build and dependencies").
    #[serde(default)]
    pub patch: Option<String>,
}

/// Whether an item in `vendor/` matches its pin.
#[derive(Debug, PartialEq, Eq)]
pub enum State {
    /// There is no directory for the item.
    Missing,
    /// The directory exists but has no stamp: an interrupted fetch.
    Incomplete,
    /// The stamp names a different commit than the pin.
    Stale(String),
    /// The stamp matches the pin.
    Current,
}

/// Parses and validates the text of `vendor.lock`.
pub fn parse_lock(text: &str) -> Result<Lock, String> {
    let lock: Lock = toml::from_str(text).map_err(|e| format!("vendor.lock: {e}"))?;
    if lock.version != 1 {
        return Err(format!("vendor.lock: unsupported version {}", lock.version));
    }
    let mut seen = Vec::new();
    for item in &lock.items {
        let name = &item.name;
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        {
            return Err(format!("vendor.lock: invalid item name {name:?}"));
        }
        if seen.contains(name) {
            return Err(format!("vendor.lock: duplicate item {name:?}"));
        }
        seen.push(name.clone());
        if !is_full_hash(&item.rev) {
            return Err(format!(
                "vendor.lock: item {name:?}: rev must be a full 40-character commit hash, not {:?}",
                item.rev
            ));
        }
        if item.url.is_empty() {
            return Err(format!("vendor.lock: item {name:?} has no url"));
        }
        for path in &item.sparse {
            if !is_inside(path) {
                return Err(format!(
                    "vendor.lock: item {name:?}: invalid sparse path {path:?}"
                ));
            }
        }
        if let Some(path) = &item.patch
            && !is_inside(path)
        {
            return Err(format!(
                "vendor.lock: item {name:?}: invalid patch path {path:?}"
            ));
        }
    }
    Ok(lock)
}

/// A relative path that stays inside the directory it is relative to.
fn is_inside(path: &str) -> bool {
    let p = Path::new(path);
    !path.is_empty()
        && !path.starts_with(['/', '\\'])
        && !p.is_absolute()
        && !p
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
}

fn is_full_hash(rev: &str) -> bool {
    rev.len() == 40 && rev.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f'))
}

/// Finds the repository root: the nearest directory, starting at `start` and
/// going up, that contains `vendor.lock`.
pub fn find_root(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .find(|dir| dir.join("vendor.lock").is_file())
        .map(Path::to_path_buf)
}

/// The directory holding the vendored items: `$CLIOTREE_VENDOR_DIR` if set,
/// otherwise `vendor/` under the repository root.
pub fn vendor_dir(root: &Path) -> PathBuf {
    match std::env::var_os("CLIOTREE_VENDOR_DIR") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => root.join("vendor"),
    }
}

/// What the stamp of a complete item holds: the pinned commit, with a
/// fingerprint of the sparse list and of the patch if the item has them, so that
/// changing either fetches the item again. Line endings of the patch do not
/// count, because a checkout of the patch file may have either.
pub fn identity(item: &Item, root: &Path) -> Result<String, String> {
    let mut id = item.rev.clone();
    if !item.sparse.is_empty() {
        id.push_str(&format!(
            "+sparse-{:016x}",
            fingerprint(
                item.sparse
                    .join(
                        "
"
                    )
                    .bytes()
            )
        ));
    }
    if let Some(patch) = &item.patch {
        let bytes = fs::read(root.join(patch))
            .map_err(|e| format!("{}: cannot read the patch {patch}: {e}", item.name))?;
        id.push_str(&format!(
            "+patch-{:016x}",
            fingerprint(bytes.into_iter().filter(|b| *b != b'\r'))
        ));
    }
    Ok(id)
}

/// FNV-1a. It only has to notice a change; it is not a security measure.
fn fingerprint(bytes: impl Iterator<Item = u8>) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// Compares an item's directory with its pin, without touching the network.
/// `root` is the repository root, where patch files are found.
pub fn state(item: &Item, vendor: &Path, root: &Path) -> State {
    let dir = vendor.join(&item.name);
    if !dir.is_dir() {
        return State::Missing;
    }
    let wanted = match identity(item, root) {
        Ok(id) => id,
        Err(e) => return State::Stale(e),
    };
    match fs::read_to_string(dir.join(STAMP)) {
        Err(_) => State::Incomplete,
        Ok(text) if text.trim() == wanted => State::Current,
        Ok(text) => State::Stale(text.trim().to_owned()),
    }
}

/// Fetches every item that is not current. Returns how many were fetched.
pub fn fetch_missing(lock: &Lock, vendor: &Path, root: &Path) -> Result<usize, String> {
    let mut fetched = 0;
    for item in &lock.items {
        if state(item, vendor, root) != State::Current {
            fetch_item(item, vendor, root)?;
            fetched += 1;
        }
    }
    Ok(fetched)
}

/// Fetches one item at its pinned commit, replacing whatever is there.
pub fn fetch_item(item: &Item, vendor: &Path, root: &Path) -> Result<(), String> {
    let identity = identity(item, root)?;
    let dir = vendor.join(&item.name);
    println!("vendor: fetching {} at {} ...", item.name, &item.rev[..12]);
    if dir.exists() {
        remove_dir_all_force(&dir)?;
    }
    fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;

    git(&dir, &["init", "-q"])?;
    // Very long paths exist in some repositories (the ruff fork has test file
    // names that exceed the Windows limit when checked out in full).
    git(&dir, &["config", "core.longpaths", "true"])?;
    git(&dir, &["config", "advice.detachedHead", "false"])?;
    // The same bytes on every platform, so that a patch applies the same way.
    git(&dir, &["config", "core.autocrlf", "false"])?;
    git(&dir, &["remote", "add", "origin", &item.url])?;
    if !item.sparse.is_empty() {
        git(&dir, &["sparse-checkout", "init", "--cone"])?;
        let mut args = vec!["sparse-checkout", "set"];
        args.extend(item.sparse.iter().map(String::as_str));
        git(&dir, &args)?;
    }
    git_network(&dir, &["fetch", "-q", "--depth", "1", "origin", &item.rev])?;
    git(&dir, &["checkout", "-q", "--detach", "FETCH_HEAD"])?;

    let head = git(&dir, &["rev-parse", "HEAD"])?;
    if head.trim() != item.rev {
        return Err(format!(
            "{}: checked out {} but vendor.lock pins {}",
            item.name,
            head.trim(),
            item.rev
        ));
    }
    if let Some(patch) = &item.patch {
        let patch_file = root.join(patch);
        let patch_file = patch_file.to_string_lossy();
        git(&dir, &["apply", "--ignore-whitespace", &patch_file])
            .map_err(|e| format!("{}: the patch does not apply: {e}", item.name))?;
    }
    fs::write(dir.join(STAMP), format!("{identity}\n"))
        .map_err(|e| format!("cannot write the stamp for {}: {e}", item.name))?;
    println!("vendor: {} is complete", item.name);
    Ok(())
}

/// Checks that an item is complete and that its checkout is at the pinned commit.
pub fn verify_item(item: &Item, vendor: &Path, root: &Path) -> Result<(), String> {
    match state(item, vendor, root) {
        State::Current => {}
        State::Missing => return Err(format!("{}: not fetched", item.name)),
        State::Incomplete => return Err(format!("{}: incomplete (no stamp)", item.name)),
        State::Stale(found) => {
            return Err(format!(
                "{}: stamp is {found} but vendor.lock pins {}",
                item.name, item.rev
            ));
        }
    }
    let head = git(&vendor.join(&item.name), &["rev-parse", "HEAD"])?;
    if head.trim() != item.rev {
        return Err(format!(
            "{}: checked out {} but vendor.lock pins {}",
            item.name,
            head.trim(),
            item.rev
        ));
    }
    if let Some(patch) = &item.patch {
        // The patch is applied if it can be taken back out again.
        let patch_file = root.join(patch);
        let patch_file = patch_file.to_string_lossy();
        git(
            &vendor.join(&item.name),
            &[
                "apply",
                "--ignore-whitespace",
                "--reverse",
                "--check",
                &patch_file,
            ],
        )
        .map_err(|e| format!("{}: the patch {patch} is not applied: {e}", item.name))?;
    }
    Ok(())
}

/// The vendor items the workspace declares it needs, from
/// `[workspace.metadata.cliotree] vendor = [...]` in the root `Cargo.toml`.
pub fn required_from_manifest(manifest_text: &str) -> Result<Vec<String>, String> {
    let value: toml::Value =
        toml::from_str(manifest_text).map_err(|e| format!("Cargo.toml: {e}"))?;
    let list = value
        .get("workspace")
        .and_then(|w| w.get("metadata"))
        .and_then(|m| m.get("cliotree"))
        .and_then(|c| c.get("vendor"));
    match list {
        None => Ok(Vec::new()),
        Some(toml::Value::Array(items)) => items
            .iter()
            .map(|v| {
                v.as_str().map(str::to_owned).ok_or_else(|| {
                    "Cargo.toml: workspace.metadata.cliotree.vendor must be a list of names"
                        .to_owned()
                })
            })
            .collect(),
        Some(_) => {
            Err("Cargo.toml: workspace.metadata.cliotree.vendor must be a list of names".to_owned())
        }
    }
}

/// Fetches the vendor items that the workspace declares it needs, if they are
/// missing or out of date. Cargo cannot do this itself: it needs every path
/// dependency to exist before any build script runs, so the fetch has to happen
/// before Cargo is started (see DESIGN.md, "Build and dependencies").
pub fn ensure_required(root: &Path) -> Result<(), String> {
    let manifest =
        fs::read_to_string(root.join("Cargo.toml")).map_err(|e| format!("Cargo.toml: {e}"))?;
    let required = required_from_manifest(&manifest)?;
    if required.is_empty() {
        return Ok(());
    }
    let lock_text =
        fs::read_to_string(root.join("vendor.lock")).map_err(|e| format!("vendor.lock: {e}"))?;
    let lock = parse_lock(&lock_text)?;
    let vendor = vendor_dir(root);
    for name in &required {
        let item = lock.items.iter().find(|i| &i.name == name).ok_or_else(|| {
            format!("Cargo.toml needs vendor item {name:?}, which is not in vendor.lock")
        })?;
        if state(item, &vendor, root) != State::Current {
            fetch_item(item, &vendor, root)?;
        }
    }
    Ok(())
}

/// Returns the text of `vendor.lock` with the pin for `item` changed to `rev`.
pub fn update_rev(lock_text: &str, item: &str, rev: &str) -> Result<String, String> {
    if !is_full_hash(rev) {
        return Err(format!(
            "{rev:?} is not a full 40-character lowercase commit hash"
        ));
    }
    let mut out = Vec::new();
    let mut in_item = false;
    let mut changed = false;
    for line in lock_text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("[[item]]") {
            in_item = false;
        }
        if trimmed.starts_with("name") && trimmed.contains(&format!("\"{item}\"")) {
            in_item = true;
        }
        if in_item && !changed && trimmed.starts_with("rev") {
            let indent = &line[..line.len() - trimmed.len()];
            let before_eq = trimmed.split('=').next().unwrap_or("rev ");
            out.push(format!("{indent}{before_eq}= \"{rev}\""));
            changed = true;
            continue;
        }
        out.push(line.to_owned());
    }
    if !changed {
        return Err(format!("no item named {item:?} in vendor.lock"));
    }
    let mut text = out.join("\n");
    text.push('\n');
    parse_lock(&text)?;
    Ok(text)
}

/// Runs a `git` command in `dir` and returns its standard output.
fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    run_git(dir, &[], args)
}

/// Runs a `git` command that uses the network.
fn git_network(dir: &Path, args: &[&str]) -> Result<String, String> {
    // Git for Windows is not always configured with a certificate bundle for its
    // default TLS backend; the Windows certificate store is always present.
    let config: &[&str] = if cfg!(windows) {
        &["-c", "http.sslBackend=schannel"]
    } else {
        &[]
    };
    run_git(dir, config, args)
}

fn run_git(dir: &Path, config: &[&str], args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(config)
        .args(args)
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|e| format!("cannot run git (is it installed and on the PATH?): {e}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(format!(
            "git {} failed in {}: {}",
            args.join(" "),
            dir.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

/// Removes a directory tree, including read-only files such as git's pack files.
pub fn remove_dir_all_force(dir: &Path) -> Result<(), String> {
    if fs::remove_dir_all(dir).is_ok() {
        return Ok(());
    }
    clear_readonly(dir);
    fs::remove_dir_all(dir).map_err(|e| format!("cannot remove {}: {e}", dir.display()))
}

fn clear_readonly(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if let Ok(meta) = fs::symlink_metadata(&path) {
            let mut perms = meta.permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            perms.set_readonly(false);
            let _ = fs::set_permissions(&path, perms);
            if meta.is_dir() {
                clear_readonly(&path);
            }
        }
    }
}

/// The `vendor` subcommands shared by `cargo xtask vendor` and the standalone
/// binary: `fetch`, `status`, `verify`, and `update <item> <rev>`.
pub fn run_vendor_command(args: &[String]) -> Result<(), String> {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let root = find_root(&cwd)
        .ok_or_else(|| "vendor.lock not found in this directory or any parent".to_owned())?;
    let lock_path = root.join("vendor.lock");
    let lock_text =
        fs::read_to_string(&lock_path).map_err(|e| format!("{}: {e}", lock_path.display()))?;
    let lock = parse_lock(&lock_text)?;
    let vendor = vendor_dir(&root);

    match args.first().map(String::as_str) {
        Some("fetch") => {
            let n = fetch_missing(&lock, &vendor, &root)?;
            if n == 0 {
                println!("vendor: everything is already fetched");
            }
            Ok(())
        }
        Some("status") => {
            for item in &lock.items {
                let text = match state(item, &vendor, &root) {
                    State::Current => "current".to_owned(),
                    State::Missing => "missing".to_owned(),
                    State::Incomplete => "incomplete".to_owned(),
                    State::Stale(found) => format!("stale (has {})", &found[..found.len().min(12)]),
                };
                println!("{:<12} {}  {}", item.name, &item.rev[..12], text);
            }
            Ok(())
        }
        Some("verify") => {
            let mut failed = false;
            for item in &lock.items {
                match verify_item(item, &vendor, &root) {
                    Ok(()) => println!("{:<12} ok", item.name),
                    Err(e) => {
                        failed = true;
                        eprintln!("{e}");
                    }
                }
            }
            if failed {
                Err("vendor verification failed".to_owned())
            } else {
                Ok(())
            }
        }
        Some("update") => {
            let (Some(item), Some(rev)) = (args.get(1), args.get(2)) else {
                return Err("usage: vendor update <item> <rev>".to_owned());
            };
            let new_text = update_rev(&lock_text, item, rev)?;
            fs::write(&lock_path, new_text).map_err(|e| format!("{}: {e}", lock_path.display()))?;
            println!("vendor.lock updated; review the change, then run `vendor fetch`");
            Ok(())
        }
        _ => Err("usage: vendor <fetch|status|verify|update <item> <rev>>".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REV_A: &str = "e5554e23abfbaee85508c32fc22e5074f88aebdd";
    const REV_B: &str = "9290a8849542134d83f77e9360e7f0b93897fd93";

    fn lock_text() -> String {
        format!(
            "version = 1\n\n[[item]]\nname = \"one\"\nurl = \"https://example.com/one\"\nrev = \"{REV_A}\"\n\n\
             [[item]]\nname   = \"two\"\nurl    = \"https://example.com/two\"\nrev    = \"{REV_B}\"\nsparse = [\"crates/x\"]\n"
        )
    }

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("fetch-vendor-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn parses_a_valid_lock() {
        let lock = parse_lock(&lock_text()).unwrap();
        assert_eq!(lock.items.len(), 2);
        assert_eq!(lock.items[1].sparse, ["crates/x"]);
    }

    #[test]
    fn the_real_lock_file_is_valid() {
        let text = include_str!("../../../vendor.lock");
        let lock = parse_lock(text).unwrap();
        assert!(lock.items.iter().any(|i| i.name == "rustpython"));
        assert!(lock.items.iter().any(|i| i.name == "ruff"));
    }

    #[test]
    fn rejects_a_branch_name_as_rev() {
        let text = lock_text().replace(REV_A, "main");
        assert!(parse_lock(&text).unwrap_err().contains("40-character"));
    }

    #[test]
    fn rejects_an_abbreviated_hash() {
        let text = lock_text().replace(REV_A, &REV_A[..12]);
        assert!(parse_lock(&text).is_err());
    }

    #[test]
    fn rejects_duplicate_names_and_bad_names() {
        let dup = lock_text().replace("\"two\"", "\"one\"");
        assert!(parse_lock(&dup).unwrap_err().contains("duplicate"));
        let bad = lock_text().replace("\"two\"", "\"../two\"");
        assert!(parse_lock(&bad).unwrap_err().contains("invalid item name"));
    }

    #[test]
    fn rejects_escaping_sparse_paths() {
        let text = lock_text().replace("crates/x", "../outside");
        assert!(parse_lock(&text).unwrap_err().contains("sparse"));
    }

    #[test]
    fn rejects_an_unknown_version() {
        let text = lock_text().replace("version = 1", "version = 2");
        assert!(parse_lock(&text).unwrap_err().contains("version"));
    }

    #[test]
    fn update_changes_only_the_named_item() {
        let new_rev = "0123456789abcdef0123456789abcdef01234567";
        let updated = update_rev(&lock_text(), "two", new_rev).unwrap();
        let lock = parse_lock(&updated).unwrap();
        assert_eq!(lock.items[0].rev, REV_A);
        assert_eq!(lock.items[1].rev, new_rev);
        assert_eq!(lock.items[1].sparse, ["crates/x"]);
    }

    #[test]
    fn update_rejects_unknown_items_and_bad_revs() {
        assert!(
            update_rev(&lock_text(), "three", REV_A)
                .unwrap_err()
                .contains("no item")
        );
        assert!(update_rev(&lock_text(), "one", "main").is_err());
        assert!(update_rev(&lock_text(), "one", &REV_A.to_uppercase()).is_err());
    }

    #[test]
    fn state_follows_the_stamp() {
        let vendor = scratch("state");
        let lock = parse_lock(&lock_text()).unwrap();
        let item = &lock.items[0];
        assert_eq!(state(item, &vendor, &vendor), State::Missing);

        fs::create_dir_all(vendor.join("one")).unwrap();
        assert_eq!(state(item, &vendor, &vendor), State::Incomplete);

        fs::write(vendor.join("one").join(STAMP), format!("{REV_B}\n")).unwrap();
        assert_eq!(
            state(item, &vendor, &vendor),
            State::Stale(REV_B.to_owned())
        );

        fs::write(vendor.join("one").join(STAMP), format!("{REV_A}\n")).unwrap();
        assert_eq!(state(item, &vendor, &vendor), State::Current);
        let _ = fs::remove_dir_all(&vendor);
    }

    #[test]
    fn a_patch_path_must_stay_inside_the_repository() {
        let ok = lock_text().replace(
            "sparse = [\"crates/x\"]",
            "patch = \"vendor-patches/two.patch\"",
        );
        assert_eq!(
            parse_lock(&ok).unwrap().items[1].patch.as_deref(),
            Some("vendor-patches/two.patch")
        );
        for bad in ["../two.patch", "/etc/passwd", ""] {
            let text =
                lock_text().replace("sparse = [\"crates/x\"]", &format!("patch = \"{bad}\""));
            assert!(
                parse_lock(&text).unwrap_err().contains("patch path"),
                "{bad}"
            );
        }
    }

    #[test]
    fn the_identity_of_a_patched_item_follows_the_patch_and_ignores_line_endings() {
        let root = scratch("identity");
        let item = Item {
            name: "one".into(),
            url: "u".into(),
            rev: REV_A.into(),
            sparse: Vec::new(),
            patch: Some("p.patch".into()),
        };
        assert!(identity(&item, &root).unwrap_err().contains("p.patch"));
        fs::write(root.join("p.patch"), "a\nb\n").unwrap();
        let lf = identity(&item, &root).unwrap();
        assert!(lf.starts_with(&format!("{REV_A}+patch-")), "{lf}");
        fs::write(root.join("p.patch"), "a\r\nb\r\n").unwrap();
        assert_eq!(identity(&item, &root).unwrap(), lf);
        fs::write(root.join("p.patch"), "a\nc\n").unwrap();
        assert_ne!(identity(&item, &root).unwrap(), lf);
        let plain = Item {
            patch: None,
            ..item
        };
        assert_eq!(identity(&plain, &root).unwrap(), REV_A);
        let _ = fs::remove_dir_all(&root);
    }

    fn git_in(dir: &Path, args: &[&str]) -> String {
        run_git(
            dir,
            &["-c", "user.name=t", "-c", "user.email=t@example.com"],
            args,
        )
        .unwrap()
    }

    /// Fetches from a repository made on the spot, applies a patch, and checks
    /// that the item is current, verifies, and goes stale when the patch changes.
    #[test]
    fn a_patched_item_is_fetched_patched_verified_and_refetched_when_the_patch_changes() {
        let base = scratch("patched");
        let origin = base.join("origin");
        fs::create_dir_all(&origin).unwrap();
        git_in(&origin, &["init", "-q"]);
        fs::write(
            origin.join("Cargo.toml"),
            "dep = \"git\"
other = 1
",
        )
        .unwrap();
        git_in(&origin, &["add", "."]);
        git_in(&origin, &["commit", "-q", "-m", "first"]);
        let rev = git_in(&origin, &["rev-parse", "HEAD"]).trim().to_owned();

        let root = base.join("root");
        fs::create_dir_all(root.join("patches")).unwrap();
        let patch = "diff --git a/Cargo.toml b/Cargo.toml
--- a/Cargo.toml
+++ b/Cargo.toml
@@ -1,2 +1,2 @@
-dep = \"git\"
+dep = \"path\"
 other = 1
";
        fs::write(root.join("patches/one.patch"), patch).unwrap();
        let item = Item {
            name: "one".into(),
            url: origin.to_string_lossy().into_owned(),
            rev,
            sparse: Vec::new(),
            patch: Some("patches/one.patch".into()),
        };
        let vendor = root.join("vendor");
        assert_eq!(state(&item, &vendor, &root), State::Missing);
        fetch_item(&item, &vendor, &root).unwrap();
        assert_eq!(state(&item, &vendor, &root), State::Current);
        let text = fs::read_to_string(vendor.join("one/Cargo.toml")).unwrap();
        assert_eq!(
            text,
            "dep = \"path\"
other = 1
"
        );
        verify_item(&item, &vendor, &root).unwrap();

        // A different patch is a different item.
        fs::write(
            root.join("patches/one.patch"),
            format!(
                "{patch}
"
            ),
        )
        .unwrap();
        assert!(matches!(state(&item, &vendor, &root), State::Stale(_)));

        // A patch that does not apply stops the fetch.
        fs::write(
            root.join("patches/one.patch"),
            patch.replace("-dep = \"git\"", "-dep = \"nope\""),
        )
        .unwrap();
        let err = fetch_item(&item, &vendor, &root).unwrap_err();
        assert!(err.contains("does not apply"), "{err}");
        let _ = remove_dir_all_force(&base);
    }

    #[test]
    fn find_root_walks_up_to_the_lock_file() {
        let root = scratch("root");
        fs::write(root.join("vendor.lock"), lock_text()).unwrap();
        let nested = root.join("a").join("b");
        fs::create_dir_all(&nested).unwrap();
        assert_eq!(find_root(&nested), Some(root.clone()));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn reads_required_items_from_the_workspace_manifest() {
        let none = "[workspace]
members = []
";
        assert!(required_from_manifest(none).unwrap().is_empty());
        let some = "[workspace]
members = []
[workspace.metadata.cliotree]
vendor = [\"rustpython\", \"ruff\"]
";
        assert_eq!(
            required_from_manifest(some).unwrap(),
            ["rustpython", "ruff"]
        );
        let bad = "[workspace.metadata.cliotree]
vendor = \"rustpython\"
";
        assert!(required_from_manifest(bad).is_err());
    }

    #[test]
    fn the_real_manifest_parses() {
        let text = include_str!("../../../Cargo.toml");
        required_from_manifest(text).unwrap();
    }

    #[test]
    fn force_removal_handles_read_only_files() {
        let dir = scratch("readonly");
        let file = dir.join("sub").join("pack.idx");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, b"x").unwrap();
        let mut perms = fs::metadata(&file).unwrap().permissions();
        perms.set_readonly(true);
        fs::set_permissions(&file, perms).unwrap();
        remove_dir_all_force(&dir).unwrap();
        assert!(!dir.exists());
    }
}
