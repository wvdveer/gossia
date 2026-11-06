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

use gossia_i18n::tr;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gossia_calc::python::PythonSettings;
use gossia_calc::sources::{SOURCES_FILE, SourceStore};
use gossia_calc::{Ceilings, Effective, Fingerprint, Setting, TrustStore};
use serde_json::{Map, Value as Json, json};

use crate::file::{Limits, save_atomic};

/// The version of both files that this build writes.
pub const CONFIG_VERSION: u64 = 1;
pub const PREFERENCES_FILE: &str = "preferences.json";
pub const TRUST_FILE: &str = "trust.json";
/// The history of the code editor's Immediate window.
pub const IMMEDIATE_FILE: &str = "immediate-history.json";
/// How many recent files are listed.
pub const MAX_RECENT: usize = 10;
/// Names the directory instead of the operating system's usual place, for
/// tests and for running two configurations side by side.
pub const CONFIG_DIR_VARIABLE: &str = "GOSSIA_CONFIG_DIR";

/// Where the files are kept instead of a folder: the storage of the host of
/// the wasm version (DESIGN.md, Web port, 0.3.0), each file a JSON text
/// under its name.
pub trait Storage {
    fn read(&self, name: &str) -> Option<String>;
    fn write(&self, name: &str, text: &str) -> Result<(), String>;
}

/// Where the files are, if anywhere.
#[derive(Clone, Default)]
pub struct Config {
    /// `None` keeps everything in memory, as the in-process tests do, unless
    /// there is a storage.
    pub dir: Option<PathBuf>,
    /// The host's storage, in place of the folder.
    pub storage: Option<Rc<dyn Storage>>,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("dir", &self.dir)
            .field("storage", &self.storage.is_some())
            .finish()
    }
}

impl PartialEq for Config {
    fn eq(&self, other: &Config) -> bool {
        let same_storage = match (&self.storage, &other.storage) {
            (Some(a), Some(b)) => Rc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        self.dir == other.dir && same_storage
    }
}

impl Eq for Config {}

impl Config {
    pub fn in_memory() -> Config {
        Config::default()
    }

    pub fn at(dir: impl Into<PathBuf>) -> Config {
        Config {
            dir: Some(dir.into()),
            storage: None,
        }
    }

    /// The files in the host's storage.
    pub fn in_storage(storage: Rc<dyn Storage>) -> Config {
        Config {
            dir: None,
            storage: Some(storage),
        }
    }

    /// Whether what is written is kept after the application ends.
    pub fn keeps(&self) -> bool {
        self.dir.is_some() || self.storage.is_some()
    }

    /// `GOSSIA_CONFIG_DIR` if it is set, otherwise the operating system's
    /// usual place for application settings: `%APPDATA%\Gossia` on Windows,
    /// `~/Library/Application Support/Gossia` on macOS, and
    /// `$XDG_CONFIG_HOME/gossia` (or `~/.config/gossia`) elsewhere. In
    /// memory only if none of these can be found.
    pub fn standard() -> Config {
        let var = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty());
        if let Some(dir) = var(CONFIG_DIR_VARIABLE) {
            return Config::at(dir);
        }
        let dir = if cfg!(windows) {
            var("APPDATA").map(|d| PathBuf::from(d).join("Gossia"))
        } else if cfg!(target_os = "macos") {
            var("HOME").map(|h| PathBuf::from(h).join("Library/Application Support/Gossia"))
        } else {
            var("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .or_else(|| var("HOME").map(|h| PathBuf::from(h).join(".config")))
                .map(|d| d.join("gossia"))
        };
        Config { dir, storage: None }
    }

    fn path(&self, name: &str) -> Option<PathBuf> {
        self.dir.as_ref().map(|d| d.join(name))
    }

