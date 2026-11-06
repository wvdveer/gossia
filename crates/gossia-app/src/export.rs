//! Exporting a tab as a standalone file (DESIGN.md, "Import and export",
//! "Exporting"): *File > Export...* and *Export...* on a tab's menu open a
//! dialog for the format, the layout and the schema; *Export...* in it asks
//! for the file.
//!
//! The schema tab of a data tab can be written beside it, as `<name>.xsd` or
//! `<name>.schema.json` in the same folder. Both files are written atomically.

use gossia_i18n::tr;
use std::path::{Path, PathBuf};

use gossia_calc::{ExportFormat, SchemaFormat};

use crate::app::GossiaApp;
use crate::file::save_atomic;
use crate::view::named;
use crate::workbook_file::{Browsing, FileDialog};

/// For CSV and TSV in the export dialog: which array of objects is written,
/// when the tab has more than one, and the fields that are left out.
fn rows_choice(ui: &mut egui::Ui, calc: &gossia_calc::Calc, plan: &mut ExportPlan) {
    if plan.rows.len() > 1 {
        ui.label(tr!("Rows from"));
        for at in plan.rows.clone() {
            let label = if at.is_root() {
                plan.tab.clone()
            } else {
                format!("{}.{at}", plan.tab)
            };
            if ui.radio(plan.at.as_ref() == Some(&at), label).clicked() {
                plan.at = Some(at);
            }
        }
    }
    let left_out = plan
        .at
        .as_ref()
        .and_then(|at| calc.export_row_columns(&plan.tab, at))
        .map(|(_, left_out)| left_out)
        .unwrap_or_default();
    if !left_out.is_empty() {
        ui.weak(tr!(
            "Lists and maps have no place in a row, so these are left out: {fields}.",
            fields = left_out.join(", ")
        ));
    }
    ui.weak(tr!(
        "Fields of records inside an item become columns such as address.city."
    ));
    ui.add_space(4.0);
}

/// The choices of the export dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportPlan {
    pub tab: String,
    /// What the tab can be written as: its own format first.
    pub formats: Vec<ExportFormat>,
    pub format: ExportFormat,
    /// Indented by two spaces, or compact.
    pub pretty: bool,
    /// The schema tab that can be written beside the file, if any.
    pub schema: Option<String>,
    pub with_schema: bool,
    /// For CSV and TSV: the arrays of objects that can be written as rows,
    /// and the one chosen.
    pub rows: Vec<gossia_core::Path>,
    pub at: Option<gossia_core::Path>,
}

impl ExportPlan {
    /// Whether the schema is written too: asked for, there, and not for text.
    pub fn writes_schema(&self) -> bool {
        self.with_schema
            && self.schema.is_some()
            && self.format != ExportFormat::Text
            && !self.format.is_rows()
    }
}

impl GossiaApp {
    /// Opens the export dialog for a tab.
    pub fn begin_export(&mut self, tab: &str) {
        match self.export_plan(tab) {
            Ok(plan) => self.export = Some(plan),
            Err(e) => self.notice = Some(e),
        }
    }

    /// What the dialog starts with: the tab's own format, indented, without
    /// the schema.
    pub fn export_plan(&self, tab: &str) -> Result<ExportPlan, String> {
        let formats = self.calc().export_formats(tab);
        let format = *formats
            .first()
            .ok_or_else(|| tr!("there is no tab {tab}", tab = tab))?;
        let rows = self.calc().export_row_arrays(tab);
        Ok(ExportPlan {
            at: rows.first().cloned(),
            rows,
            tab: tab.to_owned(),
            formats,
            format,
            pretty: true,
            schema: self.calc().export_schema(tab).map(str::to_owned),
            with_schema: false,
        })
    }

