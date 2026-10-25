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

use cliotree_core::form::FormDesign;
use cliotree_core::{ErrorKind, ErrorValue, Path, json, xml};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use super::*;

/// The format version this build writes. A file of the same major version
/// opens; a newer minor version opens with a warning; a newer major version is
/// refused.
pub const FORMAT_MAJOR: u32 = 1;
pub const FORMAT_MINOR: u32 = 0;

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
    /// `builtin:jsonschema`, `builtin:xsd`, or `sheet:<Tab>`.
    schema: String,
    /// Whether the tabs that use this schema tab follow its changes.
    #[serde(default, skip_serializing_if = "is_false")]
    auto_update: bool,
    /// The customised forms of the tab, in display order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    forms: Vec<String>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// How a tab is tied to its schema, as the manifest writes it.
enum Binding {
    JsonSchema,
    Xsd,
    Sheet(String),
}

impl Binding {
    fn parse(text: &str) -> Option<Binding> {
        match text {
            "builtin:jsonschema" => Some(Binding::JsonSchema),
            "builtin:xsd" => Some(Binding::Xsd),
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
    /// The state of the view, which the application owns and this layer only
    /// carries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    view: Option<Json>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OverlayFormula {
    /// The expression, without the `=`.
    formula: String,
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
                    },
                    *auto_update,
                ),
                SchemaRole::Bound(schema_sheet) => (format!("sheet:{schema_sheet}"), false),
            };

            let is_xml = sheet.spec.xml().is_some();
            let data = if is_xml {
                let document = sheet.xml.clone().unwrap_or_default();
                xml::to_string(&sheet.data, &sheet.spec, &document, true)
                    .map_err(|e| unsavable(format!("tab {name}: {e}")))?
            } else {
                json::to_string(&sheet.data, true)
                    .map_err(|e| unsavable(format!("tab {name}: {e}")))?
            };
            files.push((
                format!("sheets/{name}/data.{}", if is_xml { "xml" } else { "json" }),
                lf(data),
            ));

            let overlay = self.overlay_of(name, sheet, meta.views.get(name).cloned());
            if !overlay.formulas.is_empty() || overlay.view.is_some() {
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
            });
        }

        for (name, module) in &self.modules {
            if !safe_part(name) {
                return Err(unsavable(format!(
                    "the module name {name:?} cannot be used as a file name"
                )));
            }
            files.push((format!("modules/{name}.py"), lf(module.source.to_string())));
        }

        let manifest = Manifest {
            format_version: version_text(FORMAT_MAJOR, FORMAT_MINOR),
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
            formulas.insert(
                path.to_string(),
                OverlayFormula {
                    formula: formula.text.clone(),
                    status,
                },
            );
        }
        Overlay { formulas, view }
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
            if tab.kind != "data" {
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
        let xml_data = match &binding {
            Binding::JsonSchema => false,
            Binding::Xsd => true,
            Binding::Sheet(schema_sheet) => {
                if !self.is_schema_sheet(schema_sheet) {
                    return Err(fail(format!(
                        "the schema tab {schema_sheet} is not in the file"
                    )));
                }
                self.schema_format(schema_sheet) == Some(SchemaFormat::Xsd)
            }
        };
        let path = format!(
            "sheets/{name}/data.{}",
            if xml_data { "xml" } else { "json" }
        );
        let text = package
            .text(&path)?
            .ok_or_else(|| fail(format!("the file has no {path}")))?;
        used.insert(path.clone());
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
            Binding::Sheet(schema_sheet) => {
                let spec = self.lowered(schema_sheet).map_err(|e| {
                    fail(format!(
                        "its schema tab {schema_sheet} cannot be used as a type: {e}"
                    ))
                })?;
                let (node, document) = if spec.xml().is_some() {
                    let parsed = xml::parse(text, &spec).map_err(|e| bad(&path, e))?;
                    (parsed.node, Some(parsed.document))
                } else {
                    (json::parse(text).map_err(|e| bad(&path, e))?.node, None)
                };
                self.add_sheet(name, spec, node)
                    .map_err(|e| fail(e.to_string()))?;
                self.bind_schema(name, schema_sheet)
                    .map_err(|e| fail(e.to_string()))?;
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
            for (path_text, entry) in overlay.formulas {
                if let Err(message) = self.restore_formula(name, &path_text, entry) {
                    warnings.push(format!(
                        "Tab {name}: the formula at {path_text} is dropped: {message}"
                    ));
                }
            }
        }
        Ok(())
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
        let syntax = self.syntax.check(&entry.formula);
        let s = self.sheets.get_mut(sheet).ok_or("no such tab")?;
        if s.spec.resolve(&path).is_none() {
            return Err("the schema defines no node there".to_owned());
        }
        if s.formulas.computed_by(&path).is_some() || !s.formulas.below(&path).is_empty() {
            return Err("it overlaps another formula".to_owned());
        }
        let mut formula = Formula {
            text: entry.formula,
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