    /// The object in a file, or `None` if there is no file. A file that
    /// cannot be read is moved aside, with a warning, so that writing the
    /// file again later does not lose it.
    fn read(&self, name: &str, warnings: &mut Vec<String>) -> Option<Map<String, Json>> {
        if let Some(storage) = &self.storage {
            let text = storage.read(name)?;
            return match serde_json::from_str::<Json>(&text) {
                Ok(Json::Object(object)) => Some(object),
                _ => {
                    warnings.push(format!(
                        "The stored {name} cannot be read, so the defaults are used."
                    ));
                    None
                }
            };
        }
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
        let mut text =
            serde_json::to_string_pretty(&Json::Object(object)).expect("JSON always serializes");
        text.push('\n');
        if let Some(storage) = &self.storage {
            return storage.write(name, &text);
        }
        let Some(path) = self.path(name) else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
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

/// A whole number from `range.0` to `range.1` at `key`, or the default with a
/// warning.
fn number_in(
    object: Option<&Map<String, Json>>,
    section: &str,
    key: &str,
    range: (u64, u64),
    default: u64,
    warnings: &mut Vec<String>,
) -> u64 {
    let n = number(object, section, key, range.0, default, warnings);
    if n > range.1 {
        warnings.push(format!(
            "In {PREFERENCES_FILE}, {section}.{key} is {n}, more than {}; the default \
             {default} is used.",
            range.1
        ));
        return default;
    }
    n
}

/// A yes or no at `key`, or the default with a warning.
fn flag(
    object: Option<&Map<String, Json>>,
    section: &str,
    key: &str,
    default: bool,
    warnings: &mut Vec<String>,
) -> bool {
    match object.and_then(|o| o.get(key)) {
        None => default,
        Some(Json::Bool(b)) => *b,
        Some(value) => {
            warnings.push(format!(
                "In {PREFERENCES_FILE}, {section}.{key} is {value}, which cannot be used; \
                 the default {default} is used."
            ));
            default
        }
    }
}

fn object_at<'a>(object: &'a Map<String, Json>, key: &str) -> Option<&'a Map<String, Json>> {
    object.get(key).and_then(Json::as_object)
}

/// Which theme the window uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeChoice {
    /// The operating system's.
    #[default]
    System,
    Light,
    Dark,
}

impl ThemeChoice {
    pub const ALL: [ThemeChoice; 3] = [ThemeChoice::System, ThemeChoice::Light, ThemeChoice::Dark];

    pub fn key(self) -> &'static str {
        match self {
            ThemeChoice::System => "system",
            ThemeChoice::Light => "light",
            ThemeChoice::Dark => "dark",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ThemeChoice::System => tr!("Follow system"),
            ThemeChoice::Light => tr!("Light"),
            ThemeChoice::Dark => tr!("Dark"),
        }
    }

    fn from_key(key: &str) -> Option<ThemeChoice> {
        ThemeChoice::ALL.into_iter().find(|t| t.key() == key)
    }
}

/// The look of the window (Options, General).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Appearance {
    pub theme: ThemeChoice,
    /// The interface scale, in percent.
    pub scale_percent: u64,
    /// The language of the interface; none follows the system's.
    pub language: Option<gossia_i18n::Language>,
}

/// The range of the interface scale, in percent.
pub const SCALE_RANGE: (u64, u64) = (50, 300);

impl Default for Appearance {
    fn default() -> Appearance {
        Appearance {
            theme: ThemeChoice::System,
            scale_percent: 100,
            language: None,
        }
    }
}

/// How tall the rows of the tree grid and the tabular view are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Density {
    Compact,
    #[default]
    Normal,
    Comfortable,
}

impl Density {
    pub const ALL: [Density; 3] = [Density::Compact, Density::Normal, Density::Comfortable];

    pub fn key(self) -> &'static str {
        match self {
            Density::Compact => "compact",
            Density::Normal => "normal",
            Density::Comfortable => "comfortable",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Density::Compact => tr!("Compact"),
            Density::Normal => tr!("Normal"),
            Density::Comfortable => tr!("Comfortable"),
        }
    }

    pub fn from_key(key: &str) -> Option<Density> {
        Density::ALL.into_iter().find(|d| d.key() == key)
    }

    /// The space a row has above and below its text, together, and the
    /// least height a row has, in points.
    pub fn padding(self) -> (f32, f32) {
        match self {
            Density::Compact => (5.0, 22.0),
            Density::Normal => (11.0, 28.0),
            Density::Comfortable => (17.0, 34.0),
        }
    }
}

