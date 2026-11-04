//! *Export as PDF...* on a report tab's menu (DESIGN.md, Reports, "PDF
//! export"): a dialog says what the PDF will hold, *Export...* in it asks
//! for the file, and the PDF is written when the report is calculated, with
//! the parameter values in the parameter bar.
//!
//! The report is shown while it is exported, so that it is calculated; in a
//! workbook that is not trusted nothing runs and the PDF has what the
//! results cache has, which the dialog says. The file is written
//! atomically, by `cliotree-report`, from the same layout the tab draws.

use std::path::{Path, PathBuf};

use crate::app::ClioApp;
use crate::file::save_atomic;
use crate::report_view::local_time;
use crate::view::named;
use crate::workbook_file::{Browsing, FileDialog};

/// The PDF export of a report, while its dialog is open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfExport {
    pub tab: String,
    /// The file chosen, while the report is still being calculated: it is
    /// written when the report is done.
    pub waiting: Option<PathBuf>,
    /// Why the last attempt failed.
    pub error: Option<String>,
}

impl ClioApp {
    /// *Export as PDF...*: shows the report, so that it is calculated, and
    /// opens the dialog.
    pub fn begin_export_pdf(&mut self, tab: &str) {
        if self.calc().report(tab).is_none() {
            self.notice = Some(format!("there is no report {tab}"));
            return;
        }
        if self.sheet != tab {
            self.switch_to(tab);
        }
        self.pdf_export = Some(PdfExport {
            tab: tab.to_owned(),
            waiting: None,
            error: None,
        });
    }

    /// *Export...* in the dialog: asks for the file.
    pub fn choose_pdf_file(&mut self) {
        let Some(export) = &self.pdf_export else {
            return;
        };
        let name = format!("{}.pdf", export.tab);
        if self.files.browse {
            self.files.browser = crate::workbook_file::new_browser()
                .title(&format!("Export {} as PDF", export.tab))
                .initial_directory(self.start_folder())
                .default_file_name(&name)
                .add_save_extension("PDF", "pdf")
                .default_save_extension("PDF");
            self.files.browser.save_file();
            self.files.browsing = Some(Browsing::ExportPdf);
            return;
        }
        self.files.dialog = Some(FileDialog {
            purpose: Browsing::ExportPdf,
            text: self.start_folder().join(name).display().to_string(),
            error: None,
        });
    }

    /// The file is chosen: writes the PDF now, or when the report is
    /// calculated. A path without an extension is given `.pdf`.
    pub fn export_pdf_to(&mut self, path: &Path) -> Result<(), String> {
        let export = self
            .pdf_export
            .as_mut()
            .ok_or("there is nothing to export")?;
        let path = if path.extension().is_some() {
            path.to_path_buf()
        } else {
            path.with_extension("pdf")
        };
        let missing = self.session.calc().missing_params(&export.tab);
        if !missing.is_empty() {
            return Err(format!(
                "Fill in {} to export the report.",
                crate::report_params::and_list(&missing)
            ));
        }
        export.error = None;
        export.waiting = Some(path);
        self.write_pdf_when_ready();
        Ok(())
    }

    /// Writes the PDF waited for, once nothing of the report is left to
    /// calculate. A failure is shown in the dialog.
    pub(crate) fn write_pdf_when_ready(&mut self) {
        let Some(export) = self.pdf_export.clone() else {
            return;
        };
        let Some(path) = export.waiting else {
            return;
        };
        // The report is calculated while it is shown.
        if self.sheet != export.tab {
            self.switch_to(&export.tab);
        }
        if self.calc().is_trusted() && !self.session.is_idle() {
            return;
        }
        let written = self
            .pdf_bytes(&export.tab)
            .and_then(|bytes| save_atomic(&path, &bytes).map_err(|e| format!("{e}")))
            .map_err(|e| format!("{}: {e}", path.display()));
        match written {
            Ok(()) => self.pdf_export = None,
            Err(e) => {
                if let Some(export) = &mut self.pdf_export {
                    export.waiting = None;
                    export.error = Some(e);
                }
            }
        }
    }

    /// The report as a PDF, laid out as its tab draws it: a multipage
    /// report page by page, a single page report as one page.
    pub fn pdf_bytes(&self, tab: &str) -> Result<Vec<u8>, String> {
        cliotree_report::workbook::pdf(self.calc(), tab)
    }

    /// The dialog of a PDF export, while it is open and not waiting for a
    /// file.
    pub(crate) fn pdf_window(&mut self, ctx: &egui::Context) {
        let choosing = self.files.browsing == Some(Browsing::ExportPdf)
            || self
                .files
                .dialog
                .as_ref()
                .is_some_and(|d| d.purpose == Browsing::ExportPdf);
        if choosing {
            return;
        }
        self.write_pdf_when_ready();
        let Some(export) = self.pdf_export.clone() else {
            return;
        };
        let calc = self.calc();
        let Some(design) = calc.report(&export.tab) else {
            self.pdf_export = None;
            return;
        };
        let multipage = design.bands().is_some();
        let has_params = !design.params.is_empty();
        let missing = calc.missing_params(&export.tab);
        let untrusted = (!calc.is_trusted()).then(|| {
            calc.report_cache(&export.tab)
                .map(|c| local_time(&c.calculated))
        });
        let (mut choose, mut cancel, mut open) = (false, false, true);
        egui::Window::new(format!("Export {} as PDF", export.tab))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .min_width(380.0)
            .show(ctx, |ui| {
                ui.label(if multipage {
                    "Each page on the paper of the page setup, as the page view shows it."
                } else {
                    "One page, as wide as the paper of the page setup less its margins, \
                     and as tall as the report."
                });
                if has_params {
                    ui.weak("With the parameter values in the parameter bar.");
                }
                match &untrusted {
                    Some(Some(time)) => {
                        ui.weak(format!(
                            "This workbook is not trusted, so nothing runs: the PDF has the \
                             results calculated on {time}."
                        ));
                    }
                    Some(None) => {
                        ui.weak(
                            "This workbook is not trusted, so nothing runs, and no results of \
                             the report were kept: the PDF shows #UNTRUSTED! in their place.",
                        );
                    }
                    None => {}
                }
                if !missing.is_empty() {
                    ui.colored_label(
                        crate::view::error_color(ui),
                        format!(
                            "Fill in {} to export the report.",
                            crate::report_params::and_list(&missing)
                        ),
                    );
                }
                if export.waiting.is_some() {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Calculating the report: the PDF is written when it is done.");
                    });
                }
                if let Some(error) = &export.error {
                    ui.colored_label(crate::view::error_color(ui), error);
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    let ready = missing.is_empty() && export.waiting.is_none();
                    let response = ui.add_enabled(ready, egui::Button::new("Export..."));
                    named(&response, "Export PDF");
                    choose = response.clicked();
                    cancel = ui.button("Cancel").clicked();
                });
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    cancel = true;
                }
            });
        if !open || cancel {
            self.pdf_export = None;
        } else if choose {
            self.choose_pdf_file();
        }
        if self
            .pdf_export
            .as_ref()
            .is_some_and(|e| e.waiting.is_some())
        {
            ctx.request_repaint();
        }
    }
}
