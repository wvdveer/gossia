//! Checking a module and moving it in and out of files, in the code editor
//! (DESIGN.md, "Python modules"): *Debug > Compile* lists what is wrong with
//! the module shown, each problem a link to its line; *File > Export* writes a
//! module as a plain `.py` file and *File > Import module...* reads one in as a
//! new module, the way Excel exports and imports a `.bas`.

use std::path::{Path, PathBuf};

use egui::Key;
use egui_file_dialog::{DialogState, FileDialog as Browser};

use crate::GossiaApp;
use crate::view::{error_color, named};
use crate::workbook_file::new_browser;
use gossia_calc::ModuleStatus;

/// The extension of an exported module.
pub const PY: &str = "py";

/// What a `.py` file is chosen for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModuleFile {
    /// Writing this module to a file.
    Export(String),
    /// Reading a file in as a new module.
    Import,
}

/// The dialog for a file name, where the browser is off (the tests) or its
/// choice could not be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleFileDialog {
    pub purpose: ModuleFile,
    pub text: String,
    pub error: Option<String>,
}

/// The file browser of the code editor's window, and its dialog.
#[derive(Default)]
pub struct ModuleFiles {
    pub browser: Option<(Browser, ModuleFile)>,
    pub dialog: Option<ModuleFileDialog>,
}

impl GossiaApp {
    // ------------------------------------------------------------ compile

    /// *Debug > Compile*: puts what was typed in the workbook, and lists the
    /// problems of the module, which stay listed, and up to date, until the
    /// list is closed.
    pub fn compile_module(&mut self, module: &str) {
        self.commit_code();
        self.code.notice = None;
        self.code.compiled = Some(module.to_owned());
    }

