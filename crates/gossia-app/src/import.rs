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

use gossia_i18n::tr;
use std::collections::VecDeque;
use std::path::Path as FsPath;

use gossia_calc::SchemaFormat;
use gossia_calc::names::{self, Subject};
use gossia_core::binary::BinaryText;
use gossia_core::calendar::{self, EntryKind};
use gossia_core::copybook::{self, CodePage};
use gossia_core::dataformat::DataFormat;
use gossia_core::infer::{self, JsonInference};
use gossia_core::validate::validate;
use gossia_core::xml::XmlDocument;
use gossia_core::{Node, Value, delimited, json, metaschema, xml, xmltree, xsd};

use crate::app::GossiaApp;
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
    /// A COBOL copybook (`.cpy`), which becomes a copybook schema tab.
    Copybook,
    /// COBOL records (`.blk`), read with a copybook schema tab already in
    /// the workbook.
    Blk,
    /// A workbook of Excel or LibreOffice (`.xlsx`, `.xlsm`, `.xlsb`, `.xls`,
    /// `.ods`): each worksheet chosen is read as CSV is, into a JSON tab of
    /// its own (DESIGN.md, "Excel and ODS files, 0.4.0").
    Workbook,
    /// An iCalendar file (`.ics`): its events and its tasks, each a list of
    /// entries for a tab of their own or one there already (DESIGN.md,
    /// "Calendars and contacts, 0.4.0").
    Calendar,
    /// vCard contacts (`.vcf`), likewise.
    Contacts,
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
            ImportFormat::Csv => tr!("comma-separated values"),
            ImportFormat::Tsv => tr!("tab-separated values"),
            ImportFormat::Copybook => "a COBOL copybook",
            ImportFormat::Blk => "COBOL records",
            ImportFormat::Text => tr!("lines of text"),
            ImportFormat::Workbook => tr!("a spreadsheet workbook"),
            ImportFormat::Calendar => tr!("an iCalendar file"),
            ImportFormat::Contacts => tr!("vCard contacts"),
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
            ImportFormat::Copybook => DataFormat::Json5,
            ImportFormat::Blk => DataFormat::Blk,
            ImportFormat::Json
            | ImportFormat::Csv
            | ImportFormat::Tsv
            | ImportFormat::Text
            | ImportFormat::Workbook
            | ImportFormat::Calendar
            | ImportFormat::Contacts => DataFormat::Json,
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
    /// A COBOL copybook, as the copybook meta-schema types it.
    Copybook(Node),
}

/// A schema tab the data could be given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub schema: String,
    /// For COBOL records, the 01 record of the copybook they are read as; for
    /// XML, the global element of an XSD tab that has several, when it is
    /// not the first.
    pub record: Option<String>,
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
    /// No new tab: calendar entries or contacts are added to this tab of
    /// their kind, an entry with the key of one there replacing it.
    AddTo(String),
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
    /// The bytes of a `.blk` file, read again with the copybook chosen.
    bytes: Vec<u8>,
    /// For COBOL records, the 01 record chosen with the schema tab; for XML,
    /// the global element chosen, when it is not the first.
    pub record: Option<String>,
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
    /// For a workbook, its worksheets, each planned as a file of its own;
    /// the plan itself is then only their frame. Empty for any other file.
    pub sheets: Vec<SheetPlan>,
    /// The worksheet whose settings the dialog shows.
    pub sheet: usize,
    /// For calendar entries and contacts, their kind, and the tabs of that
    /// kind they can be added to.
    pub entries: Option<EntryKind>,
    pub add_to: Vec<String>,
}

/// One worksheet of a workbook being imported.
#[derive(Debug, Clone)]
pub struct SheetPlan {
    /// Its name in the workbook.
    pub title: String,
    /// Whether it is imported. An empty sheet is not, and cannot be.
    pub ticked: bool,
    pub rows: usize,
    pub plan: ImportPlan,
}

