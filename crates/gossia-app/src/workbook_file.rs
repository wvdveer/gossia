//! Opening and saving workbooks: the file the window is showing, the File menu,
//! the dialogs it opens, and what is asked when there are unsaved changes
//! (DESIGN.md, "File format", "Trust model").
//!
//! Deciding what is in a file is `gossia-calc`'s `Calc::from_package`; the zip
//! and the disk are in [`crate::file`]. This is the part that joins them to the
//! window, and decides whether a file that was opened is trusted.

use gossia_i18n::tr;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use egui_file_dialog::{DialogState, FileDialog as Browser, FileDialogLabels};
use gossia_calc::python::PythonSyntax;
use gossia_calc::{Calc, Decision, FORMAT_MAJOR, FORMAT_MINOR, Meta, Opened, Session, TrustStore};
use serde_json::Value as Json;

use crate::app::GossiaApp;
use crate::file::{read_zip, save_atomic, write_zip};
use crate::trust_ui::TrustBanner;
use crate::view::named;

/// A file browser that draws what the window's fonts have: the Phosphor icons
/// instead of the emoji and symbols it was written with.
pub(crate) fn new_browser() -> Browser {
    use crate::icons as i;
    let with = |icon: &str, text: &str| format!("{icon}  {text}");
    let labels = FileDialogLabels {
        title_select_directory: with(i::FOLDER, tr!("Select Folder")),
        title_select_file: with(i::FOLDER_OPEN, tr!("Open File")),
        title_select_multiple: with(i::FILE_TEXT, tr!("Select Multiple")),
        title_save_file: with(i::FLOPPY, tr!("Save File")),
        reload: with(i::RELOAD, tr!("Reload")),
        working_directory: with(i::ARROW_UP_RIGHT, tr!("Go to working directory")),
        select_all: with(i::CHECK_SQUARE, tr!("Select all")),
        home_dir: with(i::HOUSE, tr!("Home")),
        desktop_dir: with(i::DESKTOP, tr!("Desktop")),
        documents_dir: with(i::FILE_TEXT, tr!("Documents")),
        downloads_dir: with(i::DOWNLOAD, tr!("Downloads")),
        audio_dir: with(i::MUSIC, tr!("Audio")),
        pictures_dir: with(i::IMAGE, tr!("Pictures")),
        videos_dir: with(i::FILM, tr!("Videos")),
        pin_folder: with(i::PIN, tr!("Pin")),
        unpin_folder: with(i::X, tr!("Unpin")),
        rename_pinned_folder: with(i::PENCIL, tr!("Rename")),
        open_button: with(i::FOLDER_OPEN, tr!("Open")),
        save_button: with(i::FLOPPY, tr!("Save")),
        cancel_button: with(i::PROHIBIT, tr!("Cancel")),
        ..FileDialogLabels::default()
    };
    Browser::new()
        .labels(labels)
        .err_icon(i::WARNING)
        .default_file_icon(i::FILE)
        .default_folder_icon(i::FOLDER)
        .device_icon(i::DRIVE)
        .removable_device_icon(i::USB)
        .parent_directory_icon(i::CARET_UP)
        .back_icon(i::CARET_LEFT)
        .forward_icon(i::CARET_RIGHT)
        .new_folder_icon(i::PLUS)
        .menu_icon(i::MENU)
        .search_icon(i::SEARCH)
        .path_edit_icon(i::PENCIL)
}

/// The extension of a workbook.
pub const EXTENSION: &str = "gossia";

/// The file the window is showing, and what goes with it.
pub struct Document {
    /// Where it is saved, if it has been.
    pub path: Option<PathBuf>,
    /// The id the trust model knows the workbook by.
    pub workbook_id: String,
    /// What a newer version put in the file and this one does not know: kept,
    /// and written back unchanged.
    pub unknown: Vec<(String, Vec<u8>)>,
    /// The state of the view of each tab, as it was in the file. What this
    /// version knows of it is put in the window when the file is opened; the
    /// rest is written back as it was (see [`crate::view_state`]).
    pub views: BTreeMap<String, Json>,
    /// The state of the view of the whole workbook as it was in the file
    /// (`view.json`), kept for what this version does not know of it.
    pub workbook_view: Option<Json>,
    /// The version of the forms, and of the chosen parameters of reports,
    /// as they were saved.
    saved_forms: (u64, u64),
}

