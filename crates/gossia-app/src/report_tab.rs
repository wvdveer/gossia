//! Report tabs in the window (DESIGN.md, "Reports" and "Managing tabs"): the
//! page a report tab shows, its menu, and making, duplicating and deleting
//! one. A report holds no data, so the data tree, the formula bar and the
//! other views of a data tab are not shown for it.
//!
//! The report itself is drawn by `report_view.rs`.

use gossia_core::Path;
use gossia_core::report::{self, ReportDesign, ReportKind};

use crate::app::GossiaApp;

/// The arrays a new report can start from: each data tab's arrays, as
/// `(tab, path, how a formula names it)`.
pub type StartingPoints = Vec<(String, Path, String)>;

impl GossiaApp {
    /// Whether the current tab is a report.
    pub fn in_report(&self) -> bool {
        self.calc().is_report(&self.sheet)
    }

    /// The arrays of the data tabs that a new report can start from.
    pub fn starting_points(&self) -> StartingPoints {
        let calc = self.calc();
        let mut out = Vec::new();
        for tab in calc.sheet_names() {
            if calc.is_schema_sheet(tab) {
                continue;
            }
            let Some(spec) = calc.spec(tab) else {
                continue;
            };
            for path in report::starting_arrays(spec) {
                let shown = gossia_calc::Loc::new(tab, path.clone()).to_string();
                out.push((tab.to_owned(), path, shown));
            }
        }
        out
    }

    /// Makes a report tab at the end of the tabs and shows it: empty, or
    /// starting from the array `start` (a tab and a path in it), which makes
    /// a multipage report whose detail band shows each item. One entry in the
    /// history, *New report Sales*.
    pub fn create_report_tab(
        &mut self,
        name: &str,
        kind: ReportKind,
        start: Option<&(String, Path)>,
    ) -> Result<(), String> {
        if !self.can_change_tabs() {
            return Err(self.notice.clone().unwrap_or_default());
        }
        let name = name.trim();
        // Starting from an array makes a multipage report whose detail band
        // reads each item (owner, 2026-11-03).
        let design = match start {
            Some((tab, path)) => {
                let spec = self
                    .calc()
                    .spec(tab)
                    .ok_or_else(|| format!("there is no tab {tab}"))?;
                report::starting_design(spec, tab, path)
                    .ok_or_else(|| format!("{tab} has no array at {path}"))?
            }
            None => ReportDesign::new(kind),
        };
        self.session
            .calc_mut()
            .create_report(name, design)
            .map_err(|e| e.to_string())?;
        self.follow_tabs();
        self.show_tab(name);
        self.session.tick();
        Ok(())
    }

    /// *Duplicate* on a report tab's menu: a copy of the design, after it,
    /// shown. Returns the copy's name.
    pub fn duplicate_report_tab(&mut self, name: &str) -> Result<String, String> {
        if !self.can_change_tabs() {
            return Err(self.notice.clone().unwrap_or_default());
        }
        let copy = self
            .session
            .calc_mut()
            .duplicate_report(name)
            .map_err(|e| e.to_string())?;
        self.follow_tabs();
        self.show_tab(&copy);
        self.session.tick();
        Ok(copy)
    }

    /// The menu of a report tab, in the Edit menu and the tab's context menu.
    pub(crate) fn report_menu_items(&mut self, ui: &mut egui::Ui, tab: &str) {
        let can_change = self.editing.is_none() && self.session.running_procedure().is_none();
        ui.add_enabled_ui(can_change, |ui| {
            if ui.button("New tab...").clicked() {
                self.begin_new_tab();
                ui.close();
            }
            if ui.button("New report...").clicked() {
                self.begin_new_report();
                ui.close();
            }
            if ui.button("Import...").clicked() {
                self.begin_import();
                ui.close();
            }
            ui.separator();
            let mut on = tab == self.sheet && self.layout_active();
            if ui.checkbox(&mut on, "Edit report").clicked() {
                self.switch_to(tab);
                self.set_layout(on);
                ui.close();
            }
            if ui.button("Export as PDF...").clicked() {
                self.begin_export_pdf(tab);
                ui.close();
            }
            if ui.button("Export as HTML...").clicked() {
                self.begin_export_html(tab);
                ui.close();
            }
            let saved = self.doc.path.is_some();
            let copy = ui
                .add_enabled(saved, egui::Button::new("Copy batch command"))
                .on_hover_text(
                    "Copies a command line that writes this report as a PDF with bidwillii,                      with the parameter values chosen and the workbook's trust digest",
                )
                .on_disabled_hover_text("Save the workbook first: the command runs its file");
            if copy.clicked() {
                self.copy_batch_command(tab);
                ui.close();
            }
            ui.separator();
            if ui.button("Rename tab...").clicked() {
                self.begin_rename_tab(tab);
                ui.close();
            }
            if ui.button("Duplicate").clicked() {
                if let Err(why) = self.duplicate_report_tab(tab) {
                    self.notice = Some(why);
                }
                ui.close();
            }
            if ui.button("Delete tab...").clicked() {
                self.begin_delete_tab(tab);
                ui.close();
            }
        });
    }

