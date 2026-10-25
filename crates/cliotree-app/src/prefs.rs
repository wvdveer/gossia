//! What belongs to the user rather than to any workbook (DESIGN.md, "Settings
//! and limits", "Where preferences live", and "Trust model"): the application
//! preferences, the recent files, and the record of the workbooks the user has
//! trusted. They are kept in one per-user configuration directory, as two JSON
//! files, `preferences.json` and `trust.json`.
//!
//! Each file carries a version number. What a newer version wrote and this one
//! does not know is kept and written back unchanged, a setting that is missing
//! takes its default, and a value that cannot be used takes its default with a
//! warning. A file that cannot be read at all is moved aside, not overwritten.
//!
//! Another window may change the files while this one runs, so a change is made
//! by reading the file again, applying the change to what it holds, and writing
//! it back.

use std::path::{Path, PathBuf};

use cliotree_calc::{Ceilings, Fingerprint, Setting, TrustStore};
use serde_json::{Map, Value as Json, json};

use crate::file::{Limits, save_atomic};

/// The version of both files that this build writes.
pub const CONFIG_VERSION: u64 = 1;
pub const PREFERENCES_FILE: &str = "preferences.json";
pub const TRUST_FILE: &str = "trust.json";
/// How many recent files are listed.
pub const MAX_RECENT: usize = 10;
/// Names the directory instead of the operating system's usual place, for
/// tests and for running two configurations side by side.
pub const CONFIG_DIR_VARIABLE: &str = "CLIOTREE_CONFIG_DIR";

/// Where the files are, if anywhere.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    /// `None` keeps everything in memory, as the in-process tests do.
    pub dir: Option<PathBuf>,
}

impl Config {
    pub fn in_memory() -> Config {
        Config { dir: None }
    }

    pub fn at(dir: impl Into<PathBuf>) -> Config {
        Config {
            dir: Some(dir.into()),
        }
    }

    /// `CLIOTREE_CONFIG_DIR` if it is set, otherwise the operating system's
    /// usual place for application settings: `%APPDATA%\ClioTree` on Windows,
    /// `~/Library/Application Support/ClioTree` on macOS, and
    /// `$XDG_CONFIG_HOME/cliotree` (or `~/.config/cliotree`) elsewhere. In
    /// memory only if none of these can be found.
    pub fn standard() -> Config {
        let var = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty());
        if let Some(dir) = var(CONFIG_DIR_VARIABLE) {
            return Config::at(dir);
        }
        let dir = if cfg!(windows) {
            var("APPDATA").map(|d| PathBuf::from(d).join("ClioTree"))
        } else if cfg!(target_os = "macos") {
            var("HOME").map(|h| PathBuf::from(h).join("Library/Application Support/ClioTree"))
        } else {
            var("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .or_else(|| var("HOME").map(|h| PathBuf::from(h).join(".config")))
                .map(|d| d.join("cliotree"))
        };
        Config { dir }
    }

    fn path(&self, name: &str) -> Option<PathBuf> {
        self.dir.as_ref().map(|d| d.join(name))
    }