impl Default for Document {
    fn default() -> Document {
        Document {
            path: None,
            workbook_id: new_workbook_id(),
            unknown: Vec::new(),
            views: BTreeMap::new(),
            workbook_view: None,
            saved_forms: (0, 0),
        }
    }
}

/// What changes when the forms or reports change, or a report's parameter
/// is chosen: unsaved changes that are not in the history.
fn designs_version(calc: &gossia_calc::Calc) -> (u64, u64) {
    (calc.forms_version(), calc.params_version())
}

pub use gossia_user::file::new_workbook_id;

/// What the user was doing when the unsaved changes came up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pending {
    /// Leaving the application.
    Quit,
    /// Choosing a file to open.
    Open,
    /// Opening this file, from the recent files.
    OpenRecent(PathBuf),
    /// Starting a new workbook.
    New,
}

/// The dialog for a file name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDialog {
    pub purpose: Browsing,
    pub text: String,
    pub error: Option<String>,
}

/// What the file browser, or the dialog for a file name, was opened for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Browsing {
    Open,
    SaveAs,
    /// A JSON, XML or text file to import as tabs.
    Import,
    /// The file a tab is exported to.
    Export,
    /// The PDF a report is exported to.
    ExportPdf,
    /// The HTML page a report is exported to.
    ExportHtml,
}

/// What the file commands keep between frames.
#[derive(Debug)]
pub struct FileUi {
    /// The text dialog for a file name. It is what the commands open when the
    /// browser is off, and where a reason is shown when the browser's choice
    /// cannot be used.
    pub dialog: Option<FileDialog>,
    /// The file browser, and what it is open for.
    pub browser: Browser,
    pub browsing: Option<Browsing>,
    /// Whether the commands open the browser (they do in the application; the
    /// tests turn it off to type a name).
    pub browse: bool,
    /// Waiting for the answer to "Save the changes?".
    pub confirm: Option<Pending>,
    /// The user has said to leave without saving.
    pub quit_confirmed: bool,
    /// The title last given to the window.
    pub title: String,
}

impl Default for FileUi {
    fn default() -> FileUi {
        FileUi {
            dialog: None,
            browser: new_browser(),
            browsing: None,
            browse: true,
            confirm: None,
            quit_confirmed: false,
            title: String::new(),
        }
    }
}

impl GossiaApp {
    // ------------------------------------------------------------ state

    /// The name of the file, or `Untitled`.
    pub fn file_name(&self) -> String {
        self.doc
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .map_or_else(
                || tr!("Untitled").to_owned(),
                |name| name.to_string_lossy().into_owned(),
            )
    }

    /// Whether there are changes that a save would keep: edits of the data,
    /// changes to the forms and reports, and parameters chosen.
    pub fn is_dirty(&self) -> bool {
        self.calc().is_modified() || designs_version(self.calc()) != self.doc.saved_forms
    }

    /// The title of the window.
    pub fn title(&self) -> String {
        format!(
            "{}{} - Gossia",
            self.file_name(),
            if self.is_dirty() { " *" } else { "" }
        )
    }

    // ---------------------------------------------------------- opening