/// The indentation per level of the tree grid, in pixels.
pub const TREE_INDENT_RANGE: (u64, u64) = (0, 64);
/// The first width of a column of the tabular view, in pixels.
pub const COLUMN_WIDTH_RANGE: (u64, u64) = (40, 1_000);
/// The most lines the formula bar grows to.
pub const FORMULA_BAR_LINES_RANGE: (u64, u64) = (1, 40);
/// How long the pointer rests on a cell before its comment shows, in
/// milliseconds.
pub const COMMENT_DELAY_RANGE: (u64, u64) = (0, 5_000);
/// The largest a comment popup is, in pixels.
pub const COMMENT_WIDTH_RANGE: (u64, u64) = (120, 1_200);
pub const COMMENT_HEIGHT_RANGE: (u64, u64) = (60, 1_200);

/// The data views (Options, Display).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayPrefs {
    /// The indentation per level of the tree grid, in pixels.
    pub indent: u64,
    pub density: Density,
    pub type_column: bool,
    pub formula_column: bool,
    /// The width a column of the tabular view starts at, in pixels.
    pub column_width: u64,
    /// How long the pointer rests on a cell before its comment shows, in
    /// milliseconds.
    pub comment_delay: u64,
    /// The largest a comment popup is, in pixels: its text wraps at the
    /// width and scrolls past the height.
    pub comment_width: u64,
    pub comment_height: u64,
}

impl Default for DisplayPrefs {
    fn default() -> DisplayPrefs {
        DisplayPrefs {
            indent: 16,
            density: Density::Normal,
            type_column: true,
            formula_column: true,
            column_width: 120,
            comment_delay: 500,
            comment_width: 360,
            comment_height: 240,
        }
    }
}

impl DisplayPrefs {
    fn from_object(
        section: Option<&Map<String, Json>>,
        warnings: &mut Vec<String>,
    ) -> DisplayPrefs {
        let d = DisplayPrefs::default();
        let s = "display";
        let density = match section.and_then(|o| o.get("row_density")) {
            None => d.density,
            Some(value) => match value.as_str().and_then(Density::from_key) {
                Some(density) => density,
                None => {
                    warnings.push(format!(
                        "In {PREFERENCES_FILE}, display.row_density is {value}, which cannot be \
                         used; the default normal is used."
                    ));
                    d.density
                }
            },
        };
        DisplayPrefs {
            indent: number_in(
                section,
                s,
                "tree_indent_px",
                TREE_INDENT_RANGE,
                d.indent,
                warnings,
            ),
            density,
            type_column: flag(section, s, "show_type_column", d.type_column, warnings),
            formula_column: flag(
                section,
                s,
                "show_formula_column",
                d.formula_column,
                warnings,
            ),
            column_width: number_in(
                section,
                s,
                "table_column_width_px",
                COLUMN_WIDTH_RANGE,
                d.column_width,
                warnings,
            ),
            comment_delay: number_in(
                section,
                s,
                "comment_hover_delay_ms",
                COMMENT_DELAY_RANGE,
                d.comment_delay,
                warnings,
            ),
            comment_width: number_in(
                section,
                s,
                "comment_popup_width_px",
                COMMENT_WIDTH_RANGE,
                d.comment_width,
                warnings,
            ),
            comment_height: number_in(
                section,
                s,
                "comment_popup_height_px",
                COMMENT_HEIGHT_RANGE,
                d.comment_height,
                warnings,
            ),
        }
    }

    fn values(&self) -> Vec<(&'static str, Json)> {
        vec![
            ("tree_indent_px", json!(self.indent)),
            ("row_density", json!(self.density.key())),
            ("show_type_column", json!(self.type_column)),
            ("show_formula_column", json!(self.formula_column)),
            ("table_column_width_px", json!(self.column_width)),
            ("comment_hover_delay_ms", json!(self.comment_delay)),
            ("comment_popup_width_px", json!(self.comment_width)),
            ("comment_popup_height_px", json!(self.comment_height)),
        ]
    }
}

/// The default for the largest file that is imported.
pub const DEFAULT_MAX_IMPORT_BYTES: u64 = 512 << 20;