    /// The object in a file, or `None` if there is no file. A file that
    /// cannot be read is moved aside, with a warning, so that writing the
    /// file again later does not lose it.
    fn read(&self, name: &str, warnings: &mut Vec<String>) -> Option<Map<String, Json>> {
        let path = self.path(name)?;
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
            Err(e) => {
                warnings.push(format!("{} cannot be read: {e}", path.display()));
                return None;
            }
        };
        match serde_json::from_str::<Json>(&text) {
            Ok(Json::Object(object)) => Some(object),
            other => {
                let why = match other {
                    Err(e) => e.to_string(),
                    Ok(_) => "it is not a JSON object".to_owned(),
                };
                let aside = path.with_extension("json.unreadable");
                let moved = std::fs::rename(&path, &aside).is_ok();
                warnings.push(format!(
                    "{} cannot be read ({why}), so the defaults are used{}.",
                    path.display(),
                    if moved {
                        format!("; it was moved to {}", aside.display())
                    } else {
                        String::new()
                    }
                ));
                None
            }
        }
    }

    fn write(&self, name: &str, object: Map<String, Json>) -> Result<(), String> {
        let Some(path) = self.path(name) else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let mut text =
            serde_json::to_string_pretty(&Json::Object(object)).expect("JSON always serializes");
        text.push('\n');
        save_atomic(&path, text.as_bytes()).map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// The version a file says it has, which is kept if it is newer than ours.
fn version_of(object: &Map<String, Json>) -> u64 {
    object
        .get("version")
        .and_then(Json::as_u64)
        .unwrap_or(CONFIG_VERSION)
        .max(CONFIG_VERSION)
}

/// A whole number at `key` of `object`, at least `minimum`, or the default
/// with a warning.
fn number(
    object: Option<&Map<String, Json>>,
    section: &str,
    key: &str,
    minimum: u64,
    default: u64,
    warnings: &mut Vec<String>,
) -> u64 {
    match object.and_then(|o| o.get(key)) {
        None => default,
        Some(value) => match value.as_u64() {
            Some(n) if n >= minimum => n,
            _ => {
                warnings.push(format!(
                    "In {PREFERENCES_FILE}, {section}.{key} is {value}, which cannot be used; \
                     the default {default} is used."
                ));
                default
            }
        },
    }
}

fn object_at<'a>(object: &'a Map<String, Json>, key: &str) -> Option<&'a Map<String, Json>> {
    object.get(key).and_then(Json::as_object)
}

/// The application preferences.
#[derive(Debug, Clone, PartialEq)]
pub struct Preferences {
    /// The most each workbook setting may be.
    pub ceilings: Ceilings,
    /// What a workbook file may hold when it is read.
    pub limits: Limits,
    /// The bounds of the undo history: entries, and a rough weight.
    pub history_entries: usize,
    pub history_weight: usize,
    /// The workbooks opened or saved last, the most recent first.
    pub recent: Vec<PathBuf>,
    /// The file as it was read, so that what this version does not know is
    /// written back.
    raw: Map<String, Json>,
}

impl Default for Preferences {
    fn default() -> Preferences {
        Preferences {
            ceilings: Ceilings::default(),
            limits: Limits::default(),
            history_entries: cliotree_calc::DEFAULT_MAX_ENTRIES,
            history_weight: cliotree_calc::DEFAULT_MAX_WEIGHT,
            recent: Vec::new(),
            raw: Map::new(),
        }
    }
}

impl Preferences {
    fn from_object(raw: Map<String, Json>, warnings: &mut Vec<String>) -> Preferences {
        let defaults = Preferences::default();
        let mut ceilings = Ceilings::default();
        let section = object_at(&raw, "ceilings");
        for setting in Setting::ALL {
            let value = number(
                section,
                "ceilings",
                setting.key(),
                setting.minimum(),
                setting.default_ceiling(),
                warnings,
            );
            ceilings.set(setting, value);
        }
        let files = object_at(&raw, "files");
        let limits = Limits {
            entries: number(
                files,
                "files",
                "max_zip_entries",
                1,
                defaults.limits.entries as u64,
                warnings,
            ) as usize,
            total: number(
                files,
                "files",
                "max_total_bytes",
                1,
                defaults.limits.total,
                warnings,
            ),
            entry: number(
                files,
                "files",
                "max_entry_bytes",
                1,
                defaults.limits.entry,
                warnings,
            ),
        };
        let undo = object_at(&raw, "undo");
        let history_entries = number(
            undo,
            "undo",
            "max_entries",
            1,
            defaults.history_entries as u64,
            warnings,
        ) as usize;
        let history_weight = number(
            undo,
            "undo",
            "max_weight",
            1,
            defaults.history_weight as u64,
            warnings,
        ) as usize;
        let recent = raw
            .get("recent_files")
            .and_then(Json::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Json::as_str)
                    .map(PathBuf::from)
                    .take(MAX_RECENT)
                    .collect()
            })
            .unwrap_or_default();
        Preferences {
            ceilings,
            limits,
            history_entries,
            history_weight,
            recent,
            raw,
        }
    }

    /// The file's object: what was read, with what this version knows set
    /// over it.
    fn to_object(&self) -> Map<String, Json> {
        let mut out = self.raw.clone();
        out.insert("version".to_owned(), json!(version_of(&self.raw)));
        let mut section = |key: &str, values: Vec<(&str, u64)>| {
            let mut object = object_at(&self.raw, key).cloned().unwrap_or_default();
            for (name, value) in values {
                object.insert(name.to_owned(), json!(value));
            }
            out.insert(key.to_owned(), Json::Object(object));
        };
        section(
            "ceilings",
            Setting::ALL
                .iter()
                .map(|s| (s.key(), self.ceilings.get(*s)))
                .collect(),
        );
        section(
            "files",
            vec![
                ("max_zip_entries", self.limits.entries as u64),
                ("max_total_bytes", self.limits.total),
                ("max_entry_bytes", self.limits.entry),
            ],
        );
        section(
            "undo",
            vec![
                ("max_entries", self.history_entries as u64),
                ("max_weight", self.history_weight as u64),
            ],
        );
        out.insert(
            "recent_files".to_owned(),
            Json::Array(
                self.recent
                    .iter()
                    .map(|p| Json::String(p.display().to_string()))
                    .collect(),
            ),
        );
        out
    }

    /// Puts a workbook at the top of the recent files.
    pub fn add_recent(&mut self, path: &Path) {
        let path = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        self.recent.retain(|p| !same_file(p, &path));
        self.recent.insert(0, path);
        self.recent.truncate(MAX_RECENT);
    }

    pub fn remove_recent(&mut self, path: &Path) {
        self.recent.retain(|p| !same_file(p, path));
    }
}