    /// Opens a workbook, replacing the one shown. The caller has dealt with
    /// unsaved changes. Nothing changes if the file cannot be read.
    pub fn open_path(&mut self, path: &Path) -> Result<(), String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let package = read_zip(&bytes, &self.user.prefs.limits)?;
        let opened =
            Calc::from_package(Arc::new(PythonSyntax), &package).map_err(|e| e.to_string())?;
        self.install(opened, Some(path.to_path_buf()));
        self.remember_recent(path);
        Ok(())
    }

    /// Opens a workbook the host gave as bytes, under the name of its file
    /// (DESIGN.md, Web port, 0.3.0: the host opens and saves the file).
    pub fn open_bytes(&mut self, name: &str, bytes: &[u8]) -> Result<(), String> {
        let package = read_zip(bytes, &self.user.prefs.limits)?;
        let opened =
            Calc::from_package(Arc::new(PythonSyntax), &package).map_err(|e| e.to_string())?;
        self.install(opened, Some(PathBuf::from(name)));
        Ok(())
    }

    /// Opens a file from the recent files. One that is no longer there is
    /// taken off the list.
    pub fn open_recent(&mut self, path: &Path) {
        if let Err(e) = self.open_path(path) {
            if !path.exists() {
                let gone = path.to_path_buf();
                self.change_prefs(|p| p.remove_recent(&gone));
            }
            self.notice = Some(e);
        }
    }

    /// Replaces the workbook with a new one: a data tab and the schema tab it
    /// is bound to, with auto-update on, so that its fields are added from the
    /// data tab. The caller has dealt with unsaved changes.
    pub fn new_workbook(&mut self) {
        let opened = Opened {
            calc: blank_workbook(),
            workbook_id: new_workbook_id(),
            views: BTreeMap::new(),
            workbook_view: None,
            unknown: Vec::new(),
            warnings: Vec::new(),
            format_version: (FORMAT_MAJOR, FORMAT_MINOR),
        };
        self.install(opened, None);
    }

    /// Shows a workbook that was read. Whether it runs Python is decided here:
    /// a file whose executable content is as the user trusted it, or that has
    /// none, is trusted; any other is not, and the banner asks.
    fn install(&mut self, opened: Opened, path: Option<PathBuf>) {
        let Opened {
            mut calc,
            workbook_id,
            views,
            workbook_view,
            unknown,
            warnings,
            ..
        } = opened;
        let content = calc.fingerprint();
        let decision = if content.items.is_empty() {
            Decision::Trusted
        } else {
            self.user.trust.decide(&workbook_id, &content)
        };
        let (trusted, changes) = match decision {
            Decision::Trusted => (true, Vec::new()),
            Decision::Unknown => (false, Vec::new()),
            Decision::Changed(changes) => (false, changes),
        };
        if !trusted {
            calc.set_trusted(false);
        }
        let prefs = &self.user.prefs;
        calc.set_history_limits(prefs.history_entries, prefs.history_weight);
        // The workbook's limits, held down by the user's ceilings.
        let mut warnings = warnings;
        for (setting, asked, ceiling) in calc.settings().held_down(&prefs.ceilings) {
            warnings.push(tr!(
                "The workbook asks for a {setting} of {asked}; your ceiling of {ceiling} is used.",
                setting = setting.label().to_lowercase(),
                asked = asked,
                ceiling = ceiling
            ));
        }
        let effective = calc.settings().effective(&prefs.ceilings);
        let evaluator = crate::host::evaluator(
            self.user.python_settings(&effective),
            effective.eval_settings(),
        );
        let mut fresh = GossiaApp::build(Session::new(calc, evaluator), trusted);
        fresh.user = std::mem::take(&mut self.user);
        fresh.applied = Some(effective);
        fresh.doc = Document {
            path,
            workbook_id,
            unknown,
            views: views.clone(),
            workbook_view: workbook_view.clone(),
            saved_forms: designs_version(fresh.calc()),
        };
        fresh.restore_view_state(&views);
        fresh.restore_workbook_view(workbook_view.as_ref());
        if !trusted {
            fresh.trust = TrustBanner {
                changes,
                dismissed: false,
            };
        }
        fresh.show_issues = self.show_issues;
        fresh.show_toolbar = self.show_toolbar;
        fresh.show_inspector = self.show_inspector;
        fresh.code = self.code.for_next_workbook();
        fresh.files.browse = self.files.browse;
        fresh.files.quit_confirmed = false;
        if !warnings.is_empty() {
            fresh.notice = Some(warnings.join(" "));
        }
        *self = fresh;
    }

    // ----------------------------------------------------------- saving

    /// Saves to the file the workbook came from. Fails if it has none yet.
    pub fn save(&mut self) -> Result<(), String> {
        match self.doc.path.clone() {
            Some(path) => self.write_to(&path),
            None => Err(tr!("the workbook has no file yet: use Save As").to_owned()),
        }
    }

    /// Saves to another file. The copy gets a new workbook id and the trust of
    /// the original, because the user made it.
    pub fn save_as(&mut self, path: &Path) -> Result<(), String> {
        let path = with_extension(path);
        if self.doc.path.as_deref() == Some(path.as_path()) {
            return self.write_to(&path);
        }
        let old = std::mem::replace(&mut self.doc.workbook_id, new_workbook_id());
        match self.write_to(&path) {
            Ok(()) => {
                // A trusted copy was recorded as it was written; one that is
                // not trusted keeps what was recorded for the original.
                if !self.calc().is_trusted() {
                    let new = self.doc.workbook_id.clone();
                    self.change_trust(|t| {
                        t.carry(&old, &new);
                    });
                }
                Ok(())
            }
            Err(e) => {
                self.doc.workbook_id = old;
                Err(e)
            }
        }
    }

    fn write_to(&mut self, path: &Path) -> Result<(), String> {
        let (bytes, meta) = self.workbook_bytes()?;
        save_atomic(path, &bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        self.saved(meta, path);
        self.remember_recent(path);
        Ok(())
    }

    /// Saves through the host, which writes the file the workbook came from
    /// (or asks where, for a new one).
    pub fn save_to_host(&mut self, host: &dyn crate::host::Host) -> Result<(), String> {
        let (bytes, meta) = self.workbook_bytes()?;
        // A new workbook is offered under the name Save As would suggest.
        let name = match self.doc.path {
            Some(_) => self.file_name(),
            None => format!("Untitled.{EXTENSION}"),
        };
        host.save_workbook(&name, bytes);
        self.saved(meta, Path::new(&name));
        Ok(())
    }

    /// The workbook as a save would write it, without counting it as saved:
    /// for a host's backup of unsaved changes (VS Code's hot exit).
    pub fn backup_bytes(&mut self) -> Result<Vec<u8>, String> {
        self.workbook_bytes().map(|(bytes, _)| bytes)
    }

    /// The workbook as its file holds it, with the view state that goes in it.
    fn workbook_bytes(&mut self) -> Result<(Vec<u8>, Meta), String> {
        // What was typed in the code editor is saved too.
        self.commit_code();
        // Saving waits for what is still being calculated.
        if !self.session.wait_idle(Duration::from_secs(60)) {
            return Err(tr!("the workbook is still calculating; try again in a moment").to_owned());
        }
        let meta = Meta {
            workbook_id: self.doc.workbook_id.clone(),
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
            views: self.view_state(),
            workbook_view: self.workbook_view(),
            unknown: self.doc.unknown.clone(),
        };
        // The report being shown keeps its run in its results cache.
        self.session.calc_mut().take_report_run();
        let package = self.calc().to_package(&meta).map_err(|e| e.to_string())?;
        let bytes = write_zip(&package)?;
        Ok((bytes, meta))
    }

    /// What follows a save: the workbook is as saved, and trusted as it is
    /// if it is trusted.
    fn saved(&mut self, meta: Meta, path: &Path) {
        self.session.calc_mut().mark_saved();
        self.doc.saved_forms = designs_version(self.calc());
        self.doc.views = meta.views;
        self.doc.workbook_view = meta.workbook_view;
        self.doc.path = Some(path.to_path_buf());
        // What the user saved is what they trust, if they trust the workbook.
        if self.calc().is_trusted() {
            let content = self.calc().fingerprint();
            let id = self.doc.workbook_id.clone();
            self.change_trust(|t| t.trust(&id, content));
        }
    }

    // ------------------------------------------------------------ user

    /// Changes the trust record on disk. If it cannot be written the change
    /// still holds for this session, and the notice says so.
    pub(crate) fn change_trust(&mut self, change: impl FnOnce(&mut TrustStore)) {
        let id = self.doc.workbook_id.clone();
        let path = self.doc.path.clone();
        let seen = path.as_deref().map(|p| (id.as_str(), p));
        if let Err(e) = self.user.change_trust_seen(change, seen) {
            self.notice = Some(tr!(
                "The trust decision holds until Gossia ends, but it could not be kept: {e}",
                e = e
            ));
        }
    }

    pub(crate) fn change_prefs(&mut self, change: impl FnOnce(&mut crate::prefs::Preferences)) {
        if let Err(e) = self.user.change_prefs(change) {
            self.notice = Some(tr!("The preferences could not be saved: {e}", e = e));
        }
    }

    fn remember_recent(&mut self, path: &Path) {
        let path = path.to_path_buf();
        self.change_prefs(|p| p.add_recent(&path));
    }

    // --------------------------------------------------------- commands

    /// File > Open: asks first about unsaved changes.
    pub fn begin_open(&mut self) {
        self.commit_code();
        if self.is_dirty() {
            self.files.confirm = Some(Pending::Open);
        } else {
            self.show_open_dialog();
        }
    }

    /// File > Open Recent: asks first about unsaved changes.
    pub fn begin_open_recent(&mut self, path: &Path) {
        self.commit_code();
        if self.is_dirty() {
            self.files.confirm = Some(Pending::OpenRecent(path.to_path_buf()));
        } else {
            self.open_recent(path);
        }
    }

    /// File > New: asks first about unsaved changes.
    pub fn begin_new(&mut self) {
        self.commit_code();
        if self.is_dirty() {
            self.files.confirm = Some(Pending::New);
        } else {
            self.new_workbook();
        }
    }

    /// The folder the browser starts in: where the workbook is, or here.
    pub(crate) fn start_folder(&self) -> std::path::PathBuf {
        self.doc
            .path
            .as_deref()
            .and_then(Path::parent)
            .filter(|p| !p.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_default()
    }

    fn show_open_dialog(&mut self) {
        if self.files.browse {
            self.files.browser = new_browser()
                .title(tr!("Open a workbook"))
                .initial_directory(self.start_folder())
                .add_file_filter_extensions(tr!("Gossia workbooks"), vec![EXTENSION])
                .default_file_filter(tr!("Gossia workbooks"));
            self.files.browser.pick_file();
            self.files.browsing = Some(Browsing::Open);
            return;
        }
        self.files.dialog = Some(FileDialog {
            purpose: Browsing::Open,
            text: self.default_file_text(),
            error: None,
        });
    }

    /// File > Import...: a JSON, XML, CSV, TSV or text file, as tabs of this
    /// workbook.
    pub fn begin_import(&mut self) {
        // In a host, any file: the format is told from its name and text.
        if let Some(host) = crate::host::current() {
            host.pick_file(crate::host::purpose::IMPORT, &[]);
            return;
        }
        if self.files.browse {
            self.files.browser = new_browser()
                .title(tr!("Import a file"))
                .initial_directory(self.start_folder())
                .add_file_filter_extensions(
                    "JSON, XML, CSV and TSV",
                    vec!["json", "xml", "csv", "tsv", "tab"],
                )
                .add_file_filter_extensions("JSON", vec!["json"])
                .add_file_filter_extensions("XML", vec!["xml"])
                .add_file_filter_extensions("CSV and TSV", vec!["csv", "tsv", "tab"])
                .default_file_filter("JSON, XML, CSV and TSV");
            self.files.browser.pick_file();
            self.files.browsing = Some(Browsing::Import);
            return;
        }
        self.files.dialog = Some(FileDialog {
            purpose: Browsing::Import,
            text: String::new(),
            error: None,
        });
    }

    /// File > Save: saves, or asks for a name if the workbook has none.
    pub fn begin_save(&mut self) {
        if let Some(host) = crate::host::current() {
            if let Err(e) = self.save_to_host(&*host) {
                self.notice = Some(e);
            }
            return;
        }
        if self.doc.path.is_some() {
            if let Err(e) = self.save() {
                self.notice = Some(e);
            }
        } else {
            self.begin_save_as();
        }
    }

    pub fn begin_save_as(&mut self) {
        if self.files.browse {
            let name = if self.doc.path.is_some() {
                self.file_name()
            } else {
                format!("Untitled.{EXTENSION}")
            };
            self.files.browser = new_browser()
                .title(tr!("Save the workbook as"))
                .initial_directory(self.start_folder())
                .default_file_name(&name)
                .add_save_extension(tr!("Gossia workbook"), EXTENSION)
                .default_save_extension(tr!("Gossia workbook"));
            self.files.browser.save_file();
            self.files.browsing = Some(Browsing::SaveAs);
            return;
        }
        self.files.dialog = Some(FileDialog {
            purpose: Browsing::SaveAs,
            text: self.default_file_text(),
            error: None,
        });
    }

    fn default_file_text(&self) -> String {
        match &self.doc.path {
            Some(path) => path.display().to_string(),
            None => format!("Untitled.{EXTENSION}"),
        }
    }

    /// File > Quit, or the window being closed: asks first about unsaved
    /// changes.
    pub fn request_quit(&mut self, ctx: &egui::Context) {
        self.commit_code();
        if self.is_dirty() && !self.files.quit_confirmed {
            self.files.confirm = Some(Pending::Quit);
        } else {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    /// Carries out the dialog that is open.
    pub fn apply_file_dialog(&mut self) {
        let Some(mut dialog) = self.files.dialog.take() else {
            return;
        };
        let path = PathBuf::from(dialog.text.trim());
        if dialog.text.trim().is_empty() {
            dialog.error = Some(tr!("Give the name of a file.").to_owned());
            self.files.dialog = Some(dialog);
            return;
        }
        let done = match dialog.purpose {
            Browsing::SaveAs => self.save_as(&path),
            Browsing::Open => self.open_path(&path),
            Browsing::Import => self.begin_import_path(&path),
            Browsing::Export => self.export_to(&path),
            Browsing::ExportPdf | Browsing::ExportHtml => self.export_report_to(&path),
        };
        if let Err(e) = done {
            dialog.error = Some(e);
            self.files.dialog = Some(dialog);
        }
    }

    /// Answers "Save the changes?": `save` keeps them, otherwise they are lost.
    pub fn answer_unsaved(&mut self, ctx: &egui::Context, save: bool) {
        let Some(pending) = self.files.confirm.take() else {
            return;
        };
        if save {
            if self.doc.path.is_none() {
                // A name is needed first; the question is asked again after.
                self.files.confirm = Some(pending);
                self.begin_save_as();
                return;
            }
            if let Err(e) = self.save() {
                self.notice = Some(e);
                return;
            }
        }
        match pending {
            Pending::Quit => {
                self.files.quit_confirmed = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Pending::Open => self.show_open_dialog(),
            Pending::OpenRecent(path) => self.open_recent(&path),
            Pending::New => self.new_workbook(),
        }
    }

    // ---------------------------------------------------------- windows

    /// The dialog for a file name, and the question about unsaved changes.
    pub(crate) fn file_windows(&mut self, ctx: &egui::Context) {
        self.file_browser(ctx);
        if let Some(mut dialog) = self.files.dialog.take() {
            let (mut apply, mut cancel) = (false, false);
            let mut open = true;
            egui::Window::new(match dialog.purpose {
                Browsing::SaveAs => tr!("Save As"),
                Browsing::Open => tr!("Open a workbook"),
                Browsing::Import => tr!("Import a file"),
                Browsing::Export => tr!("Export"),
                Browsing::ExportPdf => tr!("Export as PDF"),
                Browsing::ExportHtml => tr!("Export as HTML"),
            })
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(tr!("The file, with its folder:"));
                let response =
                    ui.add(egui::TextEdit::singleline(&mut dialog.text).desired_width(420.0));
                named(&response, tr!("File name"));
                if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    apply = true;
                }
                if let Some(error) = &dialog.error {
                    ui.colored_label(crate::view::error_color(ui), error);
                }
                ui.horizontal(|ui| {
                    apply |= ui
                        .button(match dialog.purpose {
                            Browsing::SaveAs => tr!("Save"),
                            Browsing::Open => tr!("Open"),
                            Browsing::Import => tr!("Import"),
                            Browsing::Export | Browsing::ExportPdf | Browsing::ExportHtml => {
                                tr!("Export")
                            }
                        })
                        .clicked();
                    cancel = ui.button(tr!("Cancel")).clicked();
                });
            });
            if cancel || !open {
                // Leaving the dialog also leaves what was waiting for it.
                self.files.confirm = None;
            } else {
                self.files.dialog = Some(dialog);
                if apply {
                    self.apply_file_dialog();
                }
            }
        }
        if let Some(pending) = self.files.confirm.clone() {
            // A name is being asked for first: that dialog is the question.
            if self.files.dialog.is_some() || self.files.browsing.is_some() {
                return;
            }
            let (mut save, mut discard, mut cancel) = (false, false, false);
            let mut open = true;
            egui::Window::new(tr!("Unsaved changes"))
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    let file = self.file_name();
                    ui.label(match pending {
                        Pending::Quit => {
                            tr!("Save the changes to {file} before quitting?", file = file)
                        }
                        Pending::Open | Pending::OpenRecent(_) => tr!(
                            "Save the changes to {file} before opening another workbook?",
                            file = file
                        ),
                        Pending::New => tr!(
                            "Save the changes to {file} before starting a new workbook?",
                            file = file
                        ),
                    });
                    ui.horizontal(|ui| {
                        save = ui.button(tr!("Save")).clicked();
                        discard = ui.button(tr!("Don't save")).clicked();
                        cancel = ui.button(tr!("Cancel")).clicked();
                    });
                });
            if save {
                self.answer_unsaved(ctx, true);
            } else if discard {
                self.answer_unsaved(ctx, false);
            } else if cancel || !open {
                self.files.confirm = None;
            }
        }
    }

    /// The file browser, while it is open, and what comes of its answer: the
    /// chosen path is opened or saved to, and if that fails the reason is shown
    /// in the text dialog, with the path as it was chosen.
    fn file_browser(&mut self, ctx: &egui::Context) {
        let Some(purpose) = self.files.browsing else {
            return;
        };
        self.files.browser.update(ctx);
        if let Some(path) = self.files.browser.take_picked() {
            self.files.browsing = None;
            let done = match purpose {
                Browsing::Open => self.open_path(&path),
                Browsing::SaveAs => self.save_as(&path),
                Browsing::Import => self.begin_import_path(&path),
                Browsing::Export => self.export_to(&path),
                Browsing::ExportPdf | Browsing::ExportHtml => self.export_report_to(&path),
            };
            if let Err(error) = done {
                self.files.dialog = Some(FileDialog {
                    purpose,
                    text: path.display().to_string(),
                    error: Some(error),
                });
            }
        } else if matches!(
            self.files.browser.state(),
            DialogState::Closed | DialogState::Cancelled
        ) {
            // Leaving the browser also leaves what was waiting for it.
            self.files.browsing = None;
            self.files.confirm = None;
        }
    }

    /// Gives the window its title, when it has changed.
    pub(crate) fn update_title(&mut self, ctx: &egui::Context) {
        let title = self.title();
        if title != self.files.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.files.title = title;
            // The host shows its own mark of unsaved changes.
            if let Some(host) = crate::host::current() {
                host.set_dirty(self.is_dirty());
            }
        }
    }

    /// Asks to close the window while there are unsaved changes.
    pub(crate) fn handle_close_request(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested()) {
            self.commit_code();
        }
        if ctx.input(|i| i.viewport().close_requested())
            && self.is_dirty()
            && !self.files.quit_confirmed
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.files.confirm = Some(Pending::Quit);
        }
    }
}