/// The sizes of the code editor's font, in points.
pub const CODE_FONT_RANGE: (u64, u64) = (8, 32);
/// The indent widths, in spaces.
pub const INDENT_RANGE: (u64, u64) = (2, 8);
/// How many entries of the Immediate window's history may be kept.
pub const IMMEDIATE_ENTRIES_RANGE: (u64, u64) = (0, 10_000);
/// The largest result of the Immediate window that may be asked for, in
/// bytes: from 1 KB to 16 MB.
pub const IMMEDIATE_OUTPUT_RANGE: (u64, u64) = (1 << 10, 16 << 20);

/// The code editor and its Immediate window (Options, Code editor).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodePrefs {
    /// The size of the editor's font, in points. Its family is Geist Mono.
    pub font_size: u64,
    /// Spaces per level of indentation.
    pub indent: u64,
    pub line_numbers: bool,
    pub highlighting: bool,
    /// Wrap long lines at the width of the pane, instead of scrolling.
    pub wrap: bool,
    /// The most expressions the Immediate window's history keeps.
    pub history_entries: u64,
    /// The longest result of one expression shown, in bytes; the rest is
    /// cut off with a notice.
    pub max_output: u64,
}

impl Default for CodePrefs {
    fn default() -> CodePrefs {
        CodePrefs {
            font_size: 13,
            indent: 4,
            line_numbers: true,
            highlighting: true,
            wrap: false,
            history_entries: 100,
            max_output: 64 << 10,
        }
    }
}

impl CodePrefs {
    fn from_object(section: Option<&Map<String, Json>>, warnings: &mut Vec<String>) -> CodePrefs {
        let d = CodePrefs::default();
        let s = "code_editor";
        CodePrefs {
            font_size: number_in(
                section,
                s,
                "font_size",
                CODE_FONT_RANGE,
                d.font_size,
                warnings,
            ),
            indent: number_in(section, s, "indent_width", INDENT_RANGE, d.indent, warnings),
            line_numbers: flag(section, s, "line_numbers", d.line_numbers, warnings),
            highlighting: flag(section, s, "syntax_highlighting", d.highlighting, warnings),
            wrap: flag(section, s, "wrap_long_lines", d.wrap, warnings),
            history_entries: number_in(
                section,
                s,
                "immediate_history_entries",
                IMMEDIATE_ENTRIES_RANGE,
                d.history_entries,
                warnings,
            ),
            max_output: number_in(
                section,
                s,
                "immediate_max_output_bytes",
                IMMEDIATE_OUTPUT_RANGE,
                d.max_output,
                warnings,
            ),
        }
    }

    fn values(&self) -> Vec<(&'static str, Json)> {
        vec![
            ("font_size", json!(self.font_size)),
            ("indent_width", json!(self.indent)),
            ("line_numbers", json!(self.line_numbers)),
            ("syntax_highlighting", json!(self.highlighting)),
            ("wrap_long_lines", json!(self.wrap)),
            ("immediate_history_entries", json!(self.history_entries)),
            ("immediate_max_output_bytes", json!(self.max_output)),
        ]
    }
}

/// The application preferences.
#[derive(Debug, Clone, PartialEq)]
pub struct Preferences {
    /// The theme and the scale of the window.
    pub appearance: Appearance,
    /// The most each workbook setting may be.
    pub ceilings: Ceilings,
    /// What a workbook file may hold when it is read.
    pub limits: Limits,
    /// The largest file that is imported.
    pub max_import_bytes: u64,
    /// How *Copy* in the Raw view writes COBOL records as text.
    pub binary_copy: gossia_core::binary::BinaryText,
    /// The bounds of the undo history: entries, and a rough weight.
    pub history_entries: usize,
    pub history_weight: usize,
    /// The code editor and its Immediate window.
    pub code: CodePrefs,
    /// The tree grid and the tabular view.
    pub display: DisplayPrefs,
    /// The most lines the formula bar grows to before it scrolls.
    pub formula_bar_lines: u64,
    /// The workbooks opened or saved last, the most recent first.
    pub recent: Vec<PathBuf>,
    /// The file as it was read, so that what this version does not know is
    /// written back.
    raw: Map<String, Json>,
}

