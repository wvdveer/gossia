//! Importing a standalone file as tabs of the workbook (DESIGN.md, "Import and
//! export", "Importing a file"): from File > Import..., and by dropping files
//! on the window.
//!
//! Pasting in the Raw view imports the pasted text as if it were a file, its
//! format told from the text itself.
//!
//! A file is read into an [`ImportPlan`] first, which nothing in the workbook
//! sees until the user confirms the dialog. The plan has every schema tab the
//! data could be given, with how much of the data would not fit each one, and
//! a schema inferred from the data. The dialog starts on the first schema tab
//! the data fits completely, and on the inferred schema when none does.

use std::collections::VecDeque;
use std::path::Path as FsPath;

use cliotree_calc::SchemaFormat;
use cliotree_calc::names::{self, Subject};
use cliotree_core::dataformat::DataFormat;
use cliotree_core::infer::{self, JsonInference};
use cliotree_core::validate::validate;
use cliotree_core::xml::XmlDocument;
use cliotree_core::{Node, Value, delimited, json, metaschema, xml, xmltree, xsd};

use crate::app::ClioApp;
use crate::view::{error_color, named};

/// How the file is read, from its extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportFormat {
    Json,
    Xml,
    /// JSON5, JSONC and JSON Lines: JSON-shaped, each a tab format of its own
    /// (DESIGN.md, "Other file formats").
    Json5,
    Jsonc,
    JsonLines,
    Toml,
    /// YAML of one document, and of several, which is a list of them.
    Yaml,
    YamlDocuments,
    /// Comma-separated values: an array with an object for each row.
    Csv,
    /// Tab-separated values, read as CSV is.
    Tsv,
    /// Anything else: an array of its lines.
    Text,
}

impl ImportFormat {
    /// What the format is called in messages.
    pub fn label(self) -> &'static str {
        match self {
            ImportFormat::Json => "JSON",
            ImportFormat::Xml => "XML",
            ImportFormat::Json5 => "JSON5",
            ImportFormat::Jsonc => "JSON with comments",
            ImportFormat::JsonLines => "JSON Lines",
            ImportFormat::Toml => "TOML",
            ImportFormat::Yaml => "YAML",
            ImportFormat::YamlDocuments => "YAML documents",
            ImportFormat::Csv => "comma-separated values",
            ImportFormat::Tsv => "tab-separated values",
            ImportFormat::Text => "lines of text",
        }
    }

    /// The format of the data tab it makes: its own for the tab formats, and
    /// JSON for CSV, TSV and text, which are read into JSON.
    pub fn data_format(self) -> DataFormat {
        match self {
            ImportFormat::Xml => DataFormat::Xml,
            ImportFormat::Json5 => DataFormat::Json5,
            ImportFormat::Jsonc => DataFormat::Jsonc,
            ImportFormat::JsonLines => DataFormat::JsonLines,
            ImportFormat::Toml => DataFormat::Toml,
            ImportFormat::Yaml => DataFormat::Yaml,
            ImportFormat::YamlDocuments => DataFormat::YamlDocuments,
            ImportFormat::Json | ImportFormat::Csv | ImportFormat::Tsv | ImportFormat::Text => {
                DataFormat::Json
            }
        }
    }

    /// The format, given the text: YAML of several documents is read as
    /// YAML documents.
    pub fn refine(self, text: &str) -> ImportFormat {
        match self.data_format().refine(text) {
            DataFormat::YamlDocuments => ImportFormat::YamlDocuments,
            _ => self,
        }
    }
}

/// The schema inferred from the data, ready to be a schema tab.
#[derive(Debug, Clone)]
pub enum Inferred {
    Json(JsonInference),
    Xsd(Box<InferredXsd>),
}

/// An XSD inferred from XML data, with the data read with it.
#[derive(Debug, Clone)]
pub struct InferredXsd {
    pub text: String,
    pub warnings: Vec<String>,
    /// The XSD as the schema tab holds it.
    pub tree: Node,
    pub document: XmlDocument,
    /// The data, read with the inferred schema.
    pub data: Node,
    pub data_document: XmlDocument,
}