/// Paths that name the same file. Windows ignores case in file names.
fn same_file(a: &Path, b: &Path) -> bool {
    if cfg!(windows) {
        a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
    } else {
        a == b
    }
}

// ------------------------------------------------------------------- trust

fn trust_from_object(object: &Map<String, Json>, warnings: &mut Vec<String>) -> TrustStore {
    let mut store = TrustStore::default();
    let Some(workbooks) = object_at(object, "workbooks") else {
        return store;
    };
    for (id, record) in workbooks {
        let hash = record.get("hash").and_then(Json::as_str);
        let items = record.get("items").and_then(Json::as_object).map(|items| {
            items
                .iter()
                .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned())))
                .collect()
        });
        match (hash, items) {
            (Some(hash), Some(items)) => store.trust(
                id,
                Fingerprint {
                    hash: hash.to_owned(),
                    items,
                },
            ),
            _ => warnings.push(format!(
                "In {TRUST_FILE}, the record of workbook {id} cannot be read, so it is not \
                 trusted."
            )),
        }
    }
    store
}

fn trust_to_object(store: &TrustStore, raw: &Map<String, Json>) -> Map<String, Json> {
    let mut out = raw.clone();
    out.insert("version".to_owned(), json!(version_of(raw)));
    let workbooks: Map<String, Json> = store
        .records()
        .map(|(id, f)| (id.to_owned(), json!({ "hash": f.hash, "items": f.items })))
        .collect();
    out.insert("workbooks".to_owned(), Json::Object(workbooks));
    out
}

// -------------------------------------------------------------------- user

/// The user's configuration, as the window holds it.
#[derive(Debug, Clone, Default)]
pub struct User {
    pub config: Config,
    pub prefs: Preferences,
    /// The workbooks the user has trusted.
    pub trust: TrustStore,
    /// `trust.json` as it was read, for what this version does not know.
    trust_raw: Map<String, Json>,
}

impl User {
    /// Nothing on disk: the in-process tests, and the sample.
    pub fn in_memory() -> User {
        User::default()
    }

    /// Reads the files of `config`. A missing file is the defaults; the
    /// warnings are for the user.
    pub fn load(config: Config) -> (User, Vec<String>) {
        let mut warnings = Vec::new();
        if config.dir.is_none() {
            warnings.push(
                "There is no folder for the preferences, so trust decisions and recent files \
                 are kept only until the application ends."
                    .to_owned(),
            );
        }
        let mut user = User {
            config,
            ..User::default()
        };
        user.reload(&mut warnings);
        (user, warnings)
    }

