//! Opening and saving workbooks: the file the window is showing, the File menu,
//! the dialogs it opens, and what is asked when there are unsaved changes
//! (DESIGN.md, "File format", "Trust model").
//!
//! Deciding what is in a file is `cliotree-calc`'s `Calc::from_package`; the zip
//! and the disk are in [`crate::file`]. This is the part that joins them to the
//! window, and decides whether a file that was opened is trusted.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use cliotree_calc::python::{PythonSettings, PythonSyntax};
use cliotree_calc::{Calc, Decision, Evaluator, Meta, Opened, Session, TrustStore, python};
use egui_file_dialog::{DialogState, FileDialog as Browser, FileDialogLabels};
use serde_json::Value as Json;

use crate::app::ClioApp;
use crate::file::{Limits, read_zip, save_atomic, write_zip};
use crate::trust_ui::TrustBanner;
use crate::view::named;

/// A file browser that draws what the window's fonts have: the Phosphor icons
/// instead of the emoji and symbols it was written with.
fn new_browser() -> Browser {
    use crate::icons as i;
    let with = |icon: &str, text: &str| format!("{icon}  {text}");
    let labels = FileDialogLabels {
        title_select_directory: with(i::FOLDER, "Select Folder"),
        title_select_file: with(i::FOLDER_OPEN, "Open File"),
        title_select_multiple: with(i::FILE_TEXT, "Select Multiple"),
        title_save_file: with(i::FLOPPY, "Save File"),
        reload: with(i::RELOAD, "Reload"),
        working_directory: with(i::ARROW_UP_RIGHT, "Go to working directory"),
        select_all: with(i::CHECK_SQUARE, "Select all"),
        home_dir: with(i::HOUSE, "Home"),
        desktop_dir: with(i::DESKTOP, "Desktop"),
        documents_dir: with(i::FILE_TEXT, "Documents"),
        downloads_dir: with(i::DOWNLOAD, "Downloads"),
        audio_dir: with(i::MUSIC, "Audio"),
        pictures_dir: with(i::IMAGE, "Pictures"),
        videos_dir: with(i::FILM, "Videos"),
        pin_folder: with(i::PIN, "Pin"),
        unpin_folder: with(i::X, "Unpin"),
        rename_pinned_folder: with(i::PENCIL, "Rename"),
        open_button: with(i::FOLDER_OPEN, "Open"),
        save_button: with(i::FLOPPY, "Save"),
        cancel_button: with(i::PROHIBIT, "Cancel"),
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
pub const EXTENSION: &str = "clio";

/// The file the window is showing, and what goes with it.
pub struct Document {
    /// Where it is saved, if it has been.
    pub path: Option<PathBuf>,
    /// The id the trust model knows the workbook by.
    pub workbook_id: String,
    /// What a newer version put in the file and this one does not know: kept,
    /// and written back unchanged.
    pub unknown: Vec<(String, Vec<u8>)>,
    /// The state of the view of each tab, as it was in the file.
    pub views: BTreeMap<String, Json>,
    /// The workbooks the user has trusted (kept in memory until the
    /// preferences can keep them on disk).
    pub trust: TrustStore,
    /// What a file may hold when it is read.
    pub limits: Limits,
    /// The version of the forms as they were saved.
    saved_forms: u64,
}

impl Default for Document {
    fn default() -> Document {
        Document {
            path: None,
            workbook_id: new_workbook_id(),
            unknown: Vec::new(),
            views: BTreeMap::new(),
            trust: TrustStore::default(),
            limits: Limits::default(),
            saved_forms: 0,
        }
    }
}

/// A random id for a workbook. It is not a secret: the trust model needs the
/// content to match as well.
pub fn new_workbook_id() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let part = |salt: u128| {
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u128(nanos ^ salt);
        hasher.finish()
    };
    format!("{:016x}{:016x}", part(1), part(2))
}

/// What the user was doing when the unsaved changes came up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pending {
    /// Leaving the application.
    Quit,
    /// Choosing a file to open.
    Open,
}

/// The dialog for a file name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDialog {
    pub save: bool,
    pub text: String,
    pub error: Option<String>,
}

/// What the file browser was opened for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Browsing {
    Open,
    SaveAs,
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

impl ClioApp {
    // ------------------------------------------------------------ state

    /// The name of the file, or `Untitled`.
    pub fn file_name(&self) -> String {
        self.doc
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .map_or_else(
                || "Untitled".to_owned(),
                |name| name.to_string_lossy().into_owned(),
            )
    }

    /// Whether there are changes that a save would keep: edits of the data,
    /// and changes to the forms.
    pub fn is_dirty(&self) -> bool {
        self.calc().is_modified() || self.calc().forms_version() != self.doc.saved_forms
    }

    /// The title of the window.
    pub fn title(&self) -> String {
        format!(
            "{}{} - ClioTree",
            self.file_name(),
            if self.is_dirty() { " *" } else { "" }
        )
    }

    // ---------------------------------------------------------- opening