impl Inferred {
    fn warnings(&self) -> &[String] {
        match self {
            Inferred::Json(j) => &j.warnings,
            Inferred::Xsd(x) => &x.warnings,
        }
    }

    /// The schema as text, for the report.
    pub fn text(&self) -> String {
        match self {
            Inferred::Json(j) => json::to_string(&j.schema, true).unwrap_or_default(),
            Inferred::Xsd(x) => x.text.clone(),
        }
    }
}

/// A file that is itself a schema, ready to be a schema tab.
#[derive(Debug, Clone)]
pub enum SchemaFile {
    /// A JSON Schema document.
    Json(Node),
    /// An XSD, as the XSD meta-schema types it.
    Xsd { tree: Node, document: XmlDocument },
}

/// A schema tab the data could be given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub schema: String,
    /// How many problems validation finds in the data under it.
    pub problems: usize,
}

/// Which schema the imported tab is bound to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaChoice {
    /// No data tab: the file is a schema, and becomes a schema tab.
    SchemaTab,
    /// A new schema tab, holding the inferred schema.
    Inferred,
    /// A schema tab that is there already.
    Existing(String),
    /// No new tab: the text replaces the data of this tab, if it conforms to
    /// its schema. Offered for a paste in the tab's own format.
    Replace(String),
}

/// A file read for importing, and the choices of the dialog.
#[derive(Debug, Clone)]
pub struct ImportPlan {
    /// The file name, for the dialog and the undo entry.
    pub file: String,
    pub format: ImportFormat,
    /// The text of the file, which an XML file is read from again for the
    /// schema it is given.
    text: String,
    /// JSON or text data, without its nulls.
    data: Option<Node>,
    /// What reading the file noticed: dropped nulls, a dropped DOCTYPE,
    /// oversized numbers.
    pub notes: Vec<String>,
    /// The inferred schema, or why there is none (mixed content, more than one
    /// namespace).
    pub inferred: Result<Inferred, String>,
    pub candidates: Vec<Candidate>,
    /// The name of the data tab.
    pub name: String,
    /// The name of the schema tab, for an inferred schema.
    pub schema_name: String,
    pub choice: SchemaChoice,
    /// The tab whose data a paste may replace instead.
    pub replace: Option<String>,
    /// The file as a schema, when it is one (an XSD, or JSON that names the
    /// JSON Schema meta-schema or is called `*.schema.json`).
    pub schema_file: Option<SchemaFile>,
    /// Why the schema file cannot be used as a schema as it is, if it
    /// cannot. It is still imported, and can be corrected on its tab.
    pub schema_problem: Option<String>,
    /// Why the last attempt was refused.
    pub error: Option<String>,
}