    fn reload(&mut self, warnings: &mut Vec<String>) {
        if self.config.dir.is_none() {
            return;
        }
        self.prefs = match self.config.read(PREFERENCES_FILE, warnings) {
            Some(raw) => Preferences::from_object(raw, warnings),
            None => Preferences::default(),
        };
        match self.config.read(TRUST_FILE, warnings) {
            Some(raw) => {
                self.trust = trust_from_object(&raw, warnings);
                self.trust_raw = raw;
            }
            None => {
                self.trust = TrustStore::default();
                self.trust_raw = Map::new();
            }
        }
    }

    /// Changes the trust record: the file is read again, so that what another
    /// window recorded is kept, the change is made, and the file is written.
    /// The change is made in memory even if the file cannot be written.
    pub fn change_trust(&mut self, change: impl FnOnce(&mut TrustStore)) -> Result<(), String> {
        let mut ignored = Vec::new();
        if self.config.dir.is_some() {
            match self.config.read(TRUST_FILE, &mut ignored) {
                Some(raw) => {
                    self.trust = trust_from_object(&raw, &mut ignored);
                    self.trust_raw = raw;
                }
                None => {
                    self.trust = TrustStore::default();
                    self.trust_raw = Map::new();
                }
            }
        }
        change(&mut self.trust);
        self.config
            .write(TRUST_FILE, trust_to_object(&self.trust, &self.trust_raw))
    }

