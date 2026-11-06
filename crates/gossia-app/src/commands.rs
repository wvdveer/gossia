//! Gossia's commands by name, for a host that offers them in its own command
//! list (DESIGN.md, Web port, 0.3.0, In VS Code: "Gossia's own commands are
//! also offered in VS Code's Command Palette"). Each does what its menu item
//! does, and is refused, with the reason, where the menu item is greyed.

use crate::GossiaApp;
use gossia_i18n::tr;

/// The commands: a name, which the host's command is called by
/// (`gossia.<name>` in VS Code), and the title it shows.
pub const COMMANDS: &[(&str, &str)] = &[
    ("newTab", "New Tab..."),
    ("import", "Import..."),
    ("export", "Export..."),
    ("exportPdf", "Export as PDF..."),
    ("exportHtml", "Export as HTML..."),
    ("undo", "Undo"),
    ("redo", "Redo"),
    ("recalculate", "Recalculate"),
    ("codeEditor", "Code Editor"),
    ("dataSources", "Data Sources..."),
    ("disconnect", "Disconnect from Data Sources..."),
    ("options", "Options..."),
    ("toolbar", "Show or Hide the Toolbar"),
    ("issues", "Show or Hide the Issues Panel"),
    ("inspector", "Show or Hide the Node Inspector"),
];

impl GossiaApp {
    /// Runs the command called `name`, as its menu item does.
    pub fn run_command(&mut self, name: &str) -> Result<(), String> {
        let editing = self.editing.is_some();
        let busy = || tr!("finish or cancel the edit first").to_owned();
        match name {
            "newTab" => self.begin_new_tab(),
            "import" if editing => return Err(busy()),
            "import" => self.begin_import(),
            "export" | "exportPdf" | "exportHtml" if editing => return Err(busy()),
            "export" if self.in_report() => {
                return Err(tr!(
                    "a report is exported as PDF or HTML: Export as PDF... or Export as HTML..."
                )
                .to_owned());
            }
            "export" => {
                let tab = self.sheet.clone();
                self.begin_export(&tab);
            }
            "exportPdf" if !self.in_report() => {
                return Err(tr!("show a report to export it as PDF").to_owned());
            }
            "exportPdf" => {
                let tab = self.sheet.clone();
                self.begin_export_pdf(&tab);
            }
            "exportHtml" if !self.in_report() => {
                return Err(tr!("show a report to export it as HTML").to_owned());
            }
            "exportHtml" => {
                let tab = self.sheet.clone();
                self.begin_export_html(&tab);
            }
            "undo" | "redo" if editing || self.session.running_procedure().is_some() => {
                return Err(tr!("undo and redo wait for the edit or the procedure").to_owned());
            }
            "undo" if self.calc().undo_label().is_none() => {
                return Err(tr!("there is nothing to undo").to_owned());
            }
            "undo" => self.undo(),
            "redo" if self.calc().redo_label().is_none() => {
                return Err(tr!("there is nothing to redo").to_owned());
            }
            "redo" => self.redo(),
            "recalculate" => self.recalculate(),
            "codeEditor" => self.open_code_editor(),
            "dataSources" => self.open_sources(),
            "disconnect" => self.begin_disconnect(),
            "options" => self.begin_options(None),
            "toolbar" => self.show_toolbar = !self.show_toolbar,
            "issues" => self.show_issues = !self.show_issues,
            "inspector" => self.show_inspector = !self.show_inspector,
            other => return Err(tr!("Gossia has no command {name}", name = other)),
        }
        Ok(())
    }
}