impl ImportPlan {
    /// What the report lists for the choice made.
    pub fn report(&self) -> Vec<String> {
        let mut lines = self.notes.clone();
        match &self.choice {
            SchemaChoice::Inferred => {
                if let Ok(inferred) = &self.inferred {
                    lines.extend(inferred.warnings().iter().cloned());
                }
            }
            SchemaChoice::Existing(schema) => {
                if let Some(c) = self.candidates.iter().find(|c| &c.schema == schema)
                    && c.problems > 0
                {
                    lines.push(format!(
                        "{} in the data {} not fit {schema}; {} kept and flagged.",
                        plural(c.problems, "value", "values"),
                        if c.problems == 1 { "does" } else { "do" },
                        if c.problems == 1 { "it is" } else { "they are" },
                    ));
                }
            }
            SchemaChoice::Replace(_) => {}
            SchemaChoice::SchemaTab => {
                if let Some(why) = &self.schema_problem {
                    lines.push(format!(
                        "It cannot be used as a schema as it is: {why}. It can be corrected on its tab."
                    ));
                }
            }
        }
        lines
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

/// The format of a file, by its extension.
pub fn format_of(file: &str) -> ImportFormat {
    let extension = FsPath::new(file)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase());
    match extension.as_deref() {
        Some("json") => ImportFormat::Json,
        Some("xml" | "xsd") => ImportFormat::Xml,
        Some("json5") => ImportFormat::Json5,
        Some("jsonc") => ImportFormat::Jsonc,
        Some("jsonl" | "ndjson") => ImportFormat::JsonLines,
        Some("toml") => ImportFormat::Toml,
        Some("yaml" | "yml") => ImportFormat::Yaml,
        Some("csv") => ImportFormat::Csv,
        Some("tsv" | "tab") => ImportFormat::Tsv,
        _ => ImportFormat::Text,
    }
}

/// The format of pasted text, from the text: `{` or `[` first is JSON, `<`
/// is XML, a tab in the first line is tab-separated values (what a copied
/// range of a spreadsheet is), a comma comma-separated values, and anything
/// else lines of text.
pub fn format_of_text(text: &str) -> ImportFormat {
    let text = text.trim_start_matches('\u{FEFF}').trim_start();
    let first_line = text.lines().next().unwrap_or_default();
    if is_json_lines(text) {
        ImportFormat::JsonLines
    } else if is_toml(text) {
        ImportFormat::Toml
    } else if text.starts_with(['{', '[']) {
        ImportFormat::Json
    } else if text.starts_with('<') {
        ImportFormat::Xml
    } else if is_yaml(text) {
        ImportFormat::Yaml.refine(text)
    } else if first_line.contains('\t') {
        ImportFormat::Tsv
    } else if first_line.contains(',') {
        ImportFormat::Csv
    } else {
        ImportFormat::Text
    }
}

/// Whether the text is JSON Lines: two or more lines, each a JSON object on
/// its own (blank lines aside).
fn is_json_lines(text: &str) -> bool {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    lines.len() >= 2
        && lines
            .iter()
            .all(|l| l.starts_with('{') && l.ends_with('}') && json::parse(l).is_ok())
}

/// Whether the text is TOML: not JSON, its first line that is not blank or a
/// comment a `[table]` header or `key = value`, and the whole of it TOML.
fn is_toml(text: &str) -> bool {
    let Some(first) = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))
    else {
        return false;
    };
    let key = |k: &str| {
        !k.is_empty()
            && k.chars()
                .all(|c| c.is_ascii_alphanumeric() || "_-.\"' ".contains(c))
    };
    let header = first
        .strip_prefix('[')
        .and_then(|l| l.split('#').next())
        .map(str::trim_end)
        .and_then(|l| l.strip_suffix(']'))
        .is_some_and(|l| key(l.trim_start_matches('[').trim_end_matches(']')));
    let assignment = first.split_once('=').is_some_and(|(k, _)| key(k.trim()));
    (header || assignment) && json::parse(text).is_err() && DataFormat::Toml.read(text).is_ok()
}

