//! The workbook as the files of a `.clio` archive, in memory (DESIGN.md, "File
//! format"). This module makes and reads the *entries*: the manifest, the data
//! of each tab, its overlay, the form designs and the modules. Turning the
//! entries into a zip and back, and the files on disk, belong to the
//! application, so that this crate stays free of filesystem code.
//!
//! What is written is a pure function of the workbook: the entries are in a
//! fixed order, JSON has a stable key order and LF line endings, and nothing
//! per-save (a time, a counter) is recorded, so saving a workbook that has not
//! changed gives the same bytes.

use std::collections::{BTreeMap, HashSet};

use cliotree_core::dataformat::DataFormat;
use cliotree_core::form::FormDesign;
use cliotree_core::{CommentPlace, ErrorKind, ErrorValue, Path, json, xml};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use super::*;

/// The format version this build writes. A file of the same major version
/// opens; a newer minor version opens with a warning; a newer major version is
/// refused.
pub const FORMAT_MAJOR: u32 = 1;
pub const FORMAT_MINOR: u32 = 1;

/// The version of the rules of Names and reserved words that wrote the file,
/// so that a later version can tell whether the names in it were checked
/// against the same list.
pub const RESERVED_NAMES_VERSION: u32 = 1;

/// The Python engine, as the manifest records it. The revision is the one
/// pinned in `vendor.lock`; a file written against another revision is flagged
/// when it is opened.
pub const ENGINE_NAME: &str = "rustpython";
pub const ENGINE_REVISION: &str = "e5554e23abfbaee85508c32fc22e5074f88aebdd";

const MANIFEST: &str = "manifest.json";
/// The workbook settings. Left out when every setting has its default.
const SETTINGS: &str = "settings.json";
/// The state of the view of the whole workbook (the font of the data views),
/// which the application owns and this layer only carries. Left out when
/// empty.
const VIEW: &str = "view.json";

// ------------------------------------------------------------------ entries

/// The entries of an archive, in order. Paths use `/`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Package {
    entries: IndexMap<String, Vec<u8>>,
}

impl Package {
    pub fn new() -> Package {
        Package::default()
    }

    /// Adds an entry (replacing one of that path), after checking that the
    /// path could not reach outside the archive when it is extracted: it is
    /// relative, uses `/`, and has no empty, `.` or `..` part and no control
    /// character.
    pub fn insert(&mut self, path: &str, bytes: Vec<u8>) -> Result<(), String> {
        safe_path(path)?;
        self.entries.insert(path.to_owned(), bytes);
        Ok(())
    }

    pub fn get(&self, path: &str) -> Option<&[u8]> {
        self.entries.get(path).map(Vec::as_slice)
    }

    /// The entries, in the order they are written.
    pub fn entries(&self) -> impl Iterator<Item = (&str, &[u8])> {
        self.entries.iter().map(|(p, b)| (p.as_str(), b.as_slice()))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn text(&self, path: &str) -> Result<Option<&str>, PackageError> {
        match self.get(path) {
            None => Ok(None),
            Some(bytes) => {
                std::str::from_utf8(bytes)
                    .map(Some)
                    .map_err(|_| PackageError::BadEntry {
                        path: path.to_owned(),
                        message: "it is not UTF-8 text".to_owned(),
                    })
            }
        }
    }
}

/// Whether `path` is safe as the path of an archive entry.
pub fn safe_path(path: &str) -> Result<(), String> {
    if path.is_empty() {
        return Err("an entry has no path".to_owned());
    }
    if path.starts_with('/') || path.contains('\\') || path.contains(':') {
        return Err(format!("the path {path:?} is not relative"));
    }
    for part in path.split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return Err(format!("the path {path:?} has a part that is not a name"));
        }
        if part.chars().any(char::is_control) {
            return Err(format!("the path {path:?} has a control character"));
        }
    }
    Ok(())
}

/// Whether `name` can be used as one part of an entry path: a tab, form or
/// module name from a manifest.
fn safe_part(name: &str) -> bool {
    !name.is_empty() && !name.contains('/') && safe_path(name).is_ok()
}