    /// The extension of the exported file: `xsd` for an XSD schema tab.
    pub fn export_extension(&self, plan: &ExportPlan) -> &'static str {
        if plan.format == ExportFormat::XML
            && self.calc().schema_format(&plan.tab) == Some(SchemaFormat::Xsd)
        {
            "xsd"
        } else {
            plan.format.extension()
        }
    }

    /// The file the schema is written to, beside `path`.
    pub fn export_schema_path(&self, plan: &ExportPlan, path: &Path) -> Option<PathBuf> {
        if !plan.writes_schema() {
            return None;
        }
        let schema = plan.schema.as_deref()?;
        let stem = path
            .file_stem()
            .map_or_else(|| plan.tab.clone(), |s| s.to_string_lossy().into_owned());
        let name = match self.calc().schema_format(schema)? {
            SchemaFormat::Json => format!("{stem}.schema.json"),
            SchemaFormat::Xsd => format!("{stem}.xsd"),
            SchemaFormat::Copybook => format!("{stem}.cpy"),
        };
        Some(path.with_file_name(name))
    }

    /// *Export...* in the dialog: asks for the file.
    pub fn choose_export_file(&mut self) {
        let Some(plan) = &self.export else {
            return;
        };
        let extension = self.export_extension(plan);
        let name = format!("{}.{extension}", plan.tab);
        if let Some(host) = crate::host::current() {
            match self.export_files(Path::new(&name)) {
                Ok(files) => {
                    for (path, bytes) in files {
                        host.save_file(&path.to_string_lossy(), bytes);
                    }
                    self.export = None;
                }
                Err(e) => self.notice = Some(e),
            }
            return;
        }
        if self.files.browse {
            let kind = match extension {
                "json" => "JSON",
                "xml" => "XML",
                "xsd" => "XSD",
                "blk" => "COBOL records",
                "cpy" => "COBOL copybook",
                "csv" => "CSV",
                "tsv" => "TSV",
                _ => tr!("Text"),
            };
            self.files.browser = crate::workbook_file::new_browser()
                .title(&tr!("Export {tab}", tab = plan.tab))
                .initial_directory(self.start_folder())
                .default_file_name(&name)
                .add_save_extension(kind, extension)
                .default_save_extension(kind);
            self.files.browser.save_file();
            self.files.browsing = Some(Browsing::Export);
            return;
        }
        self.files.dialog = Some(FileDialog {
            purpose: Browsing::Export,
            text: self.start_folder().join(name).display().to_string(),
            error: None,
        });
    }

    /// Writes the file of the dialog's tab, and its schema if asked, and
    /// closes the dialog. A path without an extension is given the format's.
    pub fn export_to(&mut self, path: &Path) -> Result<(), String> {
        for (path, bytes) in self.export_files(path)? {
            save_atomic(&path, &bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        }
        self.export = None;
        Ok(())
    }

    /// The files of the dialog's tab, and its schema if asked, to be written
    /// at `path` and beside it.
    fn export_files(&self, path: &Path) -> Result<Vec<(PathBuf, Vec<u8>)>, String> {
        let plan = self
            .export
            .clone()
            .ok_or(tr!("there is nothing to export"))?;
        let path = if path.extension().is_some() {
            path.to_path_buf()
        } else {
            path.with_extension(self.export_extension(&plan))
        };
        let bytes = match (&plan.at, plan.format.is_rows()) {
            (Some(at), true) => self
                .calc()
                .export_rows(&plan.tab, at, plan.format)?
                .0
                .into_bytes(),
            _ => self
                .calc()
                .export_bytes(&plan.tab, plan.format, plan.pretty)?,
        };
        let schema = match self.export_schema_path(&plan, &path) {
            Some(schema_path) => {
                if schema_path == path {
                    return Err(tr!(
                        "the schema would be written to {path} as well; give the file another name",
                        path = path.display()
                    ));
                }
                let schema = plan.schema.as_deref().expect("a schema is written");
                let format = self
                    .calc()
                    .export_format(schema)
                    .ok_or_else(|| tr!("there is no tab {schema}", schema = schema))?;
                Some((
                    schema_path,
                    self.calc().export_text(schema, format, plan.pretty)?,
                ))
            }
            None => None,
        };
        let mut files = vec![(path, bytes)];
        if let Some((schema_path, schema_text)) = schema {
            files.push((schema_path, schema_text.into_bytes()));
        }
        Ok(files)
    }

    /// The export dialog, while it is open and not waiting for a file.
    pub(crate) fn export_window(&mut self, ctx: &egui::Context) {
        let choosing = self.files.browsing == Some(Browsing::Export)
            || self
                .files
                .dialog
                .as_ref()
                .is_some_and(|d| d.purpose == Browsing::Export);
        if choosing {
            return;
        }
        let Some(mut plan) = self.export.take() else {
            return;
        };
        let schema_file = self
            .export_schema_path(
                &ExportPlan {
                    with_schema: true,
                    ..plan.clone()
                },
                Path::new(&format!("{}.{}", plan.tab, self.export_extension(&plan))),
            )
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));
        let (mut choose, mut cancel, mut open) = (false, false, true);
        egui::Window::new(tr!("Export {tab}", tab = plan.tab))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .min_width(360.0)
            .show(ctx, |ui| {
                if plan.formats.len() > 1 {
                    ui.label(tr!("Format"));
                    for format in plan.formats.clone() {
                        if ui.radio(plan.format == format, format.label()).clicked() {
                            plan.format = format;
                        }
                    }
                    ui.add_space(4.0);
                }
                if plan.format.is_rows() {
                    rows_choice(ui, self.calc(), &mut plan);
                }
                let shaped = plan.format != ExportFormat::Text && !plan.format.is_rows();
                ui.add_enabled_ui(shaped, |ui| {
                    ui.horizontal(|ui| {
                        if ui.radio(plan.pretty, tr!("Indented")).clicked() {
                            plan.pretty = true;
                        }
                        if ui.radio(!plan.pretty, tr!("Compact")).clicked() {
                            plan.pretty = false;
                        }
                    });
                    if let (Some(schema), Some(file)) = (&plan.schema, &schema_file) {
                        let response = ui.checkbox(
                            &mut plan.with_schema,
                            tr!("Also write the schema, {schema}, beside it", schema = schema),
                        );
                        named(&response, tr!("Export the schema"));
                        if plan.with_schema {
                            ui.weak(tr!("As {file} (named after the file), replacing a file of that name.", file = file));
                        }
                    }
                });
                ui.weak(tr!("Formulas are not written: the values they computed are."));
                let format = match plan.format {
                    ExportFormat::Data(format) => Some(format),
                    _ => None,
                };
                let left_out = format.map_or(0, |f| self.calc().comments_left_out(&plan.tab, f));
                if left_out > 0 {
                    ui.weak(format!(
                        "{} has no place for comments, so the {} of {} {} not written; the workbook keeps {}.",
                        format.map(|f| f.label()).unwrap_or_default(),
                        if left_out == 1 { "comment" } else { "comments" },
                        plan.tab,
                        if left_out == 1 { "is" } else { "are" },
                        if left_out == 1 { "it" } else { "them" },
                    ));
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    choose = ui.button(tr!("Export...")).clicked();
                    cancel = ui.button(tr!("Cancel")).clicked();
                });
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    cancel = true;
                }
            });
        if open && !cancel {
            self.export = Some(plan);
            if choose {
                self.choose_export_file();
            }
        }
    }
}