/// Whether the text is YAML: its first line that is not blank or a comment
/// is `---`, `key: value`, `key:` or `- item`, and the whole of it a YAML
/// table or list (any text at all is a YAML string).
fn is_yaml(text: &str) -> bool {
    let Some(first) = text
        .lines()
        .map(str::trim_end)
        .find(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
    else {
        return false;
    };
    let looks = first.starts_with("---")
        || first.trim_start().starts_with("- ")
        || first.contains(": ")
        || first.ends_with(':');
    looks
        && cliotree_core::yaml::parse(text)
            .is_ok_and(|p| matches!(p.node.value(), Value::Object(_) | Value::Array(_)))
}

/// What the dialog and the undo entry call a paste.
pub const PASTED: &str = "the pasted text";

/// A valid tab name made from a file name: characters a name cannot have
/// become `_`, a name that would start with a digit or `_` gets `Data` in
/// front, and a reserved word or a name in use gets a number after it.
pub fn name_from_file(file: &str, taken: impl Fn(&str) -> bool) -> String {
    let stem = FsPath::new(file)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut name: String = stem
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if name.is_empty() {
        name = "Data".to_owned();
    } else if name.starts_with(|c: char| c.is_ascii_digit() || c == '_') {
        name = format!("Data{name}");
    }
    unique(&name, taken)
}

/// `base`, or `base2`, `base3`... the first that is a valid tab name and not
/// taken.
pub fn unique(base: &str, taken: impl Fn(&str) -> bool) -> String {
    let fits = |n: &str| names::problem(n, Subject::Tab).is_none() && !taken(n);
    if fits(base) {
        return base.to_owned();
    }
    (2..)
        .map(|k| format!("{base}{k}"))
        .find(|n| fits(n))
        .expect("some number makes the name free")
}

impl ClioApp {
    /// Reads a file and opens the import dialog for it.
    pub fn begin_import_path(&mut self, path: &FsPath) -> Result<(), String> {
        let limit = self.user.prefs.max_import_bytes;
        if let Ok(meta) = std::fs::metadata(path)
            && meta.len() > limit
        {
            return Err(too_large(&path.display().to_string(), limit));
        }
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let file = path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        let plan = self.plan_import(&file, &bytes)?;
        self.imports.push_back(plan);
        Ok(())
    }

    /// What dropping files on the window does: a workbook is opened (asking
    /// about unsaved changes first), and any other file is imported, one
    /// dialog after another. A file that cannot be read says why.
    pub fn drop_files(&mut self, paths: Vec<std::path::PathBuf>) {
        let mut problems = Vec::new();
        for path in paths {
            let workbook = path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case(crate::workbook_file::EXTENSION));
            if workbook {
                self.begin_open_recent(&path);
            } else if let Err(e) = self.begin_import_path(&path) {
                problems.push(e);
            }
        }
        if !problems.is_empty() {
            self.notice = Some(problems.join(" "));
        }
    }

    /// Reads a file for importing. Nothing is added to the workbook.
    pub fn plan_import(&self, file: &str, bytes: &[u8]) -> Result<ImportPlan, String> {
        self.plan_import_as(file, file, format_of(file), bytes)
    }

    /// Reads pasted text for importing, as a file of the format the text
    /// shows. The tab is called `Pasted`. When the text is in the format of
    /// the tab shown, the dialog also offers to replace that tab's data.
    pub fn plan_paste(&self, text: &str) -> Result<ImportPlan, String> {
        let format = format_of_text(text);
        let mut plan = self.plan_import_as(PASTED, "Pasted", format, text.as_bytes())?;
        let same_format = match format {
            ImportFormat::Csv | ImportFormat::Tsv | ImportFormat::Text => false,
            _ => self.calc().format(&self.sheet) == Some(format.data_format()),
        };
        if same_format && !self.calc().is_schema_sheet(&self.sheet) {
            plan.replace = Some(self.sheet.clone());
        }
        Ok(plan)
    }

    /// Opens the import dialog for pasted text, or says why it cannot be read.
    pub fn begin_paste(&mut self, text: &str) -> Result<(), String> {
        let plan = self.plan_paste(text)?;
        self.imports.push_back(plan);
        Ok(())
    }

    /// Reads `bytes` as `format`; `file` names it in messages and `name_from`
    /// gives the tab its name.
    fn plan_import_as(
        &self,
        file: &str,
        name_from: &str,
        format: ImportFormat,
        bytes: &[u8],
    ) -> Result<ImportPlan, String> {
        let limit = self.user.prefs.max_import_bytes;
        if bytes.len() as u64 > limit {
            return Err(too_large(file, limit));
        }
        let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
        let text = String::from_utf8(bytes.to_vec()).map_err(|_| {
            format!(
                "{} is not UTF-8 text, so it cannot be imported.",
                capital(file)
            )
        })?;
        let format = format.refine(&text);
        let calc = self.calc();
        let mut notes = Vec::new();
        let mut schema_file = None;
        let mut schema_problem = None;
        let (data, inferred, candidates) = match format {
            ImportFormat::Json5
            | ImportFormat::Jsonc
            | ImportFormat::JsonLines
            | ImportFormat::Toml
            | ImportFormat::Yaml
            | ImportFormat::YamlDocuments => {
                let data_format = format.data_format();
                let parsed = data_format
                    .read(&text)
                    .map_err(|e| format!("{} is not {}: {e}", capital(file), format.label()))?;
                notes.extend(parsed.warnings.iter().map(|w| {
                    if w.path.is_root() {
                        format!("{}.", w.message)
                    } else {
                        format!("{}: {}.", w.path, w.message)
                    }
                }));
                let inference = infer::infer_json(&parsed.node);
                notes.extend(inference.dropped.iter().cloned());
                let data = inference.data.clone();
                let candidates = self.schema_tabs(SchemaFormat::Json, |spec| {
                    data_format
                        .root_problem(&spec.get(spec.root()).kind)
                        .is_none()
                        .then(|| validate(spec, &data).len())
                });
                (Some(data), Ok(Inferred::Json(inference)), candidates)
            }
            ImportFormat::Json | ImportFormat::Text | ImportFormat::Csv | ImportFormat::Tsv => {
                let inference = if format == ImportFormat::Json {
                    let parsed = json::parse_lenient(&text)
                        .map_err(|e| format!("{} is not JSON: {e}", capital(file)))?;
                    notes.extend(parsed.warnings.iter().map(|w| {
                        if w.path.is_root() {
                            format!("{}.", w.message)
                        } else {
                            format!("{}: {}.", w.path, w.message)
                        }
                    }));
                    if is_json_schema(file, &parsed.node) {
                        schema_problem = cliotree_core::jsonschema::lower(&parsed.node)
                            .err()
                            .map(|e| e.to_string());
                        schema_file = Some(SchemaFile::Json(parsed.node.clone()));
                    }
                    infer::infer_json(&parsed.node)
                } else if format == ImportFormat::Csv {
                    delimited::infer_delimited(&text, ',')
                } else if format == ImportFormat::Tsv {
                    delimited::infer_delimited(&text, '\t')
                } else {
                    infer::text_lines(&text)
                };
                notes.extend(inference.dropped.iter().cloned());
                let data = inference.data.clone();
                let candidates =
                    self.schema_tabs(SchemaFormat::Json, |spec| Some(validate(spec, &data).len()));
                (Some(data), Ok(Inferred::Json(inference)), candidates)
            }
            ImportFormat::Xml => {
                let doc = xmltree::parse(&text)
                    .map_err(|e| format!("{} is not well-formed XML: {e}", capital(file)))?;
                if doc.root.name == "schema" && doc.root.namespace.as_deref() == Some(xsd::XSD_NS) {
                    let tree = xml::parse(&text, metaschema::xsd_meta())
                        .map_err(|e| format!("{} is not an XSD: {e}", capital(file)))?;
                    schema_problem = xsd::lower(&doc.root).err().map(|e| e.to_string());
                    schema_file = Some(SchemaFile::Xsd {
                        tree: tree.node,
                        document: tree.document,
                    });
                }
                let inferred = infer::infer_xsd(&doc).and_then(|inference| {
                    let spec = xsd::lower_str(&inference.text).map_err(|e| e.to_string())?;
                    let schema = xml::parse(&inference.text, metaschema::xsd_meta())
                        .map_err(|e| e.to_string())?;
                    let data = xml::parse(&text, &spec).map_err(|e| e.to_string())?;
                    Ok(Inferred::Xsd(Box::new(InferredXsd {
                        text: inference.text,
                        warnings: inference.warnings,
                        tree: schema.node,
                        document: schema.document,
                        data: data.node,
                        data_document: data.document,
                    })))
                });
                let candidates = self.schema_tabs(SchemaFormat::Xsd, |spec| {
                    let parsed = xml::parse(&text, spec).ok()?;
                    Some(validate(spec, &parsed.node).len())
                });
                (None, inferred, candidates)
            }
        };
        let taken = |n: &str| calc.tab_name_problem(n, None).is_some();
        let name = name_from_file(name_from, taken);
        let schema_name = unique(&format!("{name}Schema"), |n| {
            taken(n) || n.eq_ignore_ascii_case(&name)
        });
        let choice = match candidates.iter().find(|c| c.problems == 0) {
            _ if schema_file.is_some() => SchemaChoice::SchemaTab,
            Some(fits) => SchemaChoice::Existing(fits.schema.clone()),
            None if inferred.is_ok() => SchemaChoice::Inferred,
            None => match candidates.first() {
                Some(c) => SchemaChoice::Existing(c.schema.clone()),
                None => {
                    return Err(format!(
                        "{file} cannot be imported: {}.",
                        inferred.err().unwrap_or_default()
                    ));
                }
            },
        };
        Ok(ImportPlan {
            file: file.to_owned(),
            format,
            text,
            data,
            notes,
            inferred,
            candidates,
            name,
            schema_name,
            choice,
            replace: None,
            schema_file,
            schema_problem,
            error: None,
        })
    }

    /// The schema tabs of a format that can be used, each with what `check`
    /// says of the data under it; those it refuses are left out.
    fn schema_tabs(
        &self,
        format: SchemaFormat,
        check: impl Fn(&cliotree_core::TypeSpec) -> Option<usize>,
    ) -> Vec<Candidate> {
        let calc = self.calc();
        calc.sheet_names()
            // A schema with auto-update on takes one tab only.
            .filter(|s| {
                calc.schema_format(s) == Some(format)
                    && !(calc.auto_update(s) && !calc.schema_users(s).is_empty())
            })
            .filter_map(|s| {
                let spec = calc.schema_spec(s).ok()?;
                Some(Candidate {
                    schema: s.to_owned(),
                    problems: check(&spec)?,
                })
            })
            .collect()
    }

    /// Why the plan cannot be imported as it is, or None.
    pub fn import_problem(&self, plan: &ImportPlan) -> Option<String> {
        if let SchemaChoice::Replace(_) = plan.choice {
            return None;
        }
        let name = plan.name.trim();
        if let Some(why) = self.calc().tab_name_problem(name, None) {
            return Some(format!("The tab name: {why}."));
        }
        if plan.choice == SchemaChoice::Inferred {
            if let Err(why) = &plan.inferred {
                return Some(format!("No schema can be inferred: {why}."));
            }
            let schema = plan.schema_name.trim();
            if let Some(why) = self.calc().tab_name_problem(schema, None) {
                return Some(format!("The schema tab's name: {why}."));
            }
            if schema.eq_ignore_ascii_case(name) {
                return Some("The two tabs need different names.".to_owned());
            }
        }
        None
    }

    /// Adds the tabs of a plan: one entry in the history, *Import file.json*.
    /// An inferred schema has auto-update off, so that the next file of the
    /// same shape can be given it too. The data tab becomes the current tab.
    pub fn import(&mut self, plan: &ImportPlan) -> Result<(), String> {
        if self.session.running_procedure().is_some() {
            return Err("a button procedure is running; wait for it to finish".to_owned());
        }
        if !self.commit_editing() {
            return Err(self.notice.clone().unwrap_or_default());
        }
        if let Some(why) = self.import_problem(plan) {
            return Err(why);
        }
        if let SchemaChoice::Replace(tab) = &plan.choice {
            let tab = tab.clone();
            self.show_tab(&tab);
            return self.paste_document(&plan.text);
        }
        if plan.choice == SchemaChoice::SchemaTab {
            let name = plan.name.trim();
            let calc = self.session.calc_mut();
            let done = match &plan.schema_file {
                Some(SchemaFile::Json(node)) => calc.create_json_schema_tab(name, node.clone()),
                Some(SchemaFile::Xsd { tree, document }) => {
                    calc.create_xsd_schema_tab(name, tree.clone(), document.clone())
                }
                None => return Err("the file is not a schema".to_owned()),
            };
            done.map_err(|e| e.to_string())?;
            self.follow_tabs();
            self.show_tab(name);
            self.notice = None;
            self.session.tick();
            return Ok(());
        }
        let name = plan.name.trim();
        let schema_name = plan.schema_name.trim();
        let before = self.calc().history_len().0;
        let calc = self.session.calc_mut();
        calc.begin_entry(&format!("Import {}", plan.file));
        let done = match (&plan.choice, &plan.inferred) {
            (SchemaChoice::Inferred, Ok(Inferred::Json(inference))) => calc
                .create_json_schema_tab(schema_name, inference.schema.clone())
                .and_then(|()| {
                    calc.create_data_tab(name, schema_name, inference.data.clone(), None)
                }),
            (SchemaChoice::Inferred, Ok(Inferred::Xsd(x))) => calc
                .create_xsd_schema_tab(schema_name, x.tree.clone(), x.document.clone())
                .and_then(|()| {
                    calc.create_data_tab(
                        name,
                        schema_name,
                        x.data.clone(),
                        Some(x.data_document.clone()),
                    )
                }),
            (SchemaChoice::Inferred, Err(why)) => Err(cliotree_calc::EditError::Path(why.clone())),
            (SchemaChoice::Replace(_) | SchemaChoice::SchemaTab, _) => {
                unreachable!("replacing and schema tabs returned above")
            }
            (SchemaChoice::Existing(schema), _) => match &plan.data {
                Some(data) => calc.create_data_tab(name, schema, data.clone(), None),
                None => calc
                    .schema_spec(schema)
                    .and_then(|spec| {
                        xml::parse(&plan.text, &spec)
                            .map_err(|e| cliotree_calc::EditError::Path(e.to_string()))
                    })
                    .and_then(|parsed| {
                        calc.create_data_tab(name, schema, parsed.node, Some(parsed.document))
                    }),
            },
        };
        // A tab format of its own is set in the same entry.
        let done = done.and_then(|()| match plan.format.data_format() {
            DataFormat::Json | DataFormat::Xml => Ok(()),
            format => calc.set_format(name, format),
        });
        calc.end_entry();
        if let Err(e) = done {
            // A tab made before the failure is taken out again.
            if self.calc().history_len().0 > before {
                let _ = self.session.calc_mut().undo();
                let _ = self.session.calc_mut().take_removed();
            }
            return Err(e.to_string());
        }
        self.follow_tabs();
        self.show_tab(name);
        self.notice = None;
        self.session.tick();
        Ok(())
    }

    /// The import dialog of the first file waiting for one.
    pub(crate) fn import_window(&mut self, ctx: &egui::Context) {
        let Some(mut plan) = self.imports.pop_front() else {
            return;
        };
        let problem = self.import_problem(&plan);
        let (mut apply, mut cancel, mut open) = (false, false, true);
        egui::Window::new(format!("Import {}", plan.file))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .min_width(460.0)
            .max_width(560.0)
            .show(ctx, |ui| {
                if plan.file == PASTED {
                    ui.weak(format!("Read as {}.", plan.format.label()));
                }
                let replacing = matches!(plan.choice, SchemaChoice::Replace(_));
                ui.horizontal(|ui| {
                    ui.label("Tab name");
                    let response =
                        ui.add_enabled(!replacing, egui::TextEdit::singleline(&mut plan.name));
                    named(&response, "Import tab name");
                });
                ui.add_space(4.0);
                if plan.schema_file.is_some() {
                    let chosen = plan.choice == SchemaChoice::SchemaTab;
                    let what = match plan.schema_file {
                        Some(SchemaFile::Xsd { .. }) => "an XSD",
                        _ => "a JSON Schema",
                    };
                    if ui
                        .radio(
                            chosen,
                            format!("The file is {what}: import it as a schema tab"),
                        )
                        .clicked()
                    {
                        plan.choice = SchemaChoice::SchemaTab;
                    }
                    ui.label("Or as data, with the schema");
                } else {
                    ui.label("Schema");
                }
                for candidate in &plan.candidates {
                    let text = if candidate.problems == 0 {
                        format!("{}: the data fits it", candidate.schema)
                    } else {
                        format!(
                            "{}: {} would not fit",
                            candidate.schema,
                            plural(candidate.problems, "value", "values")
                        )
                    };
                    let chosen = plan.choice == SchemaChoice::Existing(candidate.schema.clone());
                    if ui.radio(chosen, text).clicked() {
                        plan.choice = SchemaChoice::Existing(candidate.schema.clone());
                    }
                }
                match &plan.inferred {
                    Ok(_) => {
                        ui.horizontal(|ui| {
                            let chosen = plan.choice == SchemaChoice::Inferred;
                            if ui
                                .radio(chosen, "A new schema, inferred from the data, called")
                                .clicked()
                            {
                                plan.choice = SchemaChoice::Inferred;
                            }
                            let response = ui.add_enabled(
                                chosen,
                                egui::TextEdit::singleline(&mut plan.schema_name)
                                    .desired_width(160.0),
                            );
                            named(&response, "Schema tab name");
                        });
                    }
                    Err(why) => {
                        ui.add_enabled(
                            false,
                            egui::RadioButton::new(
                                false,
                                format!("No schema can be inferred: {why}"),
                            ),
                        );
                    }
                }
                if let Some(tab) = plan.replace.clone() {
                    let chosen = plan.choice == SchemaChoice::Replace(tab.clone());
                    if ui
                        .radio(
                            chosen,
                            format!("No new tab: replace the data of {tab} with it"),
                        )
                        .clicked()
                    {
                        plan.choice = SchemaChoice::Replace(tab);
                    }
                }
                if let (SchemaChoice::Inferred, Ok(inferred)) = (&plan.choice, &plan.inferred) {
                    egui::CollapsingHeader::new("The inferred schema")
                        .id_salt("inferred_schema")
                        .show(ui, |ui| {
                            egui::ScrollArea::vertical()
                                .id_salt("inferred_schema_text")
                                .max_height(160.0)
                                .show(ui, |ui| {
                                    let mut text = inferred.text();
                                    ui.add(
                                        egui::TextEdit::multiline(&mut text)
                                            .code_editor()
                                            .interactive(false)
                                            .desired_width(f32::INFINITY),
                                    );
                                });
                        });
                }
                let report = plan.report();
                if !report.is_empty() {
                    ui.add_space(4.0);
                    ui.label("The import report");
                    egui::ScrollArea::vertical()
                        .id_salt("import_report")
                        .max_height(120.0)
                        .show(ui, |ui| {
                            for line in &report {
                                ui.label(format!("\u{2022} {line}"));
                            }
                        });
                }
                if let Some(why) = problem.as_ref().or(plan.error.as_ref()) {
                    ui.colored_label(error_color(ui), why);
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    apply = ui
                        .add_enabled(problem.is_none(), egui::Button::new("Import"))
                        .clicked();
                    cancel = ui.button("Cancel").clicked();
                });
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    cancel = true;
                }
            });
        if apply && problem.is_none() {
            if let Err(why) = self.import(&plan) {
                plan.error = Some(why);
                self.imports.push_front(plan);
            }
        } else if open && !cancel {
            self.imports.push_front(plan);
        }
    }

    /// While files are dragged over the window: what dropping them does.
    pub(crate) fn drop_hint(&mut self, ctx: &egui::Context) {
        let (hovered, dropped) = ctx.input(|i| {
            let dropped: Vec<std::path::PathBuf> = i
                .raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect();
            (i.raw.hovered_files.len(), dropped)
        });
        if !dropped.is_empty() {
            self.drop_files(dropped);
        }
        if hovered == 0 {
            return;
        }
        let palette = ctx.style_of(ctx.theme()).visuals.clone();
        let palette = crate::style::Palette::of_visuals(&palette);
        let rect = ctx.content_rect();
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::unique("drop_hint"),
        ));
        painter.rect_filled(rect, 0.0, palette.panel.gamma_multiply(0.85));
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "Drop to import as a new tab (a .clio file opens)",
            egui::FontId::proportional(18.0),
            palette.text,
        );
    }
}

fn too_large(file: &str, limit: u64) -> String {
    format!(
        "{} is larger than the limit for a file of {} MB, so it is not imported.",
        capital(file),
        limit >> 20
    )
}

/// Whether JSON is a JSON Schema document: an object whose `$schema` names
/// the JSON Schema meta-schema, or a file called `*.schema.json`.
fn is_json_schema(file: &str, node: &Node) -> bool {
    let named = file.to_lowercase().ends_with(".schema.json");
    let declared = match node.value() {
        cliotree_core::Value::Object(fields) => fields.get("$schema").is_some_and(
            |s| matches!(s.value(), cliotree_core::Value::Str(s) if s.contains("json-schema.org")),
        ),
        _ => false,
    };
    named || declared
}

/// A file name as it starts a sentence: as it is, but "The pasted text".
fn capital(file: &str) -> String {
    if file == PASTED {
        "The pasted text".to_owned()
    } else {
        file.to_owned()
    }
}

/// The imports waiting for their dialog.
pub type Imports = VecDeque<ImportPlan>;
