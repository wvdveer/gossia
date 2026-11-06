//! *Export as PDF...* and *Export as HTML...* on a report tab's menu
//! (DESIGN.md, Reports, "PDF export" and "Export as HTML"): a dialog says
//! what the file will hold, *Export...* in it asks for the file, and the
//! file is written when the report is calculated, with the parameter values
//! in the parameter bar.
//!
//! The report is shown while it is exported, so that it is calculated; in a
//! workbook that is not trusted nothing runs and the file has what the
//! results cache has, which the dialog says. The file is written
//! atomically, by `gossia-report`, from the same layout the tab draws.

use gossia_i18n::tr;
use std::path::{Path, PathBuf};

use crate::app::GossiaApp;
use crate::file::save_atomic;
use crate::report_view::local_time;
use crate::view::named;
use crate::workbook_file::{Browsing, FileDialog};

/// What a report is exported as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportFormat {
    Pdf,
    Html,
}

impl ReportFormat {
    /// Its name, as the menus say it.
    pub fn name(self) -> &'static str {
        match self {
            ReportFormat::Pdf => "PDF",
            ReportFormat::Html => "HTML",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            ReportFormat::Pdf => "pdf",
            ReportFormat::Html => "html",
        }
    }

    /// The purpose of the file dialog that asks for its file.
    fn browsing(self) -> Browsing {
        match self {
            ReportFormat::Pdf => Browsing::ExportPdf,
            ReportFormat::Html => Browsing::ExportHtml,
        }
    }
}

/// The export of a report as PDF or HTML, while its dialog is open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfExport {
    pub tab: String,
    pub format: ReportFormat,
    /// *Print...*: the PDF goes to a temporary file, opened in the system's
    /// PDF program to be printed from there (see `report_print`).
    pub print: bool,
    /// The file chosen, while the report is still being calculated: it is
    /// written when the report is done.
    pub waiting: Option<PathBuf>,
    /// Why the last attempt failed.
    pub error: Option<String>,
}

impl GossiaApp {
    /// *Export as PDF...*: shows the report, so that it is calculated, and
    /// opens the dialog.
    pub fn begin_export_pdf(&mut self, tab: &str) {
        self.begin_report_export(tab, ReportFormat::Pdf, false);
    }

    /// *Export as HTML...*: as *Export as PDF...*, for an HTML page.
    pub fn begin_export_html(&mut self, tab: &str) {
        self.begin_report_export(tab, ReportFormat::Html, false);
    }

    pub(crate) fn begin_report_export(&mut self, tab: &str, format: ReportFormat, print: bool) {
        if self.calc().report(tab).is_none() {
            self.notice = Some(tr!("there is no report {tab}", tab = tab));
            return;
        }
        if self.sheet != tab {
            self.switch_to(tab);
        }
        self.pdf_export = Some(PdfExport {
            tab: tab.to_owned(),
            format,
            print,
            waiting: None,
            error: None,
        });
    }

    /// *Export...* in the dialog: asks for the file. *Print...* needs none:
    /// its PDF goes to a temporary file.
    pub fn choose_pdf_file(&mut self) {
        let Some(export) = &self.pdf_export else {
            return;
        };
        if export.print {
            let done = crate::report_print::print_path(&export.tab)
                .and_then(|path| self.export_report_to(&path));
            if let Err(e) = done
                && let Some(export) = &mut self.pdf_export
            {
                export.error = Some(e);
            }
            return;
        }
        let format = export.format;
        let name = format!("{}.{}", export.tab, format.extension());
        // In a host the file is asked for once the file is made.
        if crate::host::current().is_some() {
            if let Err(e) = self.export_report_to(Path::new(&name))
                && let Some(export) = &mut self.pdf_export
            {
                export.error = Some(e);
            }
            return;
        }
        if self.files.browse {
            self.files.browser = crate::workbook_file::new_browser()
                .title(&tr!(
                    "Export {tab} as {kind}",
                    tab = export.tab,
                    kind = format.name()
                ))
                .initial_directory(self.start_folder())
                .default_file_name(&name)
                .add_save_extension(format.name(), format.extension())
                .default_save_extension(format.name());
            self.files.browser.save_file();
            self.files.browsing = Some(format.browsing());
            return;
        }
        self.files.dialog = Some(FileDialog {
            purpose: format.browsing(),
            text: self.start_folder().join(name).display().to_string(),
            error: None,
        });
    }