    /// Changes the preferences, in the same way as [`User::change_trust`].
    pub fn change_prefs(&mut self, change: impl FnOnce(&mut Preferences)) -> Result<(), String> {
        let mut ignored = Vec::new();
        if self.config.dir.is_some() {
            self.prefs = match self.config.read(PREFERENCES_FILE, &mut ignored) {
                Some(raw) => Preferences::from_object(raw, &mut ignored),
                None => Preferences::default(),
            };
        }
        change(&mut self.prefs);
        self.config.write(PREFERENCES_FILE, self.prefs.to_object())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::workbook_file::new_workbook_id;

    struct Folder(PathBuf);

    impl Folder {
        fn new(tag: &str) -> Folder {
            let path = std::env::temp_dir().join(format!(
                "cliotree-prefs-{tag}-{}-{}",
                std::process::id(),
                new_workbook_id()
            ));
            Folder(path)
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn fingerprint(hash: &str) -> Fingerprint {
        Fingerprint {
            hash: hash.to_owned(),
            items: BTreeMap::from([("formula S!a".to_owned(), "d1".to_owned())]),
        }
    }

    #[test]
    fn with_no_files_the_defaults_are_used_and_nothing_is_written() {
        let folder = Folder::new("none");
        let (user, warnings) = User::load(Config::at(&folder.0));
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(user.prefs, Preferences::default());
        assert_eq!(user.trust.ids().count(), 0);
        assert!(!folder.0.exists());
    }

    #[test]
    fn a_trust_decision_is_kept_on_disk_and_read_by_the_next_session() {
        let folder = Folder::new("trust");
        let (mut user, _) = User::load(Config::at(&folder.0));
        user.change_trust(|t| t.trust("book1", fingerprint("h1")))
            .unwrap();
        let (again, warnings) = User::load(Config::at(&folder.0));
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(again.trust, user.trust);
        let text = std::fs::read_to_string(folder.0.join(TRUST_FILE)).unwrap();
        assert!(text.contains("\"version\": 1"), "{text}");
        assert!(text.contains("\"book1\""), "{text}");
    }

    #[test]
    fn two_windows_do_not_lose_each_others_decisions() {
        let folder = Folder::new("two");
        let (mut first, _) = User::load(Config::at(&folder.0));
        let (mut second, _) = User::load(Config::at(&folder.0));
        first
            .change_trust(|t| t.trust("book1", fingerprint("h1")))
            .unwrap();
        second
            .change_trust(|t| t.trust("book2", fingerprint("h2")))
            .unwrap();
        let (third, _) = User::load(Config::at(&folder.0));
        let ids: Vec<_> = third.trust.ids().collect();
        assert_eq!(ids, ["book1", "book2"]);
    }

    #[test]
    fn preferences_round_trip_and_keep_what_this_version_does_not_know() {
        let folder = Folder::new("prefs");
        std::fs::create_dir_all(&folder.0).unwrap();
        std::fs::write(
            folder.0.join(PREFERENCES_FILE),
            r#"{
              "version": 3,
              "theme": "dark",
              "ceilings": {"formula_time_limit_ms": 5000, "future_ceiling": 7},
              "undo": {"max_entries": 50}
            }"#,
        )
        .unwrap();
        let (mut user, warnings) = User::load(Config::at(&folder.0));
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(user.prefs.ceilings.get(Setting::FormulaTimeLimit), 5000);
        assert_eq!(user.prefs.ceilings.get(Setting::MaxRange), 100_000_000);
        assert_eq!(user.prefs.history_entries, 50);

        user.change_prefs(|p| p.add_recent(Path::new("/books/a.clio")))
            .unwrap();
        let text = std::fs::read_to_string(folder.0.join(PREFERENCES_FILE)).unwrap();
        let written: Json = serde_json::from_str(&text).unwrap();
        assert_eq!(written["version"], 3);
        assert_eq!(written["theme"], "dark");
        assert_eq!(written["ceilings"]["future_ceiling"], 7);
        assert_eq!(written["ceilings"]["formula_time_limit_ms"], 5000);
        assert_eq!(written["undo"]["max_entries"], 50);
        assert_eq!(written["files"]["max_zip_entries"], 10_000);
        assert_eq!(written["recent_files"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn a_value_that_cannot_be_used_takes_its_default_with_a_warning() {
        let folder = Folder::new("bad");
        std::fs::create_dir_all(&folder.0).unwrap();
        std::fs::write(
            folder.0.join(PREFERENCES_FILE),
            r#"{"ceilings": {"max_range_length": -5}, "files": {"max_zip_entries": "lots"}}"#,
        )
        .unwrap();
        let (user, warnings) = User::load(Config::at(&folder.0));
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert!(warnings[0].contains("ceilings.max_range_length"));
        assert_eq!(user.prefs.ceilings.get(Setting::MaxRange), 100_000_000);
        assert_eq!(user.prefs.limits, Limits::default());
    }

    #[test]
    fn a_file_that_cannot_be_read_is_moved_aside_not_overwritten() {
        let folder = Folder::new("broken");
        std::fs::create_dir_all(&folder.0).unwrap();
        std::fs::write(folder.0.join(TRUST_FILE), "{ this is not").unwrap();
        let (mut user, warnings) = User::load(Config::at(&folder.0));
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("moved"), "{warnings:?}");
        assert_eq!(
            std::fs::read_to_string(folder.0.join("trust.json.unreadable")).unwrap(),
            "{ this is not"
        );
        user.change_trust(|t| t.trust("b", fingerprint("h")))
            .unwrap();
        let (again, warnings) = User::load(Config::at(&folder.0));
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(again.trust.is_recorded("b"));
    }

    #[test]
    fn recent_files_are_the_newest_first_without_repeats_and_at_most_ten() {
        let mut prefs = Preferences::default();
        for i in 0..12 {
            prefs.add_recent(Path::new(&format!("/books/{i}.clio")));
        }
        prefs.add_recent(Path::new("/books/5.clio"));
        assert_eq!(prefs.recent.len(), MAX_RECENT);
        assert!(prefs.recent[0].ends_with("5.clio"));
        assert_eq!(
            prefs
                .recent
                .iter()
                .filter(|p| p.ends_with("5.clio"))
                .count(),
            1
        );
        assert!(prefs.recent[1].ends_with("11.clio"));
        let first = prefs.recent[0].clone();
        prefs.remove_recent(&first);
        assert!(prefs.recent[0].ends_with("11.clio"));
    }

    #[test]
    fn in_memory_nothing_is_written() {
        let mut user = User::in_memory();
        user.change_trust(|t| t.trust("b", fingerprint("h")))
            .unwrap();
        user.change_prefs(|p| p.add_recent(Path::new("x.clio")))
            .unwrap();
        assert!(user.trust.is_recorded("b"));
        assert_eq!(user.prefs.recent.len(), 1);
    }
}