    /// The list of *Compile*: a click on a problem shows its place.
    pub(crate) fn compile_pane(&mut self, ui: &mut egui::Ui) {
        let Some(module) = self.code.compiled.clone() else {
            return;
        };
        let problems = self.calc().module_problems(&module).unwrap_or_default();
        let status = self.calc().module_status(&module).cloned();
        let mut close = false;
        ui.horizontal(|ui| {
            let summary = match (problems.len(), &status) {
                (0, Some(ModuleStatus::Pending)) => "loading...".to_owned(),
                (0, Some(ModuleStatus::Untrusted)) => {
                    "no problems found without running it (the workbook is not trusted, so \
                     it is not loaded)"
                        .to_owned()
                }
                (0, _) => "no problems".to_owned(),
                (1, _) => "1 problem".to_owned(),
                (n, _) => format!("{n} problems"),
            };
            ui.strong(format!("Compile {module}: {summary}"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                close = crate::icons::button(ui, true, crate::icons::X, "x", "Close the list")
                    .clicked();
            });
        });
        let mut go = None;
        egui::ScrollArea::vertical()
            .id_salt("compile_list")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for problem in &problems {
                    let place = match (problem.line, problem.column) {
                        (Some(line), Some(column)) => format!("Line {line}, column {column}"),
                        (Some(line), None) => format!("Line {line}"),
                        _ => "Loading".to_owned(),
                    };
                    let text = egui::RichText::new(format!("{place}: {}", problem.message))
                        .color(error_color(ui));
                    let response = ui.add(egui::Button::new(text).frame(false));
                    if response.clicked() {
                        go = Some((
                            problem.line.unwrap_or(1).saturating_sub(1),
                            problem.column.unwrap_or(1).saturating_sub(1),
                        ));
                    }
                }
            });
        if close {
            self.code.compiled = None;
        } else if let Some(place) = go {
            self.open_module(&module);
            self.code.jump = Some(place);
        }
    }

    // -------------------------------------------------------------- files

    /// *File > Export*: asks where to write the module.
    pub fn begin_export_module(&mut self, module: &str) {
        self.commit_code();
        if let Some(host) = crate::host::current() {
            match self.calc().module_source(module) {
                Some(source) => {
                    let name = format!("{module}.{PY}");
                    host.save_file(&name, source.as_bytes().to_vec());
                    self.code.notice = None;
                }
                None => self.code.notice = Some(format!("there is no module called {module}")),
            }
            return;
        }
        let purpose = ModuleFile::Export(module.to_owned());
        if self.files.browse {
            let mut browser = new_browser()
                .title(&format!("Export module {module}"))
                .initial_directory(self.start_folder())
                .default_file_name(&format!("{module}.{PY}"))
                .add_save_extension("Python files", PY)
                .default_save_extension("Python files");
            browser.save_file();
            self.code.files.browser = Some((browser, purpose));
            return;
        }
        let text = self.start_folder().join(format!("{module}.{PY}"));
        self.code.files.dialog = Some(ModuleFileDialog {
            purpose,
            text: text.display().to_string(),
            error: None,
        });
    }

    /// *File > Import module...*: asks for the `.py` file to read.
    pub fn begin_import_module(&mut self) {
        self.commit_code();
        if let Some(host) = crate::host::current() {
            host.pick_file(crate::host::purpose::IMPORT_MODULE, &[PY]);
            return;
        }
        if self.files.browse {
            let mut browser = new_browser()
                .title("Import a module")
                .initial_directory(self.start_folder())
                .add_file_filter_extensions("Python files", vec![PY])
                .default_file_filter("Python files");
            browser.pick_file();
            self.code.files.browser = Some((browser, ModuleFile::Import));
            return;
        }
        self.code.files.dialog = Some(ModuleFileDialog {
            purpose: ModuleFile::Import,
            text: String::new(),
            error: None,
        });
    }

    /// Writes the module's source to a file, as UTF-8 with the line endings
    /// it has. `.py` is added to a name without an extension.
    pub fn export_module_to(&mut self, module: &str, path: &Path) -> Result<PathBuf, String> {
        self.commit_code();
        let source = self
            .calc()
            .module_source(module)
            .ok_or_else(|| format!("there is no module called {module}"))?
            .to_owned();
        let path = if path.extension().is_some() {
            path.to_path_buf()
        } else {
            path.with_extension(PY)
        };
        crate::file::save_atomic(&path, source.as_bytes())
            .map_err(|e| format!("{}: {e}", path.display()))?;
        self.code.notice = None;
        self.code.info = Some(format!("{module} exported to {}", path.display()));
        Ok(path)
    }

    /// Reads a `.py` file in as a new module after the others, named after
    /// the file, and opens it: one entry in the history. A name that cannot
    /// be used as it is gets a number, as Excel does (`Tax` becomes `Tax2`
    /// while there is a `Tax`).
    pub fn import_module_from(&mut self, path: &Path) -> Result<String, String> {
        self.commit_code();
        let shown = path.display();
        let size = std::fs::metadata(path)
            .map_err(|e| format!("{shown}: {e}"))?
            .len();
        let limit = self.user.prefs.max_import_bytes;
        if size > limit {
            return Err(format!(
                "{shown} is {size} bytes, more than the largest file imported ({limit} bytes; \
                 Tools > Options, Files and import)"
            ));
        }
        let bytes = std::fs::read(path).map_err(|e| format!("{shown}: {e}"))?;
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.add_imported_module(&shown.to_string(), &stem, bytes)
    }

    /// Reads a `.py` file that came through the host in as a new module, as
    /// [`GossiaApp::import_module_from`] does.
    pub fn import_module_bytes(&mut self, file: &str, bytes: &[u8]) -> Result<String, String> {
        self.commit_code();
        let limit = self.user.prefs.max_import_bytes;
        if bytes.len() as u64 > limit {
            return Err(format!(
                "{file} is {} bytes, more than the largest file imported ({limit} bytes; \
                 Tools > Options, Files and import)",
                bytes.len()
            ));
        }
        let stem = file.rsplit_once('.').map_or(file, |(stem, _)| stem);
        self.add_imported_module(file, stem, bytes.to_vec())
    }

    /// The module of an imported file, named after it, opened.
    fn add_imported_module(
        &mut self,
        shown: &str,
        stem: &str,
        bytes: Vec<u8>,
    ) -> Result<String, String> {
        let text = String::from_utf8(bytes)
            .map_err(|_| format!("{shown} is not UTF-8 text, so it cannot be a module"))?;
        let source = text
            .strip_prefix('\u{feff}')
            .unwrap_or(&text)
            .replace("\r\n", "\n");
        let name = self.import_name(stem);
        self.session
            .calc_mut()
            .add_module(&name, &source)
            .map_err(|e| e.to_string())?;
        self.code.notice = None;
        self.code.info = Some(format!("{shown} imported as module {name}"));
        self.open_module(&name);
        Ok(name)
    }

    /// The name of a module read from a file called `stem`: its letters,
    /// digits and underscores, with a number after it when the name is taken
    /// or reserved.
    fn import_name(&self, stem: &str) -> String {
        let mut base: String = stem
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        if base.is_empty() || base.starts_with(|c: char| c.is_ascii_digit()) {
            base = format!("Module_{base}");
        }
        // A number does not help a name kept for internal use (`_x`,
        // `gossia...`): `Module_` before it does.
        let calc = self.calc();
        [base.clone(), format!("Module_{base}")]
            .into_iter()
            .flat_map(|base| {
                std::iter::once(base.clone()).chain((2..1_000).map(move |n| format!("{base}{n}")))
            })
            .find(|name| calc.module_name_problem(name, None).is_none())
            .unwrap_or_else(|| calc.new_module_name())
    }

    /// Does what the file was chosen for.
    fn use_module_file(&mut self, purpose: &ModuleFile, path: &Path) -> Result<(), String> {
        match purpose {
            ModuleFile::Export(module) => self.export_module_to(module, path).map(|_| ()),
            ModuleFile::Import => self.import_module_from(path).map(|_| ()),
        }
    }

    /// The file browser and the file name dialog of the code editor, drawn in
    /// its window.
    pub(crate) fn module_file_windows(&mut self, ctx: &egui::Context) {
        if let Some((mut browser, purpose)) = self.code.files.browser.take() {
            browser.update(ctx);
            if let Some(path) = browser.take_picked() {
                if let Err(error) = self.use_module_file(&purpose, &path) {
                    self.code.files.dialog = Some(ModuleFileDialog {
                        purpose,
                        text: path.display().to_string(),
                        error: Some(error),
                    });
                }
            } else if !matches!(
                browser.state(),
                DialogState::Closed | DialogState::Cancelled
            ) {
                self.code.files.browser = Some((browser, purpose));
            }
        }
        self.module_file_dialog(ctx);
    }

    fn module_file_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.code.files.dialog.take() else {
            return;
        };
        let title = match &dialog.purpose {
            ModuleFile::Export(module) => format!("Export module {module}"),
            ModuleFile::Import => "Import a module".to_owned(),
        };
        let mut open = true;
        let (mut apply, mut cancel) = (false, false);
        egui::Window::new(title)
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label("File");
                let response =
                    ui.add(egui::TextEdit::singleline(&mut dialog.text).desired_width(360.0));
                named(&response, "Module file");
                if response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    apply = true;
                }
                if ui.input(|i| i.key_pressed(Key::Escape)) {
                    cancel = true;
                }
                if let Some(why) = &dialog.error {
                    ui.colored_label(error_color(ui), why);
                }
                ui.horizontal(|ui| {
                    let label = match dialog.purpose {
                        ModuleFile::Export(_) => "Export",
                        ModuleFile::Import => "Import",
                    };
                    apply |= ui
                        .add_enabled(!dialog.text.trim().is_empty(), egui::Button::new(label))
                        .clicked();
                    cancel |= ui.button("Cancel").clicked();
                });
            });
        if apply {
            let path = PathBuf::from(dialog.text.trim());
            if let Err(error) = self.use_module_file(&dialog.purpose, &path) {
                dialog.error = Some(error);
                self.code.files.dialog = Some(dialog);
            }
        } else if open && !cancel {
            self.code.files.dialog = Some(dialog);
        }
    }
}