    /// *Copy batch command*: puts the command line on the clipboard and
    /// says so, or says why there is none.
    pub fn copy_batch_command(&mut self, tab: &str) {
        match self.batch_command(tab) {
            Ok((command, note)) => {
                self.tables.to_clipboard = Some(command);
                self.notice = Some(format!(
                    "Copied the command that writes {tab} as a PDF.{}",
                    note.map(|n| format!(" {n}")).unwrap_or_default()
                ));
            }
            Err(why) => self.notice = Some(why),
        }
    }

    /// The command line that writes the report `tab` as a PDF beside the
    /// workbook (DESIGN.md, "Batch export from the command line"): the
    /// program next to this one, the workbook's file, each parameter value
    /// chosen in the parameter bar (the others take their defaults when it
    /// runs), and the trust digest of the file as it was saved. With it, a
    /// note when the file is not as the window has it.
    pub fn batch_command(&self, tab: &str) -> Result<(String, Option<String>), String> {
        let calc = self.calc();
        let design = calc
            .report(tab)
            .ok_or_else(|| format!("there is no report {tab}"))?;
        let file = self
            .doc
            .path
            .clone()
            .ok_or("Save the workbook first: the command runs its file.")?;
        let file = std::path::absolute(&file).unwrap_or(file);
        let saved = crate::file::saved_digest(&file, &self.user.prefs.limits)?;
        let program = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|d| d.join(CLI_PROGRAM)))
            .unwrap_or_else(|| std::path::PathBuf::from(CLI_PROGRAM));
        let out = file.with_file_name(format!("{tab}.pdf"));
        let mut words = vec![
            program.display().to_string(),
            "report".to_owned(),
            file.display().to_string(),
            tab.to_owned(),
            "--out".to_owned(),
            out.display().to_string(),
        ];
        let chosen = calc.chosen_params(tab);
        for param in &design.params {
            let Some(value) = chosen.and_then(|c| c.get(&param.name)) else {
                continue;
            };
            let values: Vec<&gossia_core::Value> = match value {
                gossia_core::Value::Array(items) => items.iter().map(|n| n.value()).collect(),
                other => vec![other],
            };
            for value in values {
                if !gossia_core::report::param_is_empty(value) {
                    words.push("--param".to_owned());
                    words.push(format!(
                        "{}={}",
                        param.name,
                        gossia_core::report::param_text(value)
                    ));
                }
            }
        }
        words.push("--trust-digest".to_owned());
        words.push(saved.clone());
        let note = (saved != calc.fingerprint().hash).then(|| {
            "The workbook has unsaved changes to its formulas, modules or reports; the              command runs the file as it was saved."
                .to_owned()
        });
        let command = words.iter().map(|w| quote(w)).collect::<Vec<_>>().join(" ");
        Ok((command, note))
    }
}

/// The command-line program, installed next to the application.
pub const CLI_PROGRAM: &str = if cfg!(windows) {
    "bidwillii.exe"
} else {
    "bidwillii"
};

/// A word of a command line, quoted for the shell of this system when it
/// needs to be: in double quotes on Windows, in single quotes elsewhere.
pub fn quote(word: &str) -> String {
    let plain = !word.is_empty()
        && word.chars().all(|c| {
            c.is_alphanumeric() || "-_./:=+,@".contains(c) || (cfg!(windows) && c == '\\')
        });
    if plain {
        return word.to_owned();
    }
    if cfg!(windows) {
        format!("\"{}\"", word.replace('"', "\\\""))
    } else {
        format!("'{}'", word.replace('\'', r"'\''"))
    }
}