impl ImportPlan {
    /// The tab the import ends on: the data tab, or the tab added to.
    fn target(&self) -> String {
        match &self.choice {
            SchemaChoice::AddTo(tab) | SchemaChoice::Replace(tab) => tab.clone(),
            _ => self.name.trim().to_owned(),
        }
    }

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
                if let Some(c) = self
                    .candidates
                    .iter()
                    .find(|c| &c.schema == schema && c.record == self.record)
                    && c.problems > 0
                {
                    lines.push(if c.problems == 1 {
                        tr!(
                            "1 value in the data does not fit {schema}; it is kept and flagged.",
                            schema = schema
                        )
                    } else {
                        tr!(
                            "{n} values in the data do not fit {schema}; they are kept and flagged.",
                            n = c.problems,
                            schema = schema
                        )
                    });
                }
            }
            SchemaChoice::Replace(_) | SchemaChoice::AddTo(_) => {}
            SchemaChoice::SchemaTab => {
                if let Some(why) = &self.schema_problem {
                    lines.push(tr!("It cannot be used as a schema as it is: {why}. It can be corrected on its tab.", why = why));
                }
            }
        }
        lines
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
        Some("cpy") => ImportFormat::Copybook,
        Some("blk") => ImportFormat::Blk,
        Some("xlsx" | "xlsm" | "xlsb" | "xls" | "ods") => ImportFormat::Workbook,
        Some("ics" | "ical" | "ifb") => ImportFormat::Calendar,
        Some("vcf" | "vcard") => ImportFormat::Contacts,
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
    let begins = |what: &str| {
        first_line
            .trim_end()
            .get(..what.len())
            .is_some_and(|s| s.eq_ignore_ascii_case(what))
    };
    if begins("BEGIN:VCALENDAR") {
        ImportFormat::Calendar
    } else if begins("BEGIN:VCARD") {
        ImportFormat::Contacts
    } else if is_json_lines(text) {
        ImportFormat::JsonLines
    } else if is_toml(text) {
        ImportFormat::Toml
    } else if text.starts_with(['{', '[']) {
        ImportFormat::Json
    } else if text.starts_with('<') {
        ImportFormat::Xml
    } else if is_copybook(text) {
        ImportFormat::Copybook
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

/// Whether the text is a COBOL copybook: entries that start with level
/// numbers and end with periods, at least one of them.
fn is_copybook(text: &str) -> bool {
    copybook::parse(text).is_ok_and(|book| !book.items.is_empty())
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
        && gossia_core::yaml::parse(text)
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

impl GossiaApp {
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
        let bytes = match copied_in(path, &file, &bytes) {
            Some(expanded) => expanded.into_bytes(),
            None => bytes,
        };
        let plan = self.plan_import(&file, &bytes)?;
        self.imports.push_back(plan);
        Ok(())
    }

    /// Opens the import dialog for a file read through the host. A copybook's
    /// `COPY` members cannot be looked for beside it, since there is no
    /// folder.
    pub fn begin_import_bytes(&mut self, file: &str, bytes: &[u8]) -> Result<(), String> {
        let limit = self.user.prefs.max_import_bytes;
        if bytes.len() as u64 > limit {
            return Err(too_large(file, limit));
        }
        let plan = self.plan_import(file, bytes)?;
        self.imports.push_back(plan);
        Ok(())
    }

    /// Takes the files that have come through the host (DESIGN.md, Web port,
    /// 0.3.0): picked in its dialog for a command, or dropped on the window.
    pub(crate) fn receive_picked(&mut self) {
        use crate::host::purpose;
        for file in crate::host::take_picked() {
            let workbook = file
                .name
                .rsplit_once('.')
                .is_some_and(|(_, e)| e.eq_ignore_ascii_case(crate::workbook_file::EXTENSION));
            match file.purpose.as_str() {
                purpose::IMPORT_MODULE => {
                    if let Err(e) = self.import_module_bytes(&file.name, &file.bytes) {
                        self.code.notice = Some(e);
                    }
                }
                purpose::DROP if workbook => {
                    self.notice = Some(tr!(
                        "{name} is a workbook: open it from outside Gossia, as other files are \
                         opened here.",
                        name = file.name
                    ));
                }
                _ => {
                    if let Err(e) = self.begin_import_bytes(&file.name, &file.bytes) {
                        self.notice = Some(e);
                    }
                }
            }
        }
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
        if let Some(plan) = self.plan_records_paste(text) {
            return plan;
        }
        let format = format_of_text(text);
        let mut plan = self.plan_import_as(PASTED, "Pasted", format, text.as_bytes())?;
        // Copybook text pasted on a copybook schema tab can be its text.
        if format == ImportFormat::Copybook {
            if self.calc().schema_format(&self.sheet) == Some(SchemaFormat::Copybook) {
                plan.replace = Some(self.sheet.clone());
            }
            return Ok(plan);
        }
        let same_format = match format {
            ImportFormat::Csv
            | ImportFormat::Tsv
            | ImportFormat::Text
            | ImportFormat::Calendar
            | ImportFormat::Contacts => false,
            _ => self.calc().format(&self.sheet) == Some(format.data_format()),
        };
        if same_format && !self.calc().is_schema_sheet(&self.sheet) {
            plan.replace = Some(self.sheet.clone());
        }
        Ok(plan)
    }

    /// A paste of COBOL records as text: on a tab of records, a dump, hex
    /// digits or Base64, whichever Options says to copy; anywhere, a dump. None when the text is none of these, and it is read as other
    /// text is. On a tab whose records it fits, the dialog also offers to
    /// replace them.
    fn plan_records_paste(&self, text: &str) -> Option<Result<ImportPlan, String>> {
        let calc = self.calc();
        let layout = calc.spec(&self.sheet).and_then(|s| s.copybook());
        let (kind, bytes) = match layout {
            Some(_) => BinaryText::read_any(text)?,
            None => (BinaryText::Dump, BinaryText::Dump.read(text)?),
        };
        let fits = layout.is_some_and(|l| bytes.len() == l.length);
        let mut plan = match self.plan_records(PASTED, "Pasted", &bytes) {
            Ok(plan) => plan,
            Err(why) if !fits => return Some(Err(why)),
            Err(_) => ImportPlan {
                file: PASTED.to_owned(),
                format: ImportFormat::Blk,
                text: String::new(),
                bytes: bytes.clone(),
                record: None,
                data: None,
                notes: Vec::new(),
                inferred: Err(
                    tr!("a .blk file does not say how its records are laid out").to_owned()
                ),
                candidates: Vec::new(),
                name: name_from_file("Pasted", |n| calc.tab_name_problem(n, None).is_some()),
                schema_name: String::new(),
                choice: SchemaChoice::Replace(self.sheet.clone()),
                replace: None,
                schema_file: None,
                schema_problem: None,
                error: None,
                sheets: Vec::new(),
                sheet: 0,
                entries: None,
                add_to: Vec::new(),
            },
        };
        plan.notes.insert(
            0,
            tr!(
                "Read as {count} bytes of {kind}.",
                count = bytes.len(),
                kind = kind
                    .label()
                    .split(',')
                    .next()
                    .unwrap_or_default()
                    .to_lowercase()
            ),
        );
        if fits {
            plan.replace = Some(self.sheet.clone());
        }
        Some(Ok(plan))
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
        if format == ImportFormat::Blk {
            return self.plan_records(file, name_from, bytes);
        }
        if format == ImportFormat::Workbook {
            return self.plan_workbook(file, bytes);
        }
        let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
        let text = String::from_utf8(bytes.to_vec()).map_err(|_| {
            format!(
                "{} is not UTF-8 text, so it cannot be imported.",
                capital(file)
            )
        })?;
        let format = format.refine(&text);
        if let Some(kinds) = entry_kinds(format) {
            return self.plan_entries(file, name_from, format, kinds, &text);
        }
        let calc = self.calc();
        let mut notes = Vec::new();
        let mut schema_file = None;
        let mut schema_problem = None;
        let (data, inferred, candidates) = match format {
            ImportFormat::Copybook => {
                let book = copybook::parse(&text)
                    .map_err(|e| format!("{} is not a COBOL copybook: {e}", capital(file)))?;
                schema_problem = copybook::lower(&book, CodePage::default())
                    .err()
                    .map(|e| e.to_string());
                schema_file = Some(SchemaFile::Copybook(copybook::to_node(
                    &book,
                    CodePage::default(),
                )));
                (None, Err(tr!("it is a copybook").to_owned()), Vec::new())
            }
            ImportFormat::Blk
            | ImportFormat::Workbook
            | ImportFormat::Calendar
            | ImportFormat::Contacts => {
                unreachable!("COBOL records and workbooks are planned above")
            }
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
                        schema_problem = gossia_core::jsonschema::lower(&parsed.node)
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
                let candidates = self.xsd_tabs(|spec| {
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
        let mut record = None;
        let choice = match candidates.iter().find(|c| c.problems == 0) {
            _ if schema_file.is_some() => SchemaChoice::SchemaTab,
            Some(fits) => {
                record = fits.record.clone();
                SchemaChoice::Existing(fits.schema.clone())
            }
            None if inferred.is_ok() => SchemaChoice::Inferred,
            None => match candidates.first() {
                Some(c) => {
                    record = c.record.clone();
                    SchemaChoice::Existing(c.schema.clone())
                }
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
            bytes: Vec::new(),
            record,
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
            sheets: Vec::new(),
            sheet: 0,
            entries: None,
            add_to: Vec::new(),
        })
    }

    /// Reads a `.blk` file for importing: with each copybook schema tab in the
    /// workbook, and each of its records, whose length is the file's: a
    /// `.blk` file holds one record.
    /// A `.blk` file says nothing of its layout, so no schema is inferred.
    fn plan_records(
        &self,
        file: &str,
        name_from: &str,
        bytes: &[u8],
    ) -> Result<ImportPlan, String> {
        let calc = self.calc();
        let mut candidates = Vec::new();
        for schema in calc.sheet_names() {
            if calc.schema_format(schema) != Some(SchemaFormat::Copybook)
                || (calc.auto_update(schema) && !calc.schema_users(schema).is_empty())
            {
                continue;
            }
            let records = calc.copybook_records(schema);
            let several = records.len() > 1;
            for record in records {
                let Ok(spec) = calc.record_spec(schema, &record) else {
                    continue;
                };
                let Some(layout) = spec.copybook() else {
                    continue;
                };
                if let Ok(parsed) = copybook::records::read(bytes, layout, &[]) {
                    candidates.push(Candidate {
                        schema: schema.to_owned(),
                        record: several.then(|| record.clone()),
                        problems: count_errors(&parsed.node),
                    });
                }
            }
        }
        let taken = |n: &str| calc.tab_name_problem(n, None).is_some();
        let name = name_from_file(name_from, taken);
        let schema_name = unique(&format!("{name}Schema"), |n| {
            taken(n) || n.eq_ignore_ascii_case(&name)
        });
        let chosen = candidates
            .iter()
            .find(|c| c.problems == 0)
            .or(candidates.first())
            .cloned()
            .ok_or_else(|| {
                tr!(
                    "{file} cannot be imported: a .blk file does not say how its record is \
                     laid out, and no copybook schema tab in the workbook has a record of its \
                     {count} bytes. Import its copybook (.cpy) first.",
                    file = file,
                    count = bytes.len()
                )
            })?;
        Ok(ImportPlan {
            file: file.to_owned(),
            format: ImportFormat::Blk,
            text: String::new(),
            bytes: bytes.to_vec(),
            record: chosen.record.clone(),
            data: None,
            notes: Vec::new(),
            inferred: Err(tr!("a .blk file does not say how its records are laid out").to_owned()),
            candidates,
            name,
            schema_name,
            choice: SchemaChoice::Existing(chosen.schema),
            replace: None,
            schema_file: None,
            schema_problem: None,
            error: None,
            sheets: Vec::new(),
            sheet: 0,
            entries: None,
            add_to: Vec::new(),
        })
    }

    /// Reads a workbook for importing: each worksheet becomes the plan of a
    /// tab, as a CSV file would, named `<File>_<Sheet>`, and all but the
    /// empty ones are ticked. The names are kept apart from each other as
    /// well as from the workbook's tabs.
    fn plan_workbook(&self, file: &str, bytes: &[u8]) -> Result<ImportPlan, String> {
        let read = gossia_core::spreadsheet::read(bytes).map_err(|e| {
            tr!(
                "{file} cannot be read as a spreadsheet workbook: {e}",
                file = capital(file),
                e = e
            )
        })?;
        if read.is_empty() {
            return Err(tr!("{file} has no worksheets.", file = capital(file)));
        }
        let calc = self.calc();
        let stem = FsPath::new(file)
            .file_stem()
            .map_or_else(|| file.to_owned(), |s| s.to_string_lossy().into_owned());
        let mut used: Vec<String> = Vec::new();
        let mut sheets = Vec::new();
        for sheet in read {
            let taken = |n: &str| {
                calc.tab_name_problem(n, None).is_some()
                    || used.iter().any(|u| u.eq_ignore_ascii_case(n))
            };
            // Read as a file name: a slash would part it.
            let title = sheet.name.replace(['/', '\\'], "_");
            let name = name_from_file(&format!("{stem}_{title}.xlsx"), taken);
            let schema_name = unique(&format!("{name}Schema"), |n| {
                taken(n) || n.eq_ignore_ascii_case(&name)
            });
            used.push(name.clone());
            used.push(schema_name.clone());
            let inference = sheet.inference;
            let data = inference.data.clone();
            let candidates =
                self.schema_tabs(SchemaFormat::Json, |spec| Some(validate(spec, &data).len()));
            let choice = match candidates.iter().find(|c| c.problems == 0) {
                Some(fits) => SchemaChoice::Existing(fits.schema.clone()),
                None => SchemaChoice::Inferred,
            };
            let plan = ImportPlan {
                file: file.to_owned(),
                format: ImportFormat::Workbook,
                text: String::new(),
                bytes: Vec::new(),
                record: None,
                data: Some(data),
                notes: inference.dropped.clone(),
                inferred: Ok(Inferred::Json(inference)),
                candidates,
                name,
                schema_name,
                choice,
                replace: None,
                schema_file: None,
                schema_problem: None,
                error: None,
                sheets: Vec::new(),
                sheet: 0,
                entries: None,
                add_to: Vec::new(),
            };
            sheets.push(SheetPlan {
                title: sheet.name,
                ticked: sheet.rows > 0,
                rows: sheet.rows,
                plan,
            });
        }
        let first = sheets.iter().position(|s| s.ticked).unwrap_or(0);
        let mut frame = sheets[first].plan.clone();
        frame.sheets = sheets;
        frame.sheet = first;
        Ok(frame)
    }

    /// Plans calendar entries or contacts: a part for each kind the file
    /// can hold (events and tasks for a calendar), each ticked when it has
    /// entries. A part goes into the first tab of its kind when there is
    /// one, and otherwise into a new tab with the kind's schema, or a
    /// schema tab of that kind there already.
    fn plan_entries(
        &self,
        file: &str,
        name_from: &str,
        format: ImportFormat,
        kinds: &[EntryKind],
        text: &str,
    ) -> Result<ImportPlan, String> {
        let (lists, notes) = if format == ImportFormat::Calendar {
            let read = calendar::read(text, &gossia_core::timezone::system()).map_err(|e| {
                tr!(
                    "{file} cannot be read as a calendar: {e}",
                    file = capital(file),
                    e = e
                )
            })?;
            (vec![read.events, read.tasks], read.notes)
        } else {
            let read = gossia_core::vcard::read(text).map_err(|e| {
                tr!(
                    "{file} cannot be read as contacts: {e}",
                    file = capital(file),
                    e = e
                )
            })?;
            (vec![read.contacts], read.notes)
        };
        if lists.iter().all(Vec::is_empty) {
            return Err(if format == ImportFormat::Calendar {
                tr!("{file} has no events or tasks.", file = capital(file))
            } else {
                tr!("{file} has no contacts.", file = capital(file))
            });
        }
        let calc = self.calc();
        let stem = FsPath::new(name_from).file_stem().map_or_else(
            || name_from.to_owned(),
            |s| s.to_string_lossy().into_owned(),
        );
        let mut used: Vec<String> = Vec::new();
        let mut sheets = Vec::new();
        for (&kind, entries) in kinds.iter().zip(lists) {
            let taken = |n: &str| {
                calc.tab_name_problem(n, None).is_some()
                    || used.iter().any(|u| u.eq_ignore_ascii_case(n))
            };
            let part = part_title(kind);
            let name = if kinds.len() > 1 {
                name_from_file(&format!("{stem}_{part}.ics"), taken)
            } else {
                name_from_file(&format!("{stem}.vcf"), taken)
            };
            let schema_name = unique(&format!("{name}Schema"), |n| {
                taken(n) || n.eq_ignore_ascii_case(&name)
            });
            used.push(name.clone());
            used.push(schema_name.clone());
            let data = calendar::data(&entries);
            let candidates =
                self.schema_tabs(SchemaFormat::Json, |spec| Some(validate(spec, &data).len()));
            let add_to = calc.tabs_of_kind(kind);
            let choice = match add_to.first() {
                Some(tab) => SchemaChoice::AddTo(tab.clone()),
                None => match candidates
                    .iter()
                    .find(|c| c.problems == 0 && schema_kind(calc, &c.schema) == Some(kind))
                {
                    Some(fits) => SchemaChoice::Existing(fits.schema.clone()),
                    None => SchemaChoice::Inferred,
                },
            };
            let plan = ImportPlan {
                file: file.to_owned(),
                format,
                text: String::new(),
                bytes: Vec::new(),
                record: None,
                data: Some(data.clone()),
                notes: notes.clone(),
                inferred: Ok(Inferred::Json(JsonInference {
                    schema: kind.schema(),
                    data,
                    dropped: Vec::new(),
                    warnings: Vec::new(),
                })),
                candidates,
                name,
                schema_name,
                choice,
                replace: None,
                schema_file: None,
                schema_problem: None,
                error: None,
                sheets: Vec::new(),
                sheet: 0,
                entries: Some(kind),
                add_to,
            };
            sheets.push(SheetPlan {
                title: part.to_owned(),
                ticked: !entries.is_empty(),
                rows: entries.len(),
                plan,
            });
        }
        if sheets.len() == 1 {
            return Ok(sheets.remove(0).plan);
        }
        let first = sheets.iter().position(|s| s.ticked).unwrap_or(0);
        let mut frame = sheets[first].plan.clone();
        frame.sheets = sheets;
        frame.sheet = first;
        Ok(frame)
    }

    /// The schema tabs of a format that can be used, each with what `check`
    /// says of the data under it; those it refuses are left out.
    fn schema_tabs(
        &self,
        format: SchemaFormat,
        check: impl Fn(&gossia_core::TypeSpec) -> Option<usize>,
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
                    record: None,
                    problems: check(&spec)?,
                })
            })
            .collect()
    }

    /// The XSD tabs that could type an XML document, as [`Self::schema_tabs`]
    /// does, with one candidate for each global element of a tab that has
    /// several (the root its documents may have) that `check` can read with.
    fn xsd_tabs(&self, check: impl Fn(&gossia_core::TypeSpec) -> Option<usize>) -> Vec<Candidate> {
        let calc = self.calc();
        let mut out = Vec::new();
        for s in calc.sheet_names() {
            if calc.schema_format(s) != Some(SchemaFormat::Xsd)
                || (calc.auto_update(s) && !calc.schema_users(s).is_empty())
            {
                continue;
            }
            let roots = calc.xsd_roots(s);
            if roots.len() < 2 {
                if let Ok(spec) = calc.schema_spec(s)
                    && let Some(problems) = check(&spec)
                {
                    out.push(Candidate {
                        schema: s.to_owned(),
                        record: None,
                        problems,
                    });
                }
                continue;
            }
            for (i, root) in roots.iter().enumerate() {
                if let Ok(spec) = calc.record_spec(s, root)
                    && let Some(problems) = check(&spec)
                {
                    out.push(Candidate {
                        schema: s.to_owned(),
                        record: (i > 0).then(|| root.clone()),
                        problems,
                    });
                }
            }
        }
        out
    }

    /// Why the plan cannot be imported as it is, or None.
    pub fn import_problem(&self, plan: &ImportPlan) -> Option<String> {
        if !plan.sheets.is_empty() {
            return self.workbook_problem(plan);
        }
        match &plan.choice {
            SchemaChoice::Replace(_) => return None,
            SchemaChoice::AddTo(tab) => {
                return (self.calc().entry_kind(tab) != plan.entries)
                    .then(|| tr!("{tab} no longer holds entries of this kind.", tab = tab));
            }
            _ => {}
        }
        let name = plan.name.trim();
        if let Some(why) = self.calc().tab_name_problem(name, None) {
            return Some(tr!("The tab name: {why}.", why = why));
        }
        if plan.choice == SchemaChoice::Inferred {
            if let Err(why) = &plan.inferred {
                return Some(tr!("No schema can be inferred: {why}.", why = why));
            }
            let schema = plan.schema_name.trim();
            if let Some(why) = self.calc().tab_name_problem(schema, None) {
                return Some(tr!("The schema tab's name: {why}.", why = why));
            }
            if schema.eq_ignore_ascii_case(name) {
                return Some(tr!("The two tabs need different names.").to_owned());
            }
        }
        None
    }

    /// Why the ticked worksheets cannot be imported as they are: none is
    /// ticked, one's own settings, or two new tabs with one name.
    fn workbook_problem(&self, plan: &ImportPlan) -> Option<String> {
        let ticked: Vec<&SheetPlan> = plan.sheets.iter().filter(|s| s.ticked).collect();
        if ticked.is_empty() {
            return Some(tr!("Tick at least one sheet to import.").to_owned());
        }
        let mut names: Vec<String> = Vec::new();
        for sheet in ticked {
            if let Some(why) = self.import_problem(&sheet.plan) {
                return Some(tr!("Sheet {sheet}: {why}", sheet = sheet.title, why = why));
            }
            let mut new = Vec::new();
            if !matches!(sheet.plan.choice, SchemaChoice::AddTo(_)) {
                new.push(sheet.plan.name.trim().to_owned());
            }
            if sheet.plan.choice == SchemaChoice::Inferred {
                new.push(sheet.plan.schema_name.trim().to_owned());
            }
            for name in new {
                if names.iter().any(|n| n.eq_ignore_ascii_case(&name)) {
                    return Some(tr!(
                        "Two of the new tabs would be called {name}.",
                        name = name
                    ));
                }
                names.push(name);
            }
        }
        None
    }

    /// Adds the tabs of the ticked worksheets of a workbook, in one entry in
    /// the history, *Import book.xlsx*. The first becomes the current tab.
    fn import_sheets(&mut self, plan: &ImportPlan) -> Result<(), String> {
        let before = self.calc().history_len().0;
        let calc = self.session.calc_mut();
        calc.begin_entry(&tr!("Import {file}", file = plan.file));
        let mut done = Ok(());
        for sheet in plan.sheets.iter().filter(|s| s.ticked) {
            done = create_tabs(calc, &sheet.plan);
            if done.is_err() {
                break;
            }
        }
        calc.end_entry();
        if let Err(e) = done {
            if self.calc().history_len().0 > before {
                let _ = self.session.calc_mut().undo();
                let _ = self.session.calc_mut().take_removed();
            }
            return Err(e.to_string());
        }
        self.follow_tabs();
        if let Some(first) = plan.sheets.iter().find(|s| s.ticked) {
            self.show_tab(&first.plan.target());
        }
        self.notice = None;
        self.session.tick();
        Ok(())
    }

    /// Adds the tabs of a plan: one entry in the history, *Import file.json*.
    /// An inferred schema has auto-update off, so that the next file of the
    /// same shape can be given it too. The data tab becomes the current tab.
    pub fn import(&mut self, plan: &ImportPlan) -> Result<(), String> {
        if self.session.running_procedure().is_some() {
            return Err(tr!("a button procedure is running; wait for it to finish").to_owned());
        }
        if !self.commit_editing() {
            return Err(self.notice.clone().unwrap_or_default());
        }
        if let Some(why) = self.import_problem(plan) {
            return Err(why);
        }
        if !plan.sheets.is_empty() {
            return self.import_sheets(plan);
        }
        if let SchemaChoice::Replace(tab) = &plan.choice {
            let tab = tab.clone();
            self.show_tab(&tab);
            if plan.format == ImportFormat::Blk {
                let result = self.replace_records(&tab, &plan.bytes);
                self.raw.error = result.as_ref().err().cloned();
                return result;
            }
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
                Some(SchemaFile::Copybook(tree)) => {
                    calc.create_copybook_schema_tab(name, tree.clone())
                }
                None => return Err(tr!("the file is not a schema").to_owned()),
            };
            done.map_err(|e| e.to_string())?;
            self.follow_tabs();
            self.show_tab(name);
            self.notice = None;
            self.session.tick();
            return Ok(());
        }
        let name = plan.target();
        let before = self.calc().history_len().0;
        let calc = self.session.calc_mut();
        calc.begin_entry(&tr!("Import {file}", file = plan.file));
        let done = create_tabs(calc, plan);
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
        self.show_tab(&name);
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
        egui::Window::new(tr!("Import {file}", file = plan.file))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .min_width(460.0)
            .max_width(560.0)
            .show(ctx, |ui| {
                if plan.file == PASTED {
                    ui.weak(tr!("Read as {format}.", format = plan.format.label()));
                }
                // A workbook: its sheets, each ticked to be imported, and the
                // settings of the one picked below.
                if !plan.sheets.is_empty() {
                    sheet_list(ui, &mut plan);
                    ui.separator();
                }
                let shown: &mut ImportPlan = if plan.sheets.is_empty() {
                    &mut plan
                } else {
                    let at = plan.sheet;
                    &mut plan.sheets[at].plan
                };
                let replacing =
                    matches!(shown.choice, SchemaChoice::Replace(_) | SchemaChoice::AddTo(_));
                ui.horizontal(|ui| {
                    ui.label(tr!("Tab name"));
                    let response =
                        ui.add_enabled(!replacing, egui::TextEdit::singleline(&mut shown.name));
                    named(&response, tr!("Import tab name"));
                });
                ui.add_space(4.0);
                // A copybook is a schema and nothing else.
                if matches!(shown.schema_file, Some(SchemaFile::Copybook(_))) {
                    if shown.replace.is_some() {
                        let chosen = shown.choice == SchemaChoice::SchemaTab;
                        if ui
                            .radio(
                                chosen,
                                tr!("It is a COBOL copybook: import it as a schema tab"),
                            )
                            .clicked()
                        {
                            shown.choice = SchemaChoice::SchemaTab;
                        }
                    } else {
                        ui.label(tr!(
                            "The file is a COBOL copybook: it is imported as a schema tab."
                        ));
                    }
                } else if shown.schema_file.is_some() {
                    let chosen = shown.choice == SchemaChoice::SchemaTab;
                    let what = match shown.schema_file {
                        Some(SchemaFile::Xsd { .. }) => "an XSD",
                        _ => "a JSON Schema",
                    };
                    if ui
                        .radio(
                            chosen,
                            tr!("The file is {what}: import it as a schema tab", what = what),
                        )
                        .clicked()
                    {
                        shown.choice = SchemaChoice::SchemaTab;
                    }
                    ui.label(tr!("Or as data, with the schema"));
                } else if let (Some(kind), false) = (shown.entries, shown.add_to.is_empty()) {
                    for tab in shown.add_to.clone() {
                        let chosen = shown.choice == SchemaChoice::AddTo(tab.clone());
                        let text = if kind == EntryKind::Contacts {
                            tr!(
                                "Add to the tab {tab}: a contact with the same UID, or without one the same name, replaces the one there",
                                tab = tab
                            )
                        } else {
                            tr!(
                                "Add to the tab {tab}: an entry with the same UID replaces the one there",
                                tab = tab
                            )
                        };
                        if ui.radio(chosen, text).clicked() {
                            shown.choice = SchemaChoice::AddTo(tab);
                        }
                    }
                    ui.label(tr!("Or as a new tab, with the schema"));
                } else {
                    ui.label(tr!("Schema"));
                }
                for candidate in &shown.candidates {
                    let schema = match &candidate.record {
                        Some(record) => format!("{} ({record})", candidate.schema),
                        None => candidate.schema.clone(),
                    };
                    let text = match candidate.problems {
                        0 => tr!("{schema}: the data fits it", schema = schema),
                        1 => tr!("{schema}: 1 value would not fit", schema = schema),
                        n => tr!("{schema}: {n} values would not fit", schema = schema, n = n),
                    };
                    let chosen = shown.choice == SchemaChoice::Existing(candidate.schema.clone())
                        && shown.record == candidate.record;
                    if ui.radio(chosen, text).clicked() {
                        shown.choice = SchemaChoice::Existing(candidate.schema.clone());
                        shown.record = candidate.record.clone();
                    }
                }
                let copybook = matches!(shown.schema_file, Some(SchemaFile::Copybook(_)));
                match &shown.inferred {
                    _ if copybook => {}
                    Ok(_) => {
                        let text = match shown.entries {
                            Some(EntryKind::Events) => tr!("A new schema for calendar events, called"),
                            Some(EntryKind::Tasks) => tr!("A new schema for calendar tasks, called"),
                            Some(EntryKind::Contacts) => tr!("A new schema for contacts, called"),
                            None => tr!("A new schema, inferred from the data, called"),
                        };
                        ui.horizontal(|ui| {
                            let chosen = shown.choice == SchemaChoice::Inferred;
                            if ui.radio(chosen, text).clicked()
                            {
                                shown.choice = SchemaChoice::Inferred;
                            }
                            let response = ui.add_enabled(
                                chosen,
                                egui::TextEdit::singleline(&mut shown.schema_name)
                                    .desired_width(160.0),
                            );
                            named(&response, tr!("Schema tab name"));
                        });
                    }
                    Err(why) => {
                        ui.add_enabled(
                            false,
                            egui::RadioButton::new(
                                false,
                                tr!("No schema can be inferred: {why}", why = why),
                            ),
                        );
                    }
                }
                if let Some(tab) = shown.replace.clone() {
                    let chosen = shown.choice == SchemaChoice::Replace(tab.clone());
                    if ui
                        .radio(
                            chosen,
                            tr!("No new tab: replace the data of {tab} with it", tab = tab),
                        )
                        .clicked()
                    {
                        shown.choice = SchemaChoice::Replace(tab);
                    }
                }
                if let (SchemaChoice::Inferred, Ok(inferred), None) =
                    (&shown.choice, &shown.inferred, shown.entries)
                {
                    egui::CollapsingHeader::new(tr!("The inferred schema"))
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
                let report = shown.report();
                if !report.is_empty() {
                    ui.add_space(4.0);
                    ui.label(tr!("The import report"));
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
                        .add_enabled(problem.is_none(), egui::Button::new(tr!("Import")))
                        .clicked();
                    cancel = ui.button(tr!("Cancel")).clicked();
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
        let (hovered, dropped) =
            ctx.input(|i| (i.raw.hovered_files.len(), i.raw.dropped_files.clone()));
        // A browser reads a dropped file later, and has no path for it: the
        // file comes back as one picked through the host.
        #[cfg(target_family = "wasm")]
        for file in dropped {
            let ctx = ctx.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let name = file.path().to_string_lossy().into_owned();
                match file.bytes_async().await {
                    Ok(bytes) => crate::host::picked(crate::host::Picked {
                        purpose: crate::host::purpose::DROP.to_owned(),
                        name,
                        bytes,
                    }),
                    Err(e) => web_sys_log(&format!("{name} cannot be read: {e}")),
                }
                ctx.request_repaint();
            });
        }
        #[cfg(not(target_family = "wasm"))]
        if !dropped.is_empty() {
            self.drop_files(dropped.iter().map(|f| f.path().to_path_buf()).collect());
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
            tr!("Drop to import as a new tab (a .gossia file opens)"),
            egui::FontId::proportional(18.0),
            palette.text,
        );
    }
}

/// The worksheets of a workbook in the import dialog: a tick for each that
/// has rows, and a click on its name shows its settings.
fn sheet_list(ui: &mut egui::Ui, plan: &mut ImportPlan) {
    let parts = plan.entries.is_some();
    ui.label(if parts {
        tr!("Parts to import")
    } else {
        tr!("Sheets to import")
    });
    egui::ScrollArea::vertical()
        .id_salt("import_sheets")
        .max_height(140.0)
        .show(ui, |ui| {
            for (index, sheet) in plan.sheets.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    let tick = ui.add_enabled(
                        sheet.rows > 0,
                        egui::Checkbox::without_text(&mut sheet.ticked),
                    );
                    let title = sheet.title.clone();
                    tick.widget_info(|| {
                        egui::WidgetInfo::selected(
                            egui::Role::CheckBox,
                            sheet.rows > 0,
                            sheet.ticked,
                            if parts {
                                tr!("Import part {part}", part = title)
                            } else {
                                tr!("Import sheet {sheet}", sheet = title)
                            },
                        )
                    });
                    let text = match (sheet.rows, parts) {
                        (0, _) => tr!("{sheet}: empty", sheet = sheet.title),
                        (1, false) => tr!("{sheet}: 1 row", sheet = sheet.title),
                        (n, false) => tr!("{sheet}: {n} rows", sheet = sheet.title, n = n),
                        (1, true) => tr!("{sheet}: 1 entry", sheet = sheet.title),
                        (n, true) => tr!("{sheet}: {n} entries", sheet = sheet.title, n = n),
                    };
                    if ui.selectable_label(plan.sheet == index, text).clicked() {
                        plan.sheet = index;
                    }
                });
            }
        });
}