    /// Opens a workbook, replacing the one shown. The caller has dealt with
    /// unsaved changes. Nothing changes if the file cannot be read.
    pub fn open_path(&mut self, path: &Path) -> Result<(), String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let package = read_zip(&bytes, &self.doc.limits)?;
        let opened =
            Calc::from_package(Arc::new(PythonSyntax), &package).map_err(|e| e.to_string())?;
        self.install(opened, Some(path.to_path_buf()));
        Ok(())
    }

    /// Shows a workbook that was read. Whether it runs Python is decided here:
    /// a file whose executable content is as the user trusted it, or that has
    /// none, is trusted; any other is not, and the banner asks.
    fn install(&mut self, opened: Opened, path: Option<PathBuf>) {
        let Opened {
            mut calc,
            workbook_id,
            views,
            unknown,
            warnings,
            ..
        } = opened;
        let content = calc.fingerprint();
        let decision = if content.items.is_empty() {
            Decision::Trusted
        } else {
            self.doc.trust.decide(&workbook_id, &content)
        };
        let (trusted, changes) = match decision {
            Decision::Trusted => (true, Vec::new()),
            Decision::Unknown => (false, Vec::new()),
            Decision::Changed(changes) => (false, changes),
        };
        if !trusted {
            calc.set_trusted(false);
        }
        let evaluator = Evaluator::new(
            python::factory(PythonSettings::default()),
            self.session.evaluator().settings().clone(),
        );
        let mut fresh = ClioApp::build(Session::new(calc, evaluator), trusted);
        fresh.doc = Document {
            path,
            workbook_id,
            unknown,
            views,
            trust: std::mem::take(&mut self.doc.trust),
            limits: self.doc.limits,
            saved_forms: fresh.calc().forms_version(),
        };
        if !trusted {
            fresh.trust = TrustBanner {
                changes,
                dismissed: false,
            };
        }
        fresh.show_issues = self.show_issues;
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
            None => Err("the workbook has no file yet: use Save As".to_owned()),
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
        self.doc.trust.carry(&old, &self.doc.workbook_id);
        match self.write_to(&path) {
            Ok(()) => Ok(()),
            Err(e) => {
                self.doc.workbook_id = old;
                Err(e)
            }
        }
    }

    fn write_to(&mut self, path: &Path) -> Result<(), String> {
        // Saving waits for what is still being calculated.
        if !self.session.wait_idle(Duration::from_secs(60)) {
            return Err("the workbook is still calculating; try again in a moment".to_owned());
        }
        let meta = Meta {
            workbook_id: self.doc.workbook_id.clone(),
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
            views: self.doc.views.clone(),
            unknown: self.doc.unknown.clone(),
        };
        let package = self.calc().to_package(&meta).map_err(|e| e.to_string())?;
        let bytes = write_zip(&package)?;
        save_atomic(path, &bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        self.session.calc_mut().mark_saved();
        self.doc.saved_forms = self.calc().forms_version();
        // What the user saved is what they trust, if they trust the workbook.
        if self.calc().is_trusted() {
            let content = self.calc().fingerprint();
            self.doc.trust.trust(&self.doc.workbook_id, content);
        }
        self.doc.path = Some(path.to_path_buf());
        Ok(())
    }

    // --------------------------------------------------------- commands

    /// File > Open: asks first about unsaved changes.
    pub fn begin_open(&mut self) {
        if self.is_dirty() {
            self.files.confirm = Some(Pending::Open);
        } else {
            self.show_open_dialog();
        }
    }

    /// The folder the browser starts in: where the workbook is, or here.
    fn start_folder(&self) -> std::path::PathBuf {
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
                .title("Open a workbook")
                .initial_directory(self.start_folder())
                .add_file_filter_extensions("ClioTree workbooks", vec![EXTENSION])
                .default_file_filter("ClioTree workbooks");
            self.files.browser.pick_file();
            self.files.browsing = Some(Browsing::Open);
            return;
        }
        self.files.dialog = Some(FileDialog {
            save: false,
            text: self.default_file_text(),
            error: None,
        });
    }

    /// File > Save: saves, or asks for a name if the workbook has none.
    pub fn begin_save(&mut self) {
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
                .title("Save the workbook as")
                .initial_directory(self.start_folder())
                .default_file_name(&name)
                .add_save_extension("ClioTree workbook", EXTENSION)
                .default_save_extension("ClioTree workbook");
            self.files.browser.save_file();
            self.files.browsing = Some(Browsing::SaveAs);
            return;
        }
        self.files.dialog = Some(FileDialog {
            save: true,
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
            dialog.error = Some("Give the name of a file.".to_owned());
            self.files.dialog = Some(dialog);
            return;
        }
        let done = if dialog.save {
            self.save_as(&path)
        } else {
            self.open_path(&path)
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
        }
    }

    // ---------------------------------------------------------- windows

    /// The dialog for a file name, and the question about unsaved changes.
    pub(crate) fn file_windows(&mut self, ctx: &egui::Context) {
        self.file_browser(ctx);
        if let Some(mut dialog) = self.files.dialog.take() {
            let (mut apply, mut cancel) = (false, false);
            let mut open = true;
            egui::Window::new(if dialog.save {
                "Save As"
            } else {
                "Open a workbook"
            })
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label("The file, with its folder:");
                let response =
                    ui.add(egui::TextEdit::singleline(&mut dialog.text).desired_width(420.0));
                named(&response, "File name");
                if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    apply = true;
                }
                if let Some(error) = &dialog.error {
                    ui.colored_label(crate::view::error_color(ui), error);
                }
                ui.horizontal(|ui| {
                    apply |= ui
                        .button(if dialog.save { "Save" } else { "Open" })
                        .clicked();
                    cancel = ui.button("Cancel").clicked();
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
            egui::Window::new("Unsaved changes")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label(format!(
                        "Save the changes to {} before {}?",
                        self.file_name(),
                        match pending {
                            Pending::Quit => "quitting",
                            Pending::Open => "opening another workbook",
                        }
                    ));
                    ui.horizontal(|ui| {
                        save = ui.button("Save").clicked();
                        discard = ui.button("Don't save").clicked();
                        cancel = ui.button("Cancel").clicked();
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
            };
            if let Err(error) = done {
                self.files.dialog = Some(FileDialog {
                    save: purpose == Browsing::SaveAs,
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
        }
    }

    /// Asks to close the window while there are unsaved changes.
    pub(crate) fn handle_close_request(&mut self, ctx: &egui::Context) {
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