/// The path with the extension of a workbook, if it has none.
fn with_extension(path: &Path) -> PathBuf {
    if path.extension().is_some() {
        path.to_path_buf()
    } else {
        path.with_extension(EXTENSION)
    }
}

/// What File > New makes: a data tab, `Data`, bound to its schema tab,
/// `DataSchema`, whose record has no fields yet. Auto-update is on, so the
/// fields are added from the data tab.
pub fn blank_workbook() -> Calc {
    use gossia_core::{json, jsonschema};
    const SCHEMA: &str = r#"{"type": "object", "properties": {}}"#;
    let mut calc = Calc::new(Arc::new(PythonSyntax));
    calc.add_sheet(
        "Data",
        jsonschema::lower_str(SCHEMA).expect("the blank schema is valid"),
        json::parse("{}").expect("the blank data is JSON").node,
    )
    .expect("a new workbook has no tabs");
    calc.add_schema_sheet(
        "DataSchema",
        json::parse(SCHEMA).expect("the blank schema is JSON").node,
    )
    .expect("the tabs have different names");
    calc.bind_schema("Data", "DataSchema")
        .expect("the blank schema can be used");
    calc.set_auto_update("DataSchema", true)
        .expect("one tab uses the schema");
    calc.clear_history();
    calc.mark_saved();
    calc
}