// ----------------------------------------------------------------- manifest

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Manifest {
    format_version: String,
    app_version: String,
    workbook_id: String,
    reserved_names_version: u32,
    engine: EngineEntry,
    tabs: Vec<TabEntry>,
    modules: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EngineEntry {
    name: String,
    version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TabEntry {
    name: String,
    /// `data` in version 1; `report` is reserved for a later version.
    kind: String,
    /// `builtin:jsonschema`, `builtin:xsd`, `builtin:copybook`, or `sheet:<Tab>`.
    schema: String,
    /// Whether the tabs that use this schema tab follow its changes.
    #[serde(default, skip_serializing_if = "is_false")]
    auto_update: bool,
    /// The customised forms of the tab, in display order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    forms: Vec<String>,
    /// The file format of a data tab that is not JSON or XML: `json5`,
    /// `jsonc` or `jsonl` (DESIGN.md, "Other file formats"). Its data is
    /// `data.<format>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    format: Option<String>,
    /// For a tab of COBOL records, the layout it shows of each group of
    /// `REDEFINES`, where that is not the first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    redefines: Vec<usize>,
    /// For a tab of COBOL records, the 01 record of its copybook it holds,
    /// where that is not the first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    record: Option<String>,
    /// For a tab of COBOL records saved as a variable-length record, `VB`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    recfm: Option<String>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// How a tab is tied to its schema, as the manifest writes it.
enum Binding {
    JsonSchema,
    Xsd,
    Copybook,
    Sheet(String),
}

impl Binding {
    fn parse(text: &str) -> Option<Binding> {
        match text {
            "builtin:jsonschema" => Some(Binding::JsonSchema),
            "builtin:xsd" => Some(Binding::Xsd),
            "builtin:copybook" => Some(Binding::Copybook),
            _ => text
                .strip_prefix("sheet:")
                .filter(|name| !name.is_empty())
                .map(|name| Binding::Sheet(name.to_owned())),
        }
    }
}

fn version_text(major: u32, minor: u32) -> String {
    format!("{major}.{minor}")
}

fn parse_version(text: &str) -> Option<(u32, u32)> {
    let (major, minor) = text.split_once('.')?;
    Some((major.parse().ok()?, minor.parse().ok()?))
}

// ------------------------------------------------------------------ overlay

/// The `overlay.json` of a tab: the formulas, and the state of the view.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Overlay {
    /// By the path of the node, sorted.
    #[serde(default)]
    formulas: BTreeMap<String, OverlayFormula>,
    /// The column formulas, each once; the items that have one are not listed
    /// in `formulas` unless they ended in an error.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    columns: Vec<OverlayColumn>,
    /// The state of the view, which the application owns and this layer only
    /// carries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    view: Option<Json>,
    /// The comments of a tab whose file has no place for them (JSON and
    /// JSON Lines): kept in the workbook only (owner, 2026-11-01).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    comments: Vec<OverlayComment>,
}

/// A comment of a node, in a tab whose file cannot hold it.
#[derive(Debug, Serialize, Deserialize)]
struct OverlayComment {
    path: String,
    /// `before` (left out), `start` or `end`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    place: Option<String>,
    text: String,
}

/// A formula for a field of every item of an array.
#[derive(Debug, Serialize, Deserialize)]
struct OverlayColumn {
    /// The array, as a path.
    array: String,
    field: String,
    /// The expression, without the `=`.
    formula: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct OverlayFormula {
    /// The expression, without the `=`. Absent for an item of a column, which
    /// has the column's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    formula: Option<String>,
    status: OverlayStatus,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum OverlayStatus {
    Ok,
    /// Saved while out of date.
    Pending,
    Error(OverlayError),
}

#[derive(Debug, Serialize, Deserialize)]
struct OverlayError {
    /// The code, for example `#DIV/0!`.
    kind: String,
    message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    origin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    traceback: Option<String>,
}

// ------------------------------------------------------- what goes with it

/// What the application adds to the workbook when it is saved.
#[derive(Debug, Clone, Default)]
pub struct Meta {
    /// The random id the trust model knows the workbook by.
    pub workbook_id: String,
    /// The version of the application that writes the file.
    pub app_version: String,
    /// The state of the view of each tab, by tab name.
    pub views: BTreeMap<String, Json>,
    /// The state of the view of the whole workbook: a JSON object, or None.
    pub workbook_view: Option<Json>,
    /// Entries this version does not know, to be written back unchanged.
    pub unknown: Vec<(String, Vec<u8>)>,
}

/// A workbook that was read.
pub struct Opened {
    /// Nothing in it is out of date, and nothing is to undo. It is trusted
    /// until told otherwise: [`Calc::set_trusted`] when the trust check says
    /// no, [`Calc::recalc_all`] when it says yes.
    pub calc: Calc,
    pub workbook_id: String,
    pub views: BTreeMap<String, Json>,
    /// The state of the view of the whole workbook (`view.json`), if any.
    pub workbook_view: Option<Json>,
    /// Entries this version does not know, to be handed back on save.
    pub unknown: Vec<(String, Vec<u8>)>,
    /// Things worth telling the user: the file opened, but not quite as it
    /// was written.
    pub warnings: Vec<String>,
    /// The format version that wrote the file.
    pub format_version: (u32, u32),
}

/// Why a workbook could not be written or read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackageError {
    /// The archive has no `manifest.json`.
    NoManifest,
    /// The manifest cannot be read.
    BadManifest(String),
    /// The file was written by a newer major version of the format.
    TooNew { needed: String },
    /// An entry cannot be read.
    BadEntry { path: String, message: String },
    /// A tab cannot be made from its entries.
    Tab { tab: String, message: String },
    /// The workbook has something that cannot be saved.
    Unsavable(String),
}

impl std::fmt::Display for PackageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PackageError::NoManifest => {
                f.write_str("the file has no manifest.json, so it is not a ClioTree workbook")
            }
            PackageError::BadManifest(message) => {
                write!(f, "the manifest cannot be read: {message}")
            }
            PackageError::TooNew { needed } => write!(
                f,
                "the file was written by a newer version of ClioTree (it needs the format \
                 version {needed}, which this version cannot read)"
            ),
            PackageError::BadEntry { path, message } => write!(f, "{path}: {message}"),
            PackageError::Tab { tab, message } => write!(f, "tab {tab}: {message}"),
            PackageError::Unsavable(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for PackageError {}

fn json_text<T: Serialize>(value: &T) -> Vec<u8> {
    let mut text = serde_json::to_string_pretty(value).expect("these types always serialize");
    text.push('\n');
    text.into_bytes()
}

fn lf(mut text: String) -> Vec<u8> {
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text.into_bytes()
}

// ------------------------------------------------------------------ writing

impl Calc {
    /// The entries of the workbook, in the order they are written. Fails if a
    /// tab cannot be saved: one that was given a type directly and so has no
    /// schema to write, or a name that is not safe as a file name.
    pub fn to_package(&self, meta: &Meta) -> Result<Package, PackageError> {
        let unsavable = |message: String| PackageError::Unsavable(message);
        let mut files: Vec<(String, Vec<u8>)> = Vec::new();
        let mut tabs = Vec::new();
        for (name, sheet) in &self.sheets {
            if !safe_part(name) {
                return Err(unsavable(format!(
                    "the tab name {name:?} cannot be used as a file name"
                )));
            }
            let (schema, auto_update) = match &sheet.schema {
                SchemaRole::Free => {
                    return Err(unsavable(format!(
                        "tab {name} has no schema tab (its type was given directly), so it \
                         cannot be saved"
                    )));
                }
                SchemaRole::Document {
                    format,
                    auto_update,
                    ..
                } => (
                    match format {
                        SchemaFormat::Json => "builtin:jsonschema".to_owned(),
                        SchemaFormat::Xsd => "builtin:xsd".to_owned(),
                        SchemaFormat::Copybook => "builtin:copybook".to_owned(),
                    },
                    *auto_update,
                ),
                SchemaRole::Bound(schema_sheet) => (format!("sheet:{schema_sheet}"), false),
            };

            // COBOL records are saved as bytes; a tab with a value that
            // cannot be written as one is saved as JSON, so no edit is lost.
            let records = (sheet.format == DataFormat::Blk)
                .then(|| self.records_bytes(name).ok())
                .flatten();
            let saved_as = match (&records, sheet.format) {
                (None, DataFormat::Blk) => DataFormat::Json,
                (_, format) => format,
            };
            let data = match (records, saved_as) {
                (Some(bytes), _) => bytes,
                (None, DataFormat::Xml) => {
                    let document = sheet.xml.clone().unwrap_or_default();
                    lf(xml::to_string(&sheet.data, &sheet.spec, &document, true)
                        .map_err(|e| unsavable(format!("tab {name}: {e}")))?)
                }
                (None, format) => lf(format
                    .write(&sheet.data, true)
                    .map_err(|e| unsavable(format!("tab {name}: {e}")))?),
            };
            files.push((format!("sheets/{name}/data.{}", saved_as.extension()), data));
            let format = match saved_as {
                DataFormat::Json | DataFormat::Xml => None,
                other => Some(other.code().to_owned()),
            };

            let mut overlay = self.overlay_of(name, sheet, meta.views.get(name).cloned());
            if !saved_as.has_comments() {
                overlay.comments = overlay_comments(&sheet.data);
            }
            if !overlay.formulas.is_empty()
                || !overlay.columns.is_empty()
                || overlay.view.is_some()
                || !overlay.comments.is_empty()
            {
                files.push((format!("sheets/{name}/overlay.json"), json_text(&overlay)));
            }

            let form_names: Vec<String> = self
                .forms
                .names(name)
                .into_iter()
                .map(str::to_owned)
                .collect();
            for form in &form_names {
                if !safe_part(form) {
                    return Err(unsavable(format!(
                        "the form name {form:?} of tab {name} cannot be used as a file name"
                    )));
                }
                if let Some(design) = self.forms.get(name, form) {
                    files.push((
                        format!("sheets/{name}/forms/{form}.json"),
                        lf(design.to_json()),
                    ));
                }
            }
            tabs.push(TabEntry {
                name: name.clone(),
                kind: "data".to_owned(),
                schema,
                auto_update,
                forms: form_names,
                format,
                redefines: sheet.redefines.clone(),
                record: sheet.record.clone(),
                recfm: sheet.variable.then(|| "VB".to_owned()),
            });
        }

        // The reports: a directory each, with the design in it.
        for (name, design) in &self.reports {
            if !safe_part(name) {
                return Err(unsavable(format!(
                    "the report name {name:?} cannot be used as a file name"
                )));
            }
            files.push((format!("reports/{name}/design.json"), lf(design.to_json())));
            tabs.push(TabEntry {
                name: name.clone(),
                kind: "report".to_owned(),
                schema: String::new(),
                auto_update: false,
                forms: Vec::new(),
                format: None,
                redefines: Vec::new(),
                record: None,
                recfm: None,
            });
        }
        // The tabs in the order of the bar.
        let order = self.tab_names();
        tabs.sort_by_key(|t| order.iter().position(|n| *n == t.name));
        // A workbook with reports needs a version that knows them.
        let minor = if self.reports.is_empty() {
            0
        } else {
            FORMAT_MINOR
        };

        for (name, module) in &self.modules {
            if !safe_part(name) {
                return Err(unsavable(format!(
                    "the module name {name:?} cannot be used as a file name"
                )));
            }
            files.push((format!("modules/{name}.py"), lf(module.source.to_string())));
        }
        if let Some(text) = self.settings.to_json() {
            files.push((SETTINGS.to_owned(), text.into_bytes()));
        }
        if let Some(view) = meta
            .workbook_view
            .as_ref()
            .filter(|v| v.as_object().is_some_and(|o| !o.is_empty()))
        {
            files.push((VIEW.to_owned(), json_text(view)));
        }

        let manifest = Manifest {
            format_version: version_text(FORMAT_MAJOR, minor),
            app_version: meta.app_version.clone(),
            workbook_id: meta.workbook_id.clone(),
            reserved_names_version: RESERVED_NAMES_VERSION,
            engine: EngineEntry {
                name: ENGINE_NAME.to_owned(),
                version: ENGINE_REVISION.to_owned(),
            },
            tabs,
            modules: self.modules.keys().cloned().collect(),
        };

        let mut package = Package::new();
        package
            .insert(MANIFEST, json_text(&manifest))
            .map_err(unsavable)?;
        for (path, bytes) in files {
            package.insert(&path, bytes).map_err(unsavable)?;
        }
        // What this version does not know goes back as it came, after what it
        // does, in path order.
        let mut unknown: Vec<&(String, Vec<u8>)> = meta.unknown.iter().collect();
        unknown.sort_by(|a, b| a.0.cmp(&b.0));
        for (path, bytes) in unknown {
            if package.get(path).is_none() {
                package.insert(path, bytes.clone()).map_err(unsavable)?;
            }
        }
        Ok(package)
    }

    fn overlay_of(&self, tab: &str, sheet: &Sheet, view: Option<Json>) -> Overlay {
        let mut formulas = BTreeMap::new();
        for (path, formula) in &sheet.formulas.by_path {
            let in_column = sheet.columns.iter().any(|c| c.holds(path));
            let loc = Loc::new(tab, path.clone());
            let status = if let Some(error) = &formula.error {
                OverlayStatus::Error(OverlayError {
                    kind: error.kind.code().to_owned(),
                    message: error.message.clone(),
                    origin: error.origin.as_ref().map(ToString::to_string),
                    traceback: formula.traceback.clone(),
                })
            } else if self.dirty.contains(&loc) {
                OverlayStatus::Pending
            } else {
                OverlayStatus::Ok
            };
            // An item of a column that is as the column makes it needs no entry.
            if in_column && !matches!(status, OverlayStatus::Error(_)) {
                continue;
            }
            formulas.insert(
                path.to_string(),
                OverlayFormula {
                    formula: (!in_column).then(|| formula.text.clone()),
                    status,
                },
            );
        }
        let columns = sheet
            .columns
            .iter()
            .map(|c| OverlayColumn {
                array: c.array.to_string(),
                field: c.field.clone(),
                formula: c.text.clone(),
            })
            .collect();
        Overlay {
            formulas,
            columns,
            view,
            comments: Vec::new(),
        }
    }
}

// ------------------------------------------------------------------ reading

fn bad(path: &str, message: impl ToString) -> PackageError {
    PackageError::BadEntry {
        path: path.to_owned(),
        message: message.to_string(),
    }
}

impl Calc {
    /// Makes the workbook of the entries. Nothing is run: what a formula held
    /// when it was saved is what it shows, until the caller decides on trust
    /// (see [`Opened::calc`]).
    pub fn from_package(
        syntax: Arc<dyn Syntax>,
        package: &Package,
    ) -> Result<Opened, PackageError> {
        let manifest_text = package.text(MANIFEST)?.ok_or(PackageError::NoManifest)?;
        let manifest: Manifest = serde_json::from_str(manifest_text)
            .map_err(|e| PackageError::BadManifest(e.to_string()))?;
        let (major, minor) = parse_version(&manifest.format_version).ok_or_else(|| {
            PackageError::BadManifest(format!(
                "the format version {:?} is not major.minor",
                manifest.format_version
            ))
        })?;
        if major > FORMAT_MAJOR {
            return Err(PackageError::TooNew {
                needed: manifest.format_version.clone(),
            });
        }
        let mut warnings = Vec::new();
        if major == FORMAT_MAJOR && minor > FORMAT_MINOR {
            warnings.push(format!(
                "The file was written by a newer version of ClioTree (format {}); what this \
                 version does not know is kept as it is.",
                manifest.format_version
            ));
        }
        if manifest.engine.name != ENGINE_NAME || manifest.engine.version != ENGINE_REVISION {
            warnings.push(format!(
                "The file was written against {} {}, and this version runs {} {}; results \
                 may differ.",
                manifest.engine.name,
                short(&manifest.engine.version),
                ENGINE_NAME,
                short(ENGINE_REVISION),
            ));
        }

        let mut seen = HashSet::new();
        for tab in &manifest.tabs {
            if !safe_part(&tab.name) {
                return Err(PackageError::BadManifest(format!(
                    "the tab name {:?} cannot be used as a file name",
                    tab.name
                )));
            }
            if !seen.insert(tab.name.to_lowercase()) {
                return Err(PackageError::BadManifest(format!(
                    "two tabs are called {} (names ignore case)",
                    tab.name
                )));
            }
            if tab.kind != "data" && tab.kind != "report" {
                warnings.push(format!(
                    "Tab {} is a {} tab, which this version cannot show; it is kept as it is.",
                    tab.name, tab.kind
                ));
            }
            if let Some(problem) = crate::names::problem(&tab.name, crate::names::Subject::Tab) {
                warnings.push(format!("Tab {}: {problem}", tab.name));
            }
        }

        let mut used: HashSet<String> = HashSet::from([MANIFEST.to_owned()]);
        let mut calc = Calc::new(syntax);
        let mut views = BTreeMap::new();
        // Schema tabs first, so that the tabs bound to them can be typed.
        for schema_pass in [true, false] {
            for tab in manifest.tabs.iter().filter(|t| t.kind == "data") {
                let binding = Binding::parse(&tab.schema).ok_or_else(|| PackageError::Tab {
                    tab: tab.name.clone(),
                    message: format!("{:?} is not a schema binding", tab.schema),
                })?;
                if matches!(binding, Binding::Sheet(_)) == schema_pass {
                    continue;
                }
                calc.read_tab(package, tab, binding, &mut used, &mut views, &mut warnings)?;
            }
        }
        // Auto-update, now that the tabs that use each schema are there to be
        // counted.
        for tab in manifest.tabs.iter().filter(|t| t.auto_update) {
            if let Some(why) = calc.auto_update_blocker(&tab.name) {
                warnings.push(format!("Auto-update of {} is off: {why}.", tab.name));
            } else if let Some(SchemaRole::Document { auto_update, .. }) =
                calc.sheets.get_mut(&tab.name).map(|s| &mut s.schema)
            {
                *auto_update = true;
            }
        }
        // The tabs, in the order of the manifest.
        let order: Vec<String> = manifest
            .tabs
            .iter()
            .filter(|t| t.kind == "data")
            .map(|t| t.name.clone())
            .collect();
        let mut loaded = std::mem::take(&mut calc.sheets);
        for name in &order {
            if let Some(sheet) = loaded.shift_remove(name) {
                calc.sheets.insert(name.clone(), sheet);
            }
        }

        for tab in manifest.tabs.iter().filter(|t| t.kind == "data") {
            for form in &tab.forms {
                if !safe_part(form) {
                    return Err(PackageError::BadManifest(format!(
                        "the form name {form:?} of tab {} cannot be used as a file name",
                        tab.name
                    )));
                }
                let path = format!("sheets/{}/forms/{form}.json", tab.name);
                let Some(text) = package.text(&path)? else {
                    warnings.push(format!(
                        "The manifest lists the form {form} of tab {}, but the file has no {path}.",
                        tab.name
                    ));
                    continue;
                };
                used.insert(path.clone());
                match FormDesign::from_json(text) {
                    Ok(design) => calc.forms.set(&tab.name, form, design),
                    Err(e) => warnings.push(format!(
                        "The form {form} of tab {} is not used: {e}",
                        tab.name
                    )),
                }
            }
        }

        // The reports. One whose design cannot be read is left out, with a
        // warning, and its file is kept as it is.
        for tab in manifest.tabs.iter().filter(|t| t.kind == "report") {
            let path = format!("reports/{}/design.json", tab.name);
            let Some(text) = package.text(&path)? else {
                warnings.push(format!(
                    "The manifest lists the report {}, but the file has no {path}.",
                    tab.name
                ));
                continue;
            };
            match cliotree_core::report::ReportDesign::from_json(text) {
                Ok(design) => {
                    used.insert(path);
                    calc.reports.insert(tab.name.clone(), design);
                }
                Err(e) => warnings.push(format!("The report {} is not used: {e}", tab.name)),
            }
        }
        calc.order = manifest
            .tabs
            .iter()
            .filter(|t| calc.tab_kind(&t.name).is_some())
            .map(|t| t.name.clone())
            .collect();

        for name in &manifest.modules {
            if !safe_part(name) {
                return Err(PackageError::BadManifest(format!(
                    "the module name {name:?} cannot be used as a file name"
                )));
            }
            let path = format!("modules/{name}.py");
            let Some(source) = package.text(&path)? else {
                warnings.push(format!(
                    "The manifest lists the module {name}, but the file has no {path}."
                ));
                continue;
            };
            used.insert(path);
            if let Err(e) = calc.add_module(name, source) {
                warnings.push(format!("Module {name} is not used: {e}"));
            }
        }

        if let Some(text) = package.text(SETTINGS)? {
            used.insert(SETTINGS.to_owned());
            let (settings, problems) = WorkbookSettings::from_json(text);
            warnings.extend(problems);
            calc.store_settings(settings);
        }

        // The view of the workbook only changes what is drawn, so one that
        // cannot be read is dropped with a warning.
        let mut workbook_view = None;
        if let Some(text) = package.text(VIEW)? {
            used.insert(VIEW.to_owned());
            match serde_json::from_str::<Json>(text) {
                Ok(view @ Json::Object(_)) => workbook_view = Some(view),
                _ => warnings.push(format!(
                    "{VIEW} is not a JSON object, so the workbook's view settings are the defaults."
                )),
            }
        }

        // Nothing has been run, edited or undone yet.
        calc.dirty.clear();
        calc.clear_history();
        calc.mark_saved();

        let unknown = package
            .entries()
            .filter(|(path, _)| !used.contains(*path))
            .map(|(path, bytes)| (path.to_owned(), bytes.to_vec()))
            .collect();
        Ok(Opened {
            calc,
            workbook_id: manifest.workbook_id,
            views,
            workbook_view,
            unknown,
            warnings,
            format_version: (major, minor),
        })
    }

    fn read_tab(
        &mut self,
        package: &Package,
        tab: &TabEntry,
        binding: Binding,
        used: &mut HashSet<String>,
        views: &mut BTreeMap<String, Json>,
        warnings: &mut Vec<String>,
    ) -> Result<(), PackageError> {
        let name = tab.name.as_str();
        let fail = |message: String| PackageError::Tab {
            tab: name.to_owned(),
            message,
        };
        let language = match &binding {
            Binding::JsonSchema | Binding::Copybook => SchemaFormat::Json,
            Binding::Xsd => SchemaFormat::Xsd,
            Binding::Sheet(schema_sheet) => {
                if !self.is_schema_sheet(schema_sheet) {
                    return Err(fail(format!(
                        "the schema tab {schema_sheet} is not in the file"
                    )));
                }
                self.schema_format(schema_sheet)
                    .unwrap_or(SchemaFormat::Json)
            }
        };
        let format = match &tab.format {
            None if language == SchemaFormat::Xsd => DataFormat::Xml,
            None => DataFormat::Json,
            Some(code) => match DataFormat::from_code(code) {
                Some(DataFormat::Json5) if matches!(binding, Binding::Copybook) => {
                    DataFormat::Json5
                }
                Some(DataFormat::Blk) if language == SchemaFormat::Copybook => DataFormat::Blk,
                Some(format)
                    if format.json_shaped()
                        && language != SchemaFormat::Xsd
                        && matches!(binding, Binding::Sheet(_)) =>
                {
                    format
                }
                _ => {
                    return Err(fail(format!(
                        "the manifest gives it the format {code:?}, which this version cannot \
                         use for it"
                    )));
                }
            },
        };
        let path = format!("sheets/{name}/data.{}", format.extension());
        if package.get(&path).is_none() {
            return Err(fail(format!("the file has no {path}")));
        }
        used.insert(path.clone());
        // COBOL records are bytes, not text.
        let text = if format == DataFormat::Blk {
            ""
        } else {
            package.text(&path)?.unwrap_or_default()
        };
        match &binding {
            Binding::JsonSchema => {
                let parsed = json::parse(text).map_err(|e| bad(&path, e))?;
                self.add_schema_sheet(name, parsed.node)
                    .map_err(|e| fail(e.to_string()))?;
            }
            Binding::Xsd => {
                let spec = cliotree_core::metaschema::xsd_meta();
                let parsed = xml::parse(text, spec).map_err(|e| bad(&path, e))?;
                self.add_xsd_sheet(name, parsed.node, parsed.document)
                    .map_err(|e| fail(e.to_string()))?;
            }
            Binding::Copybook => {
                let parsed = format.read(text).map_err(|e| bad(&path, e))?;
                self.add_copybook_sheet(name, parsed.node)
                    .map_err(|e| fail(e.to_string()))?;
            }
            Binding::Sheet(schema_sheet) => {
                let spec = self
                    .lowered_for(schema_sheet, tab.record.as_deref())
                    .map_err(|e| {
                        fail(format!(
                            "its schema tab {schema_sheet} cannot be used as a type: {e}"
                        ))
                    })?;
                let (node, document) = if spec.xml().is_some() {
                    let parsed = xml::parse(text, &spec).map_err(|e| bad(&path, e))?;
                    (parsed.node, Some(parsed.document))
                } else if let (DataFormat::Blk, Some(layout)) = (format, spec.copybook()) {
                    let bytes = package.get(&path).unwrap_or_default();
                    let parsed =
                        cliotree_core::copybook::records::read(bytes, layout, &tab.redefines)
                            .map_err(|e| bad(&path, e))?;
                    (parsed.node, None)
                } else {
                    (format.read(text).map_err(|e| bad(&path, e))?.node, None)
                };
                self.add_sheet(name, spec, node)
                    .map_err(|e| fail(e.to_string()))?;
                if let Some(record) = &tab.record {
                    self.sheet_mut(name)
                        .map_err(|e| fail(e.to_string()))?
                        .record = Some(record.clone());
                }
                if tab.recfm.as_deref() == Some("VB") {
                    self.sheet_mut(name)
                        .map_err(|e| fail(e.to_string()))?
                        .variable = true;
                }
                self.bind_schema(name, schema_sheet)
                    .map_err(|e| fail(e.to_string()))?;
                if !matches!(format, DataFormat::Json | DataFormat::Blk) {
                    self.store_format(name, format)
                        .map_err(|e| fail(e.to_string()))?;
                }
                for (set, member) in tab.redefines.iter().enumerate() {
                    self.store_redefine(name, set, *member)
                        .map_err(|e| fail(e.to_string()))?;
                }
                if let Some(document) = document {
                    let _ = self.set_xml_document(name, document);
                }
            }
        }
        let overlay_path = format!("sheets/{name}/overlay.json");
        if let Some(text) = package.text(&overlay_path)? {
            used.insert(overlay_path.clone());
            let overlay: Overlay = serde_json::from_str(text).map_err(|e| bad(&overlay_path, e))?;
            if let Some(view) = overlay.view {
                views.insert(name.to_owned(), view);
            }
            for comment in overlay.comments {
                if let Err(message) = self.restore_comment(name, &comment) {
                    warnings.push(format!(
                        "Tab {name}: the comment on {} is dropped: {message}",
                        comment.path
                    ));
                }
            }
            for column in overlay.columns {
                if let Err(message) = self.restore_column(name, &column) {
                    warnings.push(format!(
                        "Tab {name}: the column formula on {}[].{} is dropped: {message}",
                        column.array, column.field
                    ));
                }
            }
            for (path_text, entry) in overlay.formulas {
                if let Err(message) = self.restore_formula(name, &path_text, entry) {
                    warnings.push(format!(
                        "Tab {name}: the formula at {path_text} is dropped: {message}"
                    ));
                }
            }
            self.restore_column_items(name);
        }
        Ok(())
    }

    /// Puts a comment kept in the overlay back on its node, recording
    /// nothing.
    fn restore_comment(&mut self, sheet: &str, comment: &OverlayComment) -> Result<(), String> {
        let path: Path = comment.path.parse().map_err(|e| format!("{e}"))?;
        let place = match comment.place.as_deref() {
            None | Some("before") => CommentPlace::Before,
            Some("start") => CommentPlace::Start,
            Some("end") => CommentPlace::End,
            Some(other) => return Err(format!("{other} is not a place for a comment")),
        };
        let s = self.sheet_mut(sheet).map_err(|e| e.to_string())?;
        let node = s.data.get_mut(&path).ok_or("the node is not there")?;
        node.set_comment_at(place, Some(comment.text.clone()));
        Ok(())
    }

    /// Puts a column formula back. The formulas of its items follow, once the
    /// ones that were saved with an error are back.
    fn restore_column(&mut self, sheet: &str, column: &OverlayColumn) -> Result<(), String> {
        let array: Path = column.array.parse().map_err(|e| format!("{e}"))?;
        self.column_length(sheet, &array, &column.field)
            .map_err(|e| e.to_string())?;
        self.put_column(sheet, &array, &column.field, Some(&column.formula));
        Ok(())
    }

    /// Gives every item of every column the formula of the column, where it
    /// has none, without marking anything out of date.
    fn restore_column_items(&mut self, sheet: &str) {
        for column in self.column_formulas(sheet).to_vec() {
            let length = match self.data(sheet).and_then(|d| d.get(&column.array)) {
                Some(node) => match node.value() {
                    Value::Array(items) => items.len(),
                    _ => 0,
                },
                None => 0,
            };
            for index in 0..length {
                let place = column.place(index);
                let entry = OverlayFormula {
                    formula: Some(column.text.clone()),
                    status: OverlayStatus::Ok,
                };
                let has = self
                    .sheets
                    .get(sheet)
                    .is_some_and(|s| s.formulas.by_path.contains_key(&place));
                if !has {
                    let _ = self.restore_formula(sheet, &place.to_string(), entry);
                }
            }
        }
    }

    /// Puts a formula back as it was saved: its text, and the error it ended
    /// in, which the node holds again (the data file leaves it absent). Nothing
    /// is checked beyond where it can go, nothing is recorded, and nothing is
    /// run.
    fn restore_formula(
        &mut self,
        sheet: &str,
        path_text: &str,
        entry: OverlayFormula,
    ) -> Result<(), String> {
        let path: Path = path_text.parse().map_err(|e| format!("{e}"))?;
        // An item of a column has the column's formula.
        let text = match entry.formula.clone() {
            Some(text) => text,
            None => self
                .column_of(sheet, &path)
                .map(|c| c.text.clone())
                .ok_or("it has no formula and is not in a column")?,
        };
        let syntax = self.syntax.check(&text);
        let s = self.sheets.get_mut(sheet).ok_or("no such tab")?;
        if s.spec.resolve(&path).is_none() {
            return Err("the schema defines no node there".to_owned());
        }
        if s.formulas.computed_by(&path).is_some() || !s.formulas.below(&path).is_empty() {
            return Err("it overlaps another formula".to_owned());
        }
        let mut formula = Formula {
            text,
            deps: Vec::new(),
            error: None,
            traceback: None,
            syntax_error: false,
        };
        if let Err(e) = &syntax {
            formula.syntax_error = true;
            formula.error = Some(ErrorValue {
                kind: ErrorKind::Syntax,
                message: e.to_string(),
                origin: Some(path.clone()),
            });
        } else if let OverlayStatus::Error(e) = entry.status {
            let kind = ErrorKind::from_code(&e.kind)
                .ok_or_else(|| format!("{:?} is not an error kind", e.kind))?;
            formula.error = Some(ErrorValue {
                kind,
                message: e.message,
                origin: e.origin.and_then(|o| o.parse().ok()),
            });
            formula.traceback = e.traceback;
        }
        if let Some(error) = &formula.error {
            let computed = Computed::Error(error.clone());
            let _ = store(&s.spec, &mut s.data, &path, &computed);
        }
        Arc::make_mut(&mut s.formulas).insert(path, formula);
        Ok(())
    }
}

fn short(revision: &str) -> &str {
    revision.get(..12).unwrap_or(revision)
}

/// The comments of a tree, by path, for the overlay of a tab whose file has
/// no place for them.
fn overlay_comments(data: &Node) -> Vec<OverlayComment> {
    let mut out = Vec::new();
    for path in super::formats::commented(data, &Path::root()) {
        let Some(node) = data.get(&path) else {
            continue;
        };
        for place in CommentPlace::ALL {
            if let Some(text) = node.comment_at(place) {
                out.push(OverlayComment {
                    path: path.to_string(),
                    place: match place {
                        CommentPlace::Before => None,
                        CommentPlace::Start => Some("start".to_owned()),
                        CommentPlace::End => Some("end".to_owned()),
                    },
                    text: text.to_owned(),
                });
            }
        }
    }
    out
}