/// The kinds of entry a file of this format holds, in the order of its
/// parts; None for formats of other data.
fn entry_kinds(format: ImportFormat) -> Option<&'static [EntryKind]> {
    match format {
        ImportFormat::Calendar => Some(&[EntryKind::Events, EntryKind::Tasks]),
        ImportFormat::Contacts => Some(&[EntryKind::Contacts]),
        _ => None,
    }
}

/// What a part of a calendar file is called in the dialog, and in its tab's
/// name.
fn part_title(kind: EntryKind) -> &'static str {
    match kind {
        EntryKind::Events => tr!("Events"),
        EntryKind::Tasks => tr!("Tasks"),
        EntryKind::Contacts => tr!("Contacts"),
    }
}

/// The kind of entry a schema tab is for, by its root's title.
fn schema_kind(calc: &gossia_calc::Calc, schema: &str) -> Option<EntryKind> {
    let title = calc
        .data(schema)?
        .value()
        .as_object()?
        .get("title")?
        .value()
        .as_str()?;
    EntryKind::from_title(title)
}

/// Makes the tabs of one plan: the schema tab, if a new one, and the data
/// tab, with its format. The caller holds the entry in the history.
fn create_tabs(
    calc: &mut gossia_calc::Calc,
    plan: &ImportPlan,
) -> Result<(), gossia_calc::EditError> {
    let name = plan.name.trim();
    let schema_name = plan.schema_name.trim();
    let done = match (&plan.choice, &plan.inferred) {
        (SchemaChoice::Inferred, Ok(Inferred::Json(inference))) => calc
            .create_json_schema_tab(schema_name, inference.schema.clone())
            .and_then(|()| calc.create_data_tab(name, schema_name, inference.data.clone(), None)),
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
        (SchemaChoice::Inferred, Err(why)) => Err(gossia_calc::EditError::Path(why.clone())),
        (SchemaChoice::AddTo(tab), _) => {
            let kind = plan
                .entries
                .ok_or_else(|| gossia_calc::EditError::Path("not calendar entries".into()))?;
            let entries = plan
                .data
                .as_ref()
                .and_then(|d| d.value().as_array())
                .map(|a| a.to_vec())
                .unwrap_or_default();
            calc.add_entries(tab, kind, &entries, &tr!("Import {file}", file = plan.file))
                .map(|_| ())
        }
        (SchemaChoice::Replace(_) | SchemaChoice::SchemaTab, _) => {
            unreachable!("replacing and schema tabs returned above")
        }
        (SchemaChoice::Existing(schema), _) if plan.format == ImportFormat::Blk => {
            let record = plan.record.as_deref();
            let spec = match record {
                Some(record) => calc.record_spec(schema, record),
                None => calc.schema_spec(schema),
            };
            spec.and_then(|spec| {
                let layout = spec.copybook().ok_or_else(|| {
                    gossia_calc::EditError::Path(format!("{schema} is not a copybook"))
                })?;
                copybook::records::read(&plan.bytes, layout, &[])
                    .map_err(|e| gossia_calc::EditError::Path(e.to_string()))
            })
            .and_then(|parsed| calc.create_records_tab(name, schema, record, parsed.node))
            .and_then(|()| {
                // A variable-length record is written back as one.
                let variable = calc
                    .spec(name)
                    .and_then(|spec| spec.copybook())
                    .is_some_and(|layout| copybook::records::has_rdw(&plan.bytes, layout));
                if variable {
                    calc.set_variable(name, true)
                } else {
                    Ok(())
                }
            })
        }
        (SchemaChoice::Existing(schema), _) => match &plan.data {
            Some(data) => calc.create_data_tab(name, schema, data.clone(), None),
            // An XSD tab with several global elements: the one chosen.
            None => match plan.record.as_deref() {
                Some(root) => calc.record_spec(schema, root),
                None => calc.schema_spec(schema),
            }
            .and_then(|spec| {
                xml::parse(&plan.text, &spec)
                    .map_err(|e| gossia_calc::EditError::Path(e.to_string()))
            })
            .and_then(|parsed| {
                calc.create_xml_tab(
                    name,
                    schema,
                    plan.record.as_deref(),
                    parsed.node,
                    parsed.document,
                )
            }),
        },
    };
    // A tab format of its own is set in the same entry.
    done.and_then(|()| match plan.format.data_format() {
        // XML and COBOL records are the formats of their schema tabs.
        DataFormat::Json | DataFormat::Xml | DataFormat::Blk => Ok(()),
        format => calc.set_format(name, format),
    })
}