    /// The file is chosen: writes it now, or when the report is
    /// calculated. A path without an extension is given `.pdf` or `.html`.
    pub fn export_report_to(&mut self, path: &Path) -> Result<(), String> {
        let export = self
            .pdf_export
            .as_mut()
            .ok_or(tr!("there is nothing to export"))?;
        let path = if path.extension().is_some() {
            path.to_path_buf()
        } else {
            path.with_extension(export.format.extension())
        };
        let missing = self.session.calc().missing_params(&export.tab);
        if !missing.is_empty() {
            return Err(tr!(
                "Fill in {params} to export the report.",
                params = gossia_i18n::and_list(&missing)
            ));
        }
        export.error = None;
        export.waiting = Some(path);
        self.write_pdf_when_ready();
        Ok(())
    }

    /// Writes the file waited for, once nothing of the report is left to
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
        let host = crate::host::current();
        let bytes = match export.format {
            ReportFormat::Pdf => self.pdf_bytes(&export.tab),
            ReportFormat::Html => self.html_bytes(&export.tab),
        };
        let written = bytes
            .and_then(|bytes| match &host {
                Some(host) => {
                    host.save_file(&path.to_string_lossy(), bytes);
                    Ok(())
                }
                None => save_atomic(&path, &bytes).map_err(|e| format!("{e}")),
            })
            .map_err(|e| format!("{}: {e}", path.display()))
            .and_then(|()| {
                if export.print {
                    self.open_outside(&path)
                } else {
                    Ok(())
                }
            });
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
        gossia_report::workbook::pdf(self.calc(), tab)
    }

    /// The report as an HTML page, laid out as its tab draws it.
    pub fn html_bytes(&self, tab: &str) -> Result<Vec<u8>, String> {
        gossia_report::workbook::html(self.calc(), tab).map(String::into_bytes)
    }

    /// The dialog of a report's export, while it is open and not waiting
    /// for a file.
    pub(crate) fn pdf_window(&mut self, ctx: &egui::Context) {
        let exporting = |b: &Browsing| matches!(b, Browsing::ExportPdf | Browsing::ExportHtml);
        let choosing = self.files.browsing.as_ref().is_some_and(exporting)
            || self
                .files
                .dialog
                .as_ref()
                .is_some_and(|d| exporting(&d.purpose));
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
        let kind = export.format.name();
        let html = export.format == ReportFormat::Html;
        let print = export.print;
        let (mut choose, mut cancel, mut open) = (false, false, true);
        let title = if print {
            tr!("Print {tab}", tab = export.tab)
        } else {
            tr!("Export {tab} as {kind}", tab = export.tab, kind = kind)
        };
        egui::Window::new(title)
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .min_width(380.0)
            .show(ctx, |ui| {
                if print {
                    ui.label(tr!(
                        "Opens the report as a PDF in your PDF program, where you print it \
                         and choose the printer."
                    ));
                }
                ui.label(match (multipage, html) {
                    (true, false) => {
                        tr!("Each page on the paper of the page setup, as the page view shows it.")
                    }
                    (true, true) => {
                        "Each page as the page view shows it, in one file with no scripts; \
                         printed, each page goes on a sheet of the paper of the page setup."
                    }
                    (false, false) => {
                        "One page, as wide as the paper of the page setup less its margins, \
                         and as tall as the report."
                    }
                    (false, true) => {
                        "The report as it is shown, in one file with no scripts, as wide as \
                         the paper of the page setup less its margins."
                    }
                });
                if has_params {
                    ui.weak(tr!("With the parameter values in the parameter bar."));
                }
                match &untrusted {
                    Some(Some(time)) => {
                        ui.weak(tr!(
                            "This workbook is not trusted, so nothing runs: the {kind} has the \
                             results calculated on {time}.",
                            kind = kind,
                            time = time
                        ));
                    }
                    Some(None) => {
                        ui.weak(tr!(
                            "This workbook is not trusted, so nothing runs, and no results of \
                             the report were kept: the {kind} shows #UNTRUSTED! in their place.",
                            kind = kind
                        ));
                    }
                    None => {}
                }
                if !missing.is_empty() {
                    ui.colored_label(
                        crate::view::error_color(ui),
                        tr!(
                            "Fill in {params} to export the report.",
                            params = gossia_i18n::and_list(&missing)
                        ),
                    );
                }
                if export.waiting.is_some() {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(tr!(
                            "Calculating the report: the {kind} is written when it is done.",
                            kind = kind
                        ));
                    });
                }
                if let Some(error) = &export.error {
                    ui.colored_label(crate::view::error_color(ui), error);
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    let ready = missing.is_empty() && export.waiting.is_none();
                    let button = if print {
                        tr!("Print...")
                    } else {
                        tr!("Export...")
                    };
                    let response = ui.add_enabled(ready, egui::Button::new(button));
                    named(
                        &response,
                        match (print, html) {
                            (true, _) => tr!("Print report"),
                            (false, true) => tr!("Export HTML"),
                            (false, false) => tr!("Export PDF"),
                        },
                    );
                    choose = response.clicked();
                    cancel = ui.button(tr!("Cancel")).clicked();
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