impl Default for Preferences {
    fn default() -> Preferences {
        Preferences {
            appearance: Appearance::default(),
            ceilings: Ceilings::default(),
            limits: Limits::default(),
            max_import_bytes: DEFAULT_MAX_IMPORT_BYTES,
            binary_copy: Default::default(),
            history_entries: gossia_calc::DEFAULT_MAX_ENTRIES,
            history_weight: gossia_calc::DEFAULT_MAX_WEIGHT,
            code: CodePrefs::default(),
            display: DisplayPrefs::default(),
            formula_bar_lines: 6,
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
        let max_import_bytes = number(
            files,
            "files",
            "max_import_bytes",
            1,
            defaults.max_import_bytes,
            warnings,
        );
        let binary_copy = match files.and_then(|o| o.get("copy_records_as")) {
            None => defaults.binary_copy,
            Some(value) => match value
                .as_str()
                .and_then(gossia_core::binary::BinaryText::from_key)
            {
                Some(kind) => kind,
                None => {
                    warnings.push(format!(
                        "In {PREFERENCES_FILE}, files.copy_records_as is {value}, which cannot \
                         be used; records are copied as a hex dump."
                    ));
                    defaults.binary_copy
                }
            },
        };
        let section = object_at(&raw, "appearance");
        let theme = match section.and_then(|o| o.get("theme")) {
            None => ThemeChoice::default(),
            Some(value) => match value.as_str().and_then(ThemeChoice::from_key) {
                Some(theme) => theme,
                None => {
                    warnings.push(format!(
                        "In {PREFERENCES_FILE}, appearance.theme is {value}, which cannot be \
                         used; the theme follows the system."
                    ));
                    ThemeChoice::default()
                }
            },
        };
        let mut scale_percent = number(
            section,
            "appearance",
            "scale_percent",
            SCALE_RANGE.0,
            100,
            warnings,
        );
        if scale_percent > SCALE_RANGE.1 {
            warnings.push(format!(
                "In {PREFERENCES_FILE}, appearance.scale_percent is {scale_percent}, more than \
                 {}; the default 100 is used.",
                SCALE_RANGE.1
            ));
            scale_percent = 100;
        }
        let language = match section.and_then(|o| o.get("language")) {
            None => None,
            Some(Json::String(tag)) if tag == "system" => None,
            Some(value) => match value.as_str().and_then(gossia_i18n::Language::from_tag) {
                Some(language) => Some(language),
                None => {
                    warnings.push(format!(
                        "In {PREFERENCES_FILE}, appearance.language is {value}, which cannot be \
                         used; the language follows the system."
                    ));
                    None
                }
            },
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
        let code = CodePrefs::from_object(object_at(&raw, "code_editor"), warnings);
        let display = DisplayPrefs::from_object(object_at(&raw, "display"), warnings);
        let formula_bar_lines = number_in(
            object_at(&raw, "editing"),
            "editing",
            "formula_bar_lines",
            FORMULA_BAR_LINES_RANGE,
            defaults.formula_bar_lines,
            warnings,
        );
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
            appearance: Appearance {
                theme,
                scale_percent,
                language,
            },
            ceilings,
            limits,
            max_import_bytes,
            binary_copy,
            history_entries,
            history_weight,
            code,
            display,
            formula_bar_lines,
            recent,
            raw,
        }
    }

    /// The file's object: what was read, with what this version knows set
    /// over it.
    fn to_object(&self) -> Map<String, Json> {
        let mut out = self.raw.clone();
        out.insert("version".to_owned(), json!(version_of(&self.raw)));
        let mut section = |key: &str, values: Vec<(&str, Json)>| {
            let mut object = object_at(&self.raw, key).cloned().unwrap_or_default();
            for (name, value) in values {
                object.insert(name.to_owned(), value);
            }
            out.insert(key.to_owned(), Json::Object(object));
        };
        section(
            "appearance",
            vec![
                ("theme", json!(self.appearance.theme.key())),
                ("scale_percent", json!(self.appearance.scale_percent)),
                (
                    "language",
                    json!(self.appearance.language.map_or("system", |l| l.tag())),
                ),
            ],
        );
        section(
            "ceilings",
            Setting::ALL
                .iter()
                .map(|s| (s.key(), json!(self.ceilings.get(*s))))
                .collect(),
        );
        section(
            "files",
            vec![
                ("max_zip_entries", json!(self.limits.entries)),
                ("max_total_bytes", json!(self.limits.total)),
                ("max_entry_bytes", json!(self.limits.entry)),
                ("max_import_bytes", json!(self.max_import_bytes)),
                ("copy_records_as", json!(self.binary_copy.key())),
            ],
        );
        section(
            "undo",
            vec![
                ("max_entries", json!(self.history_entries)),
                ("max_weight", json!(self.history_weight)),
            ],
        );
        section("code_editor", self.code.values());
        section("display", self.display.values());
        section(
            "editing",
            vec![("formula_bar_lines", json!(self.formula_bar_lines))],
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

/// The file each trusted workbook was last seen in, from `trust.json`.
fn trust_files(object: &Map<String, Json>) -> BTreeMap<String, PathBuf> {
    object_at(object, "workbooks")
        .into_iter()
        .flatten()
        .filter_map(|(id, record)| {
            let file = record.get("file").and_then(Json::as_str)?;
            Some((id.clone(), PathBuf::from(file)))
        })
        .collect()
}

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

fn trust_to_object(
    store: &TrustStore,
    files: &BTreeMap<String, PathBuf>,
    raw: &Map<String, Json>,
) -> Map<String, Json> {
    let mut out = raw.clone();
    out.insert("version".to_owned(), json!(version_of(raw)));
    let workbooks: Map<String, Json> = store
        .records()
        .map(|(id, f)| {
            let mut record = json!({ "hash": f.hash, "items": f.items });
            if let Some(file) = files.get(id) {
                record["file"] = json!(file.display().to_string());
            }
            (id.to_owned(), record)
        })
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
    /// The file each trusted workbook was last seen in, for the list on the
    /// Trust and security page. A workbook trusted before it had a file has
    /// none.
    pub trust_files: BTreeMap<String, PathBuf>,
    /// `trust.json` as it was read, for what this version does not know.
    trust_raw: Map<String, Json>,
    /// What was typed in the Immediate window, the oldest first.
    pub immediate: Vec<String>,
    /// The data source definitions of this user on this machine, in
    /// `data-sources.json` beside `trust.json` (DESIGN.md, "Data sources").
    pub sources: SourceStore,
}

impl User {
    /// Nothing on disk: the in-process tests, and the sample.
    pub fn in_memory() -> User {
        User::default()
    }

    /// What the interpreter is made with: the settings in force, and this
    /// user's data sources.
    pub fn python_settings(&self, effective: &Effective) -> PythonSettings {
        PythonSettings {
            sources: self.sources.clone(),
            ..effective.python_settings()
        }
    }

    /// Reads the files of `config`. A missing file is the defaults; the
    /// warnings are for the user.
    pub fn load(config: Config) -> (User, Vec<String>) {
        let mut warnings = Vec::new();
        if !config.keeps() {
            warnings.push(
                "There is no folder for the preferences, so trust decisions and recent files \
                 are kept only until the application ends."
                    .to_owned(),
            );
        }
        let sources = match &config.dir {
            Some(dir) => SourceStore::at(dir.join(SOURCES_FILE)),
            None => SourceStore::in_memory(),
        };
        let mut user = User {
            config,
            sources,
            ..User::default()
        };
        user.reload(&mut warnings);
        (user, warnings)
    }

    fn reload(&mut self, warnings: &mut Vec<String>) {
        if !self.config.keeps() {
            return;
        }
        self.prefs = match self.config.read(PREFERENCES_FILE, warnings) {
            Some(raw) => Preferences::from_object(raw, warnings),
            None => Preferences::default(),
        };
        match self.config.read(TRUST_FILE, warnings) {
            Some(raw) => {
                self.trust = trust_from_object(&raw, warnings);
                self.trust_files = trust_files(&raw);
                self.trust_raw = raw;
            }
            None => {
                self.trust = TrustStore::default();
                self.trust_files = BTreeMap::new();
                self.trust_raw = Map::new();
            }
        }
        self.immediate = self.read_immediate(warnings);
    }

    /// The Immediate window's history as the file has it.
    fn read_immediate(&self, warnings: &mut Vec<String>) -> Vec<String> {
        self.config
            .read(IMMEDIATE_FILE, warnings)
            .and_then(|raw| {
                raw.get("entries").and_then(Json::as_array).map(|items| {
                    items
                        .iter()
                        .filter_map(Json::as_str)
                        .map(str::to_owned)
                        .collect()
                })
            })
            .unwrap_or_default()
    }

    /// Adds an expression to the end of the Immediate window's history, which
    /// keeps at most the number of entries of the preferences: the file is
    /// read again first, so that what another window added is kept. The same
    /// text typed again moves to the end instead of being kept twice.
    pub fn add_immediate(&mut self, text: &str) -> Result<(), String> {
        self.change_immediate(|entries| {
            entries.retain(|e| e != text);
            entries.push(text.to_owned());
        })
    }

    /// Drops the oldest entries of the Immediate window's history past the
    /// number the preferences keep, if there are any.
    pub fn keep_immediate(&mut self) -> Result<(), String> {
        if self.immediate.len() as u64 <= self.prefs.code.history_entries {
            return Ok(());
        }
        self.change_immediate(|_| {})
    }

    /// *Clear history* on the Code editor page.
    pub fn clear_immediate(&mut self) -> Result<(), String> {
        self.change_immediate(Vec::clear)
    }

    fn change_immediate(&mut self, change: impl FnOnce(&mut Vec<String>)) -> Result<(), String> {
        if self.config.keeps() {
            let mut ignored = Vec::new();
            self.immediate = self.read_immediate(&mut ignored);
        }
        change(&mut self.immediate);
        let keep = usize::try_from(self.prefs.code.history_entries).unwrap_or(usize::MAX);
        let extra = self.immediate.len().saturating_sub(keep);
        self.immediate.drain(..extra);
        let mut object = Map::new();
        object.insert("version".to_owned(), json!(CONFIG_VERSION));
        object.insert("entries".to_owned(), json!(self.immediate));
        self.config.write(IMMEDIATE_FILE, object)
    }

    /// Changes the trust record: the file is read again, so that what another
    /// window recorded is kept, the change is made, and the file is written.
    /// The change is made in memory even if the file cannot be written.
    pub fn change_trust(&mut self, change: impl FnOnce(&mut TrustStore)) -> Result<(), String> {
        self.change_trust_seen(change, None)
    }

    /// Changes the trust record as [`User::change_trust`] does, and notes the
    /// file that workbook `seen.0` is in, if it is trusted.
    pub fn change_trust_seen(
        &mut self,
        change: impl FnOnce(&mut TrustStore),
        seen: Option<(&str, &Path)>,
    ) -> Result<(), String> {
        let mut ignored = Vec::new();
        if self.config.keeps() {
            match self.config.read(TRUST_FILE, &mut ignored) {
                Some(raw) => {
                    self.trust = trust_from_object(&raw, &mut ignored);
                    self.trust_files = trust_files(&raw);
                    self.trust_raw = raw;
                }
                None => {
                    self.trust = TrustStore::default();
                    self.trust_files = BTreeMap::new();
                    self.trust_raw = Map::new();
                }
            }
        }
        change(&mut self.trust);
        if let Some((id, path)) = seen {
            let path = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
            self.trust_files.insert(id.to_owned(), path);
        }
        let trust = &self.trust;
        self.trust_files.retain(|id, _| trust.is_recorded(id));
        self.config.write(
            TRUST_FILE,
            trust_to_object(&self.trust, &self.trust_files, &self.trust_raw),
        )
    }

    /// Changes the preferences, in the same way as [`User::change_trust`].
    pub fn change_prefs(&mut self, change: impl FnOnce(&mut Preferences)) -> Result<(), String> {
        let mut ignored = Vec::new();
        if self.config.keeps() {
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
    use crate::file::new_workbook_id;

    struct Folder(PathBuf);

    impl Folder {
        fn new(tag: &str) -> Folder {
            let path = std::env::temp_dir().join(format!(
                "gossia-prefs-{tag}-{}-{}",
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
        assert_eq!(user.prefs.ceilings.get(Setting::MaxRange), 1_000_000);
        assert_eq!(user.prefs.history_entries, 50);

        user.change_prefs(|p| p.add_recent(Path::new("/books/a.gossia")))
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
        assert_eq!(user.prefs.ceilings.get(Setting::MaxRange), 1_000_000);
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
            prefs.add_recent(Path::new(&format!("/books/{i}.gossia")));
        }
        prefs.add_recent(Path::new("/books/5.gossia"));
        assert_eq!(prefs.recent.len(), MAX_RECENT);
        assert!(prefs.recent[0].ends_with("5.gossia"));
        assert_eq!(
            prefs
                .recent
                .iter()
                .filter(|p| p.ends_with("5.gossia"))
                .count(),
            1
        );
        assert!(prefs.recent[1].ends_with("11.gossia"));
        let first = prefs.recent[0].clone();
        prefs.remove_recent(&first);
        assert!(prefs.recent[0].ends_with("11.gossia"));
    }

    #[test]
    fn the_language_is_kept_and_a_tag_not_known_follows_the_system() {
        let folder = Folder::new("language");
        let (mut user, _) = User::load(Config::at(&folder.0));
        assert_eq!(
            user.prefs.appearance.language, None,
            "the system's at first"
        );
        user.change_prefs(|p| p.appearance.language = Some(gossia_i18n::Language::Japanese))
            .unwrap();
        let text = std::fs::read_to_string(folder.0.join(PREFERENCES_FILE)).unwrap();
        assert!(text.contains("\"language\": \"ja\""), "{text}");
        let (again, warnings) = User::load(Config::at(&folder.0));
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(
            again.prefs.appearance.language,
            Some(gossia_i18n::Language::Japanese)
        );
        std::fs::write(
            folder.0.join(PREFERENCES_FILE),
            r#"{"version": 3, "appearance": {"language": "klingon"}}"#,
        )
        .unwrap();
        let (odd, warnings) = User::load(Config::at(&folder.0));
        assert_eq!(odd.prefs.appearance.language, None);
        assert!(warnings[0].contains("appearance.language"), "{warnings:?}");
    }

    #[test]
    fn in_memory_nothing_is_written() {
        let mut user = User::in_memory();
        user.change_trust(|t| t.trust("b", fingerprint("h")))
            .unwrap();
        user.change_prefs(|p| p.add_recent(Path::new("x.gossia")))
            .unwrap();
        assert!(user.trust.is_recorded("b"));
        assert_eq!(user.prefs.recent.len(), 1);
    }

    /// The host's storage, as a map of texts.
    #[derive(Default)]
    struct Memory(std::cell::RefCell<BTreeMap<String, String>>);

    impl Storage for Memory {
        fn read(&self, name: &str) -> Option<String> {
            self.0.borrow().get(name).cloned()
        }

        fn write(&self, name: &str, text: &str) -> Result<(), String> {
            self.0.borrow_mut().insert(name.to_owned(), text.to_owned());
            Ok(())
        }
    }

    #[test]
    fn the_files_can_be_kept_in_a_hosts_storage() {
        let storage = Rc::new(Memory::default());
        storage
            .write(
                PREFERENCES_FILE,
                r#"{"version": 3, "appearance": {"theme": "dark"}}"#,
            )
            .unwrap();
        let (mut user, warnings) = User::load(Config::in_storage(storage.clone()));
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(user.prefs.appearance.theme, ThemeChoice::Dark);
        user.change_trust(|t| t.trust("w1", fingerprint("h1")))
            .unwrap();
        user.add_immediate("1 + 1").unwrap();
        let trust: Json = serde_json::from_str(&storage.read(TRUST_FILE).unwrap()).unwrap();
        assert!(trust.to_string().contains("w1"), "{trust}");
        assert!(storage.read(IMMEDIATE_FILE).unwrap().contains("1 + 1"));

        // Read again, as the next session does.
        let (again, _) = User::load(Config::in_storage(storage.clone()));
        assert!(again.trust.is_recorded("w1"));
        assert_eq!(again.immediate, ["1 + 1"]);
        // Text that is not a JSON object gives the defaults, with a warning.
        storage.write(PREFERENCES_FILE, "not json").unwrap();
        let (fresh, warnings) = User::load(Config::in_storage(storage));
        assert_eq!(fresh.prefs.appearance.theme, ThemeChoice::default());
        assert_eq!(warnings.len(), 1, "{warnings:?}");
    }
}