/// How many error values a tree holds: the bytes of COBOL records that do
/// not read as their field.
fn count_errors(node: &Node) -> usize {
    match node.value() {
        Value::Error(_) => 1,
        Value::Object(fields) => fields.values().map(count_errors).sum(),
        Value::Array(items) => items.iter().map(count_errors).sum(),
        _ => 0,
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
        gossia_core::Value::Object(fields) => fields.get("$schema").is_some_and(
            |s| matches!(s.value(), gossia_core::Value::Str(s) if s.contains("json-schema.org")),
        ),
        _ => false,
    };
    named || declared
}

/// A file name as it starts a sentence: as it is, but "The pasted text".
fn capital(file: &str) -> String {
    if file == PASTED {
        tr!("The pasted text").to_owned()
    } else {
        file.to_owned()
    }
}

/// The imports waiting for their dialog.
pub type Imports = VecDeque<ImportPlan>;

/// The text of a copybook file with the members its `COPY` statements name
/// copied in, read from the file's folder (`NAME.cpy`, `NAME.cbl`, `NAME`,
/// as written or in lower case). None for any other file, or a copybook
/// with no `COPY` statement: it is planned as it is, and says what is wrong.
fn copied_in(path: &FsPath, file: &str, bytes: &[u8]) -> Option<String> {
    if format_of(file) != ImportFormat::Copybook {
        return None;
    }
    let text = std::str::from_utf8(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes)).ok()?;
    if copybook::parse(text).is_ok() {
        return None;
    }
    let folder = path.parent()?;
    let mut load = |name: &str| {
        let names = [name.to_owned(), name.to_ascii_lowercase()];
        names.iter().find_map(|name| {
            ["cpy", "CPY", "cbl", "CBL", "cob", "COB", ""]
                .iter()
                .map(|ext| {
                    if ext.is_empty() {
                        folder.join(name)
                    } else {
                        folder.join(format!("{name}.{ext}"))
                    }
                })
                .find(|p| p.is_file())
                .and_then(|p| std::fs::read(p).ok())
                .map(|b| String::from_utf8_lossy(&b).into_owned())
        })
    };
    let book = copybook::parse_with(text, &mut load).ok()?;
    Some(copybook::to_text(&book))
}

/// A dropped file the browser could not read: nothing in the window waits
/// for it, so the console says why.
#[cfg(target_family = "wasm")]
fn web_sys_log(text: &str) {
    web_sys::console::warn_1(&text.into());
}
