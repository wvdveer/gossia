//! Report tabs in the window (DESIGN.md, "Reports" and "Managing tabs"): the
//! page a report tab shows, its menu, and making, duplicating and deleting
//! one. A report holds no data, so the data tree, the formula bar and the
//! other views of a data tab are not shown for it.
//!
//! The report itself is drawn by `report_view.rs`.

use cliotree_core::Path;
use cliotree_core::report::{self, ReportDesign, ReportKind};

use crate::app::ClioApp;

/// The arrays a new report can start from: each data tab's arrays, as
/// `(tab, path, how a formula names it)`.
pub type StartingPoints = Vec<(String, Path, String)>;

impl ClioApp {
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
                let shown = cliotree_calc::Loc::new(tab, path.clone()).to_string();
                out.push((tab.to_owned(), path, shown));
            }
        }
        out
    }

    /// Makes a report tab at the end of the tabs and shows it: empty, or
    /// starting from the array `start` (a tab and a path in it). One entry in
    /// the history, *New report Sales*.
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
        let design = match start {
            None => ReportDesign::new(kind),
            Some((tab, path)) => {
                let spec = self
                    .calc()
                    .spec(tab)
                    .ok_or_else(|| format!("there is no tab called {tab}"))?;
                report::starting_design(kind, spec, tab, path).ok_or_else(|| {
                    format!(
                        "{} is not an array",
                        cliotree_calc::Loc::new(tab.as_str(), path.clone())
                    )
                })?
            }
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
    /// *Edit report*, *Export as PDF...* and *Copy batch command* come with
    /// later steps of 0.2.0, and are shown greyed until then.
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
            for (label, why) in [
                ("Export as PDF...", "PDF export is not built yet"),
                (
                    "Copy batch command",
                    "The command-line program, cliotree-cli, is not built yet",
                ),
            ] {
                ui.add_enabled(false, egui::Button::new(label))
                    .on_disabled_hover_text(why);
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
}
