//! The code editor (DESIGN.md, "Python modules"): a second window, opened with
//! *Tools > Code editor* or Alt+F11, as Excel opens its Visual Basic editor. The
//! project explorer lists the workbook's modules and its customised forms; each
//! open item has a tab with line numbers, highlighting and automatic
//! indentation; the procedure box jumps to a function. *Compile* and the `.py`
//! files are in `code_check.rs`, the Immediate window in `immediate.rs`.
//!
//! **Typing and the history.** The text of each open module is the editor's
//! own while it is typed. It goes to the workbook, as one undoable entry, once
//! typing has paused for [`PAUSE`] seconds, and at once before anything that
//! needs the workbook as it is: undo and redo, saving, opening, closing, and
//! the module commands. So a burst of typing is one entry, and editing a
//! module does not reload it at every key. Undo in this window is the
//! workbook's, not a text box's own, so there is one history.
//!
//! **Forms.** The design of a customised form is shown as the JSON it is
//! stored as. What is typed goes in after the same pause, but only once it is
//! a design that can be used; until then the form keeps its design and the
//! status line says why. Forms are not in the workbook's history, so undo in a
//! form's tab is the text box's own.

use egui::text::{CCursor, CCursorRange};
use egui::{Key, KeyboardShortcut, Modifiers, ViewportBuilder, ViewportClass, ViewportId};

use crate::ClioApp;
use crate::code_text::{self, Edited};
use crate::tab_rename::select_all;
use crate::view::{error_color, named};
use cliotree_calc::ModuleStatus;
use cliotree_core::form::FormDesign;

/// The title of the code editor's window.
pub const TITLE: &str = "Code editor - ClioTree";

/// How long typing pauses before the text is put in the workbook, in seconds.
pub const PAUSE: f64 = 1.0;

/// Spaces per level of indentation in the JSON of a form design, which is
/// how it is written.
const FORM_INDENT: usize = 2;

/// The size the window opens at.
const SIZE: [f32; 2] = [900.0, 640.0];

/// What a tab of the editor holds.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CodeItem {
    Module(String),
    /// The design of a customised form of a tab, as its JSON text.
    Form {
        sheet: String,
        form: String,
    },
}

impl CodeItem {
    /// The name on the tab.
    pub fn title(&self) -> String {
        match self {
            CodeItem::Module(name) => name.clone(),
            CodeItem::Form { sheet, form } => format!("{sheet}: {form}"),
        }
    }

    pub fn module(&self) -> Option<&str> {
        match self {
            CodeItem::Module(name) => Some(name),
            CodeItem::Form { .. } => None,
        }
    }
}

/// What the code editor keeps between frames.
pub struct CodeEditor {
    /// Whether the window is shown.
    pub open: bool,
    /// The items open in tabs, in the order they were opened.
    pub tabs: Vec<CodeTab>,
    /// The tab shown.
    pub current: usize,
    /// The dialog of *Rename...*, while it is open.
    pub rename: Option<RenameModule>,
    /// The module *Remove...* asks about.
    pub remove: Option<String>,
    /// Why the last command was refused.
    pub notice: Option<String>,
    /// What the last command did, when it says so (an export).
    pub info: Option<String>,
    /// Put the cursor at this line and column (0-based) of the tab shown, and
    /// scroll to it.
    pub jump: Option<(usize, usize)>,
    /// The module whose problems *Compile* lists, while the list is shown.
    pub compiled: Option<String>,
    /// The `.py` file being chosen, and its dialog.
    pub files: crate::code_check::ModuleFiles,
    /// The Immediate window.
    pub immediate: crate::immediate::ImmediateUi,
    /// The version of the forms the open form tabs were last compared with.
    forms_seen: u64,
    /// Bring the window to the front at the next frame.
    focus: bool,
    /// Whether the window was drawn in the last frame. A window's first frame
    /// only measures it, and a text box in it cannot take the focus yet.
    pub(crate) drawn: bool,
    /// How long typing pauses before it goes to the workbook: [`PAUSE`],
    /// longer in tests, whose clock runs fast.
    pub pause: f64,
}

impl Default for CodeEditor {
    fn default() -> CodeEditor {
        CodeEditor {
            open: false,
            tabs: Vec::new(),
            current: 0,
            rename: None,
            remove: None,
            notice: None,
            info: None,
            jump: None,
            compiled: None,
            files: Default::default(),
            immediate: Default::default(),
            forms_seen: 0,
            focus: false,
            drawn: false,
            pause: PAUSE,
        }
    }
}

/// An item open in a tab.
pub struct CodeTab {
    pub item: CodeItem,
    /// The text as typed, which is the item's source once it is put in the
    /// workbook.
    pub text: String,
    /// When the text was last typed, while it has not been put in the
    /// workbook yet.
    pub typed_at: Option<f64>,
    /// For a form: why what was typed cannot be its design. The form keeps the
    /// design it had, and the text stays as typed, until it can.
    pub problem: Option<String>,
}

/// The dialog of *Rename...*.
pub struct RenameModule {
    pub from: String,
    pub name: String,
    pub error: Option<String>,
    focused: bool,
}

impl CodeEditor {
    /// The tab shown, if any.
    pub fn current_tab(&self) -> Option<&CodeTab> {
        self.tabs.get(self.current)
    }

    /// The module shown, if a module is shown.
    pub fn current_module(&self) -> Option<&str> {
        self.current_tab().and_then(|t| t.item.module())
    }

    /// Whether some text has been typed and not yet put in the workbook.
    pub fn has_typing(&self) -> bool {
        self.tabs.iter().any(|t| t.typed_at.is_some())
    }

    /// The code editor of a workbook opened in the same window: shown if it
    /// was, with nothing open in it, and the Immediate window as it was.
    pub fn for_next_workbook(&mut self) -> CodeEditor {
        CodeEditor {
            open: self.open,
            pause: self.pause,
            immediate: std::mem::take(&mut self.immediate),
            ..CodeEditor::default()
        }
    }
}

fn viewport_id() -> ViewportId {
    ViewportId::from_hash_of("cliotree_code_editor")
}

fn editor_id(item: &CodeItem) -> egui::Id {
    egui::Id::unique(("code_text", item))
}

impl ClioApp {
    // ---------------------------------------------------------- commands

    /// *Tools > Code editor* and Alt+F11: shows the window, or brings it to the
    /// front, with the first module open if none is.
    pub fn open_code_editor(&mut self) {
        self.code.open = true;
        self.code.focus = true;
        let first: Option<String> = self.calc().module_names().next().map(str::to_owned);
        if self.code.tabs.is_empty()
            && let Some(first) = first
        {
            self.open_module(&first);
        }
    }

    /// Opens a module in a tab, or shows its tab.
    pub fn open_module(&mut self, name: &str) {
        let Some(source) = self.calc().module_source(name).map(str::to_owned) else {
            return;
        };
        self.open_item(CodeItem::Module(name.to_owned()), source);
    }

    /// Opens the design of a customised form in a tab, as its JSON text, or
    /// shows its tab.
    pub fn open_form_design(&mut self, sheet: &str, form: &str) {
        let Some(design) = self.calc().forms().get(sheet, form) else {
            return;
        };
        let text = design.to_json();
        self.open_item(
            CodeItem::Form {
                sheet: sheet.to_owned(),
                form: form.to_owned(),
            },
            text,
        );
    }

    fn open_item(&mut self, item: CodeItem, text: String) {
        if let Some(i) = self.code.tabs.iter().position(|t| t.item == item) {
            self.code.current = i;
            return;
        }
        self.code.tabs.push(CodeTab {
            item,
            text,
            typed_at: None,
            problem: None,
        });
        self.code.current = self.code.tabs.len() - 1;
        self.code.jump = Some((0, 0));
    }

    /// Closes a tab, putting what was typed in it in the workbook first.
    pub fn close_module_tab(&mut self, index: usize) {
        self.commit_code();
        if index < self.code.tabs.len() {
            self.code.tabs.remove(index);
        }
        self.code.current = self
            .code
            .current
            .min(self.code.tabs.len().saturating_sub(1));
    }

    /// *Insert > Module*: a new, empty module named `Module1`, `Module2`, ...,
    /// open in a tab.
    pub fn insert_module(&mut self) {
        self.commit_code();
        let name = self.calc().new_module_name();
        match self.session.calc_mut().add_module(&name, "") {
            Ok(()) => {
                self.code.notice = None;
                self.open_module(&name);
            }
            Err(e) => self.code.notice = Some(e.to_string()),
        }
    }

    /// Renames a module: one entry in the history.
    pub fn rename_module(&mut self, from: &str, to: &str) -> Result<(), String> {
        self.commit_code();
        self.session
            .calc_mut()
            .rename_module(from, to)
            .map_err(|e| e.to_string())?;
        self.follow_module_renames();
        Ok(())
    }

    /// Removes a module, and closes its tab: one entry in the history.
    pub fn remove_module(&mut self, name: &str) -> Result<(), String> {
        self.commit_code();
        self.session
            .calc_mut()
            .remove_module(name)
            .map_err(|e| e.to_string())?;
        self.code
            .tabs
            .retain(|t| t.item != CodeItem::Module(name.to_owned()));
        self.code.current = self
            .code
            .current
            .min(self.code.tabs.len().saturating_sub(1));
        Ok(())
    }

    /// Puts what has been typed in the workbook: one history entry for each
    /// module that was typed in, and the design of each form whose text can be
    /// one.
    pub fn commit_code(&mut self) {
        let mut typed: Vec<(usize, CodeItem, String)> = Vec::new();
        for (i, tab) in self.code.tabs.iter_mut().enumerate() {
            if tab.typed_at.take().is_some() {
                typed.push((i, tab.item.clone(), tab.text.clone()));
            }
        }
        for (i, item, text) in typed {
            match item {
                CodeItem::Module(module) => {
                    if let Err(e) = self.session.calc_mut().set_module_source(&module, &text) {
                        self.code.notice = Some(e.to_string());
                    }
                }
                CodeItem::Form { sheet, form } => {
                    let problem = self.put_form_text(&sheet, &form, &text).err();
                    if let Some(tab) = self.code.tabs.get_mut(i) {
                        tab.problem = problem;
                    }
                }
            }
        }
    }

    /// Makes the text the design of a form, if it is a design that can be
    /// used; otherwise says why.
    pub fn put_form_text(&mut self, sheet: &str, form: &str, text: &str) -> Result<(), String> {
        let design = FormDesign::from_json(text)?;
        if self.calc().forms().get(sheet, form) == Some(&design) {
            return Ok(());
        }
        if self.calc().forms().get(sheet, form).is_none() {
            return Err(format!("the form {form} of {sheet} is gone"));
        }
        self.session.calc_mut().forms_mut().set(sheet, form, design);
        Ok(())
    }

    /// Keeps the tabs in step with the workbook: what was typed goes in after
    /// a pause, renames are followed, a tab whose item has gone is closed,
    /// and a source changed by undo or redo, or a design changed in layout
    /// mode, is shown. The Immediate window's answers are taken.
    pub fn sync_code(&mut self, now: f64) {
        self.follow_module_renames();
        self.collect_immediate();
        if self
            .code
            .tabs
            .iter()
            .any(|t| t.typed_at.is_some_and(|at| now - at >= self.code.pause))
        {
            self.commit_code();
        }
        let calc = self.session.calc();
        let forms_changed = calc.forms_version() != self.code.forms_seen;
        self.code.forms_seen = calc.forms_version();
        self.code.tabs.retain(|t| {
            t.typed_at.is_some()
                || match &t.item {
                    CodeItem::Module(m) => calc.module_source(m).is_some(),
                    CodeItem::Form { sheet, form } => calc.forms().get(sheet, form).is_some(),
                }
        });
        for tab in &mut self.code.tabs {
            if tab.typed_at.is_some() {
                continue;
            }
            match &tab.item {
                CodeItem::Module(m) => {
                    if let Some(source) = calc.module_source(m)
                        && source != tab.text
                    {
                        tab.text = source.to_owned();
                    }
                }
                CodeItem::Form { sheet, form } => {
                    // A text that cannot be used yet stays as typed.
                    if forms_changed
                        && tab.problem.is_none()
                        && let Some(design) = calc.forms().get(sheet, form)
                        && FormDesign::from_json(&tab.text).ok().as_ref() != Some(design)
                    {
                        tab.text = design.to_json();
                    }
                }
            }
        }
        if let Some(module) = &self.code.compiled
            && calc.module_source(module).is_none()
        {
            self.code.compiled = None;
        }
        self.code.current = self
            .code
            .current
            .min(self.code.tabs.len().saturating_sub(1));
    }

    fn follow_module_renames(&mut self) {
        for (from, to) in self.session.calc_mut().take_module_renames() {
            for tab in &mut self.code.tabs {
                if tab.item == CodeItem::Module(from.clone()) {
                    tab.item = CodeItem::Module(to.clone());
                }
            }
            if self.code.compiled.as_ref() == Some(&from) {
                self.code.compiled = Some(to.clone());
            }
        }
    }

    // ------------------------------------------------------------ window

    /// The code editor's window, while it is open.
    pub(crate) fn code_window(&mut self, ctx: &egui::Context) {
        if !self.code.open {
            self.code.drawn = false;
            return;
        }
        if self.code.has_typing() {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(self.code.pause));
        }
        let builder = ViewportBuilder::default()
            .with_title(TITLE)
            .with_inner_size(SIZE)
            .with_min_inner_size([480.0, 360.0]);
        ctx.clone()
            .show_viewport_immediate(viewport_id(), builder, |ui, class| {
                self.code_editor_ui(ui, class);
            });
    }

    fn code_editor_ui(&mut self, ui: &mut egui::Ui, class: ViewportClass) {
        let ctx = ui.ctx().clone();
        if class == ViewportClass::EmbeddedWindow {
            // Drawn inside the main window (the tests): give the panels room.
            ui.set_min_size(egui::vec2(SIZE[0] - 160.0, SIZE[1] - 120.0));
        } else if ui.input(|i| i.viewport().close_requested()) {
            self.commit_code();
            self.code.open = false;
            return;
        }
        if std::mem::take(&mut self.code.focus) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        self.code_keys(ui);
        if !self.code.open {
            return;
        }

        let bar = crate::style::bar_frame(ui);
        egui::Panel::top("code_menu")
            .frame(bar)
            .show(ui, |ui| self.code_menu(ui));
        egui::Panel::bottom("code_status")
            .frame(bar)
            .show(ui, |ui| self.code_status(ui));
        if self.code.immediate.open {
            egui::Panel::bottom("immediate")
                .resizable(true)
                .default_size(150.0)
                .min_size(80.0)
                .show(ui, |ui| self.immediate_pane(ui));
        }
        if self.code.compiled.is_some() {
            egui::Panel::bottom("compile")
                .resizable(true)
                .default_size(110.0)
                .min_size(60.0)
                .show(ui, |ui| self.compile_pane(ui));
        }
        egui::Panel::left("code_explorer")
            .resizable(true)
            .default_size(200.0)
            .show(ui, |ui| self.explorer(ui));
        egui::CentralPanel::default().show(ui, |ui| self.code_pane(ui));
        self.rename_module_window(&ctx);
        self.remove_module_window(&ctx);
        self.module_file_windows(&ctx);
        self.code.drawn = true;
    }

    /// The keys of the code editor's window: the workbook's undo and redo,
    /// saving, the Immediate window, and Alt+F11 back to the workbook.
    fn code_keys(&mut self, ui: &mut egui::Ui) {
        let undo = KeyboardShortcut::new(Modifiers::COMMAND, Key::Z);
        let redo_y = KeyboardShortcut::new(Modifiers::COMMAND, Key::Y);
        let redo_z = KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z);
        let save = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
        let back = KeyboardShortcut::new(Modifiers::ALT, Key::F11);
        let immediate = KeyboardShortcut::new(Modifiers::COMMAND, Key::G);
        // A form's text is not in the workbook's history: its tab keeps the
        // text box's own undo, as does the Immediate window's line.
        let own_undo = self
            .code
            .current_tab()
            .is_some_and(|t| matches!(t.item, CodeItem::Form { .. }))
            && ui.memory(|m| {
                m.focused().is_some_and(|id| {
                    Some(id) == self.code.current_tab().map(|t| editor_id(&t.item))
                })
            })
            || ui.memory(|m| m.has_focus(crate::immediate::input_id()));
        if !own_undo {
            // The longer shortcut first, so that Ctrl+Shift+Z is not Ctrl+Z.
            if ui.input_mut(|i| i.consume_shortcut(&redo_z) || i.consume_shortcut(&redo_y)) {
                self.redo();
            } else if ui.input_mut(|i| i.consume_shortcut(&undo)) {
                self.undo();
            }
        }
        if ui.input_mut(|i| i.consume_shortcut(&save)) {
            self.begin_save();
        }
        if ui.input_mut(|i| i.consume_shortcut(&immediate)) {
            self.show_immediate();
        }
        if ui.input_mut(|i| i.consume_shortcut(&back)) {
            self.commit_code();
            ui.ctx()
                .send_viewport_cmd_to(ViewportId::ROOT, egui::ViewportCommand::Focus);
        }
    }

    fn code_menu(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("Import module...").clicked() {
                    self.begin_import_module();
                    ui.close();
                }
                let module = self.code.current_module().map(str::to_owned);
                let export = module
                    .as_ref()
                    .map_or("Export module...".to_owned(), |m| format!("Export {m}..."));
                if ui
                    .add_enabled(module.is_some(), egui::Button::new(export))
                    .clicked()
                {
                    if let Some(module) = module {
                        self.begin_export_module(&module);
                    }
                    ui.close();
                }
                ui.separator();
                if ui.button("Save workbook").clicked() {
                    self.begin_save();
                    ui.close();
                }
                ui.separator();
                if ui.button("Close and return to ClioTree").clicked() {
                    self.commit_code();
                    self.code.open = false;
                    ui.close();
                }
            });
            ui.menu_button("Edit", |ui| {
                // What is typed counts as the next entry, so Undo names it.
                let typing = self.code.has_typing();
                let undo_label = if typing {
                    Some("typing".to_owned())
                } else {
                    self.calc().undo_label().map(str::to_owned)
                };
                let redo_label = if typing {
                    None
                } else {
                    self.calc().redo_label().map(str::to_owned)
                };
                let can_change =
                    self.editing.is_none() && self.session.running_procedure().is_none();
                let undo_text = undo_label
                    .as_ref()
                    .map_or("Undo".to_owned(), |l| format!("Undo {l}"));
                if ui
                    .add_enabled(
                        undo_label.is_some() && can_change,
                        egui::Button::new(undo_text),
                    )
                    .clicked()
                {
                    self.undo();
                    ui.close();
                }
                let redo_text = redo_label
                    .as_ref()
                    .map_or("Redo".to_owned(), |l| format!("Redo {l}"));
                if ui
                    .add_enabled(
                        redo_label.is_some() && can_change,
                        egui::Button::new(redo_text),
                    )
                    .clicked()
                {
                    self.redo();
                    ui.close();
                }
            });
            ui.menu_button("View", |ui| {
                let shown = self.code.immediate.open;
                if ui
                    .add(
                        egui::Button::selectable(shown, "Immediate window").shortcut_text("Ctrl+G"),
                    )
                    .clicked()
                {
                    if shown {
                        self.code.immediate.open = false;
                    } else {
                        self.show_immediate();
                    }
                    ui.close();
                }
            });
            ui.menu_button("Insert", |ui| {
                if ui.button("Module").clicked() {
                    self.insert_module();
                    ui.close();
                }
            });
            ui.menu_button("Debug", |ui| {
                let module = self.code.current_module().map(str::to_owned);
                let label = module
                    .as_ref()
                    .map_or("Compile".to_owned(), |m| format!("Compile {m}"));
                if ui
                    .add_enabled(module.is_some(), egui::Button::new(label))
                    .on_disabled_hover_text("Open a module to compile it.")
                    .clicked()
                {
                    if let Some(module) = module {
                        self.compile_module(&module);
                    }
                    ui.close();
                }
            });
        });
    }

    /// The project explorer: the workbook's modules, a failed one in red, and
    /// the customised forms of each tab.
    fn explorer(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.strong("Project");
        ui.separator();
        let names: Vec<String> = self.calc().module_names().map(str::to_owned).collect();
        let shown = self.code.current_tab().map(|t| t.item.clone());
        egui::ScrollArea::vertical()
            .id_salt("code_explorer_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let header = egui::CollapsingHeader::new("Modules")
                    .default_open(true)
                    .show(ui, |ui| {
                        if names.is_empty() {
                            ui.weak("No modules yet");
                        }
                        for name in &names {
                            let status = self.calc().module_status(name).cloned();
                            let mut text = egui::RichText::new(name);
                            if matches!(status, Some(ModuleStatus::Failed(_))) {
                                text = text.color(error_color(ui));
                            }
                            let selected = shown == Some(CodeItem::Module(name.clone()));
                            let response = ui.selectable_label(selected, text);
                            let response = match &status {
                                Some(ModuleStatus::Failed(why)) => {
                                    response.on_hover_text(format!("Failed to load: {why}"))
                                }
                                _ => response,
                            };
                            if response.clicked() || response.double_clicked() {
                                self.open_module(name);
                            }
                            response.context_menu(|ui| self.module_menu(ui, name));
                        }
                    });
                header.header_response.context_menu(|ui| {
                    if ui.button("Insert module").clicked() {
                        self.insert_module();
                        ui.close();
                    }
                    if ui.button("Import module...").clicked() {
                        self.begin_import_module();
                        ui.close();
                    }
                });
                self.forms_branch(ui, shown.as_ref());
            });
    }

    /// The *Forms* branch: the customised forms, under their tabs. A form
    /// that is still generated has no design to show.
    fn forms_branch(&mut self, ui: &mut egui::Ui, shown: Option<&CodeItem>) {
        let forms: Vec<(String, Vec<String>)> = self
            .calc()
            .sheet_names()
            .map(|sheet| {
                let names = self
                    .calc()
                    .forms()
                    .names(sheet)
                    .into_iter()
                    .map(str::to_owned)
                    .collect::<Vec<_>>();
                (sheet.to_owned(), names)
            })
            .filter(|(_, names)| !names.is_empty())
            .collect();
        egui::CollapsingHeader::new("Forms")
            .default_open(true)
            .show(ui, |ui| {
                if forms.is_empty() {
                    ui.weak("No customised forms");
                }
                for (sheet, names) in &forms {
                    egui::CollapsingHeader::new(sheet.as_str())
                        .id_salt(("forms_of", sheet.as_str()))
                        .default_open(true)
                        .show(ui, |ui| {
                            for form in names {
                                let item = CodeItem::Form {
                                    sheet: sheet.clone(),
                                    form: form.clone(),
                                };
                                let response =
                                    ui.selectable_label(shown == Some(&item), form.as_str());
                                if response.clicked() || response.double_clicked() {
                                    self.open_form_design(sheet, form);
                                }
                            }
                        });
                }
            });
    }

    /// The context menu of a module in the explorer.
    fn module_menu(&mut self, ui: &mut egui::Ui, name: &str) {
        if ui.button("Open").clicked() {
            self.open_module(name);
            ui.close();
        }
        if ui.button("Rename...").clicked() {
            self.code.rename = Some(RenameModule {
                from: name.to_owned(),
                name: name.to_owned(),
                error: None,
                focused: false,
            });
            ui.close();
        }
        if ui.button("Remove...").clicked() {
            self.code.remove = Some(name.to_owned());
            ui.close();
        }
        ui.separator();
        if ui.button("Export...").clicked() {
            self.begin_export_module(name);
            ui.close();
        }
        if ui.button("Compile").clicked() {
            self.open_module(name);
            self.compile_module(name);
            ui.close();
        }
        ui.separator();
        if ui.button("Insert module").clicked() {
            self.insert_module();
            ui.close();
        }
    }

    /// The tabs of the open items, the procedure box and the code.
    fn code_pane(&mut self, ui: &mut egui::Ui) {
        if self.code.tabs.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.weak("No module is open. Insert > Module makes one.");
            });
            return;
        }
        let mut close = None;
        ui.horizontal_wrapped(|ui| {
            for (i, tab) in self.code.tabs.iter().enumerate() {
                let title = tab.item.title();
                if ui
                    .selectable_label(i == self.code.current, &title)
                    .clicked()
                {
                    self.code.current = i;
                }
                let label = format!("Close {title}");
                if crate::icons::button(ui, true, crate::icons::X, "x", &label).clicked() {
                    close = Some(i);
                }
                ui.add_space(8.0);
            }
        });
        if let Some(i) = close {
            self.close_module_tab(i);
            return;
        }
        ui.separator();
        if self.code.current_module().is_some() {
            self.procedure_box(ui);
            ui.add_space(4.0);
        }
        self.code_text(ui);
    }

    /// The procedure box: the module's top-level functions; choosing one
    /// jumps to it. It shows the function the cursor is in.
    fn procedure_box(&mut self, ui: &mut egui::Ui) {
        let Some(tab) = self.code.current_tab() else {
            return;
        };
        let functions = code_text::functions(&tab.text);
        let id = editor_id(&tab.item);
        let cursor_line = egui::TextEdit::load_state(ui.ctx(), id)
            .and_then(|s| s.cursor.char_range())
            .map_or(0, |r| code_text::line_of(&tab.text, r.primary.index.0));
        let here = functions
            .iter()
            .rev()
            .find(|(_, line)| *line <= cursor_line)
            .map_or("(General)", |(name, _)| name.as_str());
        let mut jump = None;
        ui.horizontal(|ui| {
            ui.label("Procedure");
            let response = egui::ComboBox::from_id_salt("procedure_box")
                .selected_text(here)
                .width(220.0)
                .show_ui(ui, |ui| {
                    if functions.is_empty() {
                        ui.weak("No functions");
                    }
                    for (name, line) in &functions {
                        if ui.selectable_label(name == here, name).clicked() {
                            jump = Some((*line, 0));
                        }
                    }
                })
                .response;
            response.widget_info(|| {
                let mut info =
                    egui::WidgetInfo::labeled(egui::Role::ComboBox, true, "Procedure box");
                info.current_text_value = Some(here.to_owned());
                info
            });
        });
        if jump.is_some() {
            self.code.jump = jump;
        }
    }

    /// The text of the tab shown, with line numbers and highlighting, in the
    /// font, indentation and wrapping of the preferences.
    fn code_text(&mut self, ui: &mut egui::Ui) {
        let prefs = self.user.prefs.code;
        let now = ui.input(|i| i.time);
        let index = self.code.current;
        // A click gives up the focus of every text box it is not on, so a jump
        // asked for by a click (a menu, the procedure box) waits a frame.
        let wait =
            !self.code.drawn || ui.input(|i| i.pointer.any_pressed() || i.pointer.any_released());
        if wait && self.code.jump.is_some() {
            ui.ctx().request_repaint();
        }
        let jump = if wait { None } else { self.code.jump.take() };
        let Some(tab) = self.code.tabs.get_mut(index) else {
            return;
        };
        let (language, indent) = match tab.item {
            CodeItem::Module(_) => ("py", prefs.indent as usize),
            CodeItem::Form { .. } => ("json", FORM_INDENT),
        };
        let id = editor_id(&tab.item);
        let ctx = ui.ctx().clone();
        // The editor's own size of the monospace font, which the highlighter
        // takes from the style.
        let font = egui::FontId::monospace(prefs.font_size as f32);
        ui.style_mut()
            .text_styles
            .insert(egui::TextStyle::Monospace, font.clone());
        let theme = egui_extras::syntax_highlighting::CodeTheme::from_style(ui.style());

        // Enter, Tab and Shift+Tab are the editor's, before the text box sees
        // them.
        if ui.memory(|m| m.has_focus(id))
            && let Some(range) =
                egui::TextEdit::load_state(&ctx, id).and_then(|s| s.cursor.char_range())
        {
            let (a, b) = (range.primary.index.0, range.secondary.index.0);
            let (start, end) = (a.min(b), a.max(b));
            let edited = ui.input_mut(|i| {
                if i.consume_key(Modifiers::SHIFT, Key::Tab) {
                    Some(code_text::dedent(&tab.text, start, end, indent))
                } else if i.consume_key(Modifiers::NONE, Key::Tab) {
                    Some(code_text::indent(&tab.text, start, end, indent))
                } else if i.consume_key(Modifiers::NONE, Key::Enter) {
                    Some(code_text::newline(&tab.text, start, end, indent))
                } else {
                    None
                }
            });
            if let Some(Edited { text, start, end }) = edited {
                if text != tab.text {
                    tab.text = text;
                    tab.typed_at = Some(now);
                }
                set_selection(&ctx, id, start, end);
            }
        }
        if let Some((line, column)) = jump {
            let at = code_text::place(&tab.text, line, column);
            set_selection(&ctx, id, at, at);
            ctx.memory_mut(|m| m.request_focus(id));
        }

        let row_height = ui.fonts_mut(|f| f.row_height(&font));
        let wrap = prefs.wrap;
        let highlighting = prefs.highlighting;
        let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap_width: f32| {
            let mut job = if highlighting {
                egui_extras::syntax_highlighting::highlight(
                    ui.ctx(),
                    ui.style(),
                    &theme,
                    text.as_str(),
                    language,
                )
            } else {
                egui::text::LayoutJob::simple(
                    text.as_str().to_owned(),
                    font.clone(),
                    ui.visuals().text_color(),
                    f32::INFINITY,
                )
            };
            // Long lines are wrapped at the pane, or the pane scrolls sideways.
            job.wrap.max_width = if wrap { wrap_width } else { f32::INFINITY };
            ui.fonts_mut(|f| f.layout_job(job))
        };
        let numbers_width = if prefs.line_numbers {
            let digits = code_text::line_count(&tab.text).to_string().len().max(2);
            ui.fonts_mut(|f| f.glyph_width(&font, '0')) * digits as f32
        } else {
            0.0
        };
        let scroll = if wrap {
            egui::ScrollArea::vertical()
        } else {
            egui::ScrollArea::both()
        };
        scroll
            .id_salt(("code_scroll", &tab.item))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    let numbers = if prefs.line_numbers {
                        Some(
                            ui.allocate_exact_size(
                                egui::vec2(numbers_width, row_height),
                                egui::Sense::hover(),
                            )
                            .0,
                        )
                    } else {
                        None
                    };
                    let output = egui::TextEdit::multiline(&mut tab.text)
                        .id(id)
                        .code_editor()
                        .font(font.clone())
                        .frame(egui::Frame::NONE)
                        .margin(egui::Margin::ZERO)
                        .desired_width(f32::INFINITY)
                        .desired_rows(30)
                        .layouter(&mut layouter)
                        .show(ui);
                    named(&output.response, "Code");
                    if output.response.changed() {
                        tab.typed_at = Some(now);
                    }
                    if let Some(numbers) = numbers {
                        paint_line_numbers(ui, numbers.right(), &output, &font);
                    }
                    if let Some((line, _)) = jump {
                        let top = output.galley_pos.y
                            + output
                                .galley
                                .rows
                                .iter()
                                .scan(0usize, |seen, row| {
                                    let this = *seen;
                                    if row.ends_with_newline {
                                        *seen += 1;
                                    }
                                    Some((this, row.pos.y))
                                })
                                .find(|(l, _)| *l == line)
                                .map_or(line as f32 * row_height, |(_, y)| y);
                        let rect = egui::Rect::from_min_size(
                            egui::pos2(output.response.rect.left(), top),
                            egui::vec2(1.0, row_height),
                        );
                        ui.scroll_to_rect(rect, Some(egui::Align::TOP));
                    }
                });
            });
    }

    /// The status line: what loading the module shown came to, or whether
    /// the text of the form shown is its design.
    fn code_status(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if let Some(notice) = &self.code.notice {
                ui.colored_label(error_color(ui), notice);
                return;
            }
            if let Some(info) = &self.code.info {
                ui.label(info);
                return;
            }
            let Some(tab) = self.code.current_tab() else {
                ui.label(format!("{} modules", self.calc().module_names().count()));
                return;
            };
            let module = match &tab.item {
                CodeItem::Module(module) => module,
                CodeItem::Form { sheet, form } => {
                    let title = format!("Form {form} of {sheet}");
                    if tab.typed_at.is_some() {
                        ui.label(format!("{title}: waiting for typing to pause..."));
                    } else if let Some(problem) = &tab.problem {
                        ui.colored_label(
                            error_color(ui),
                            format!("{title} keeps its design until this is fixed: {problem}"),
                        );
                    } else {
                        ui.label(format!("{title}: this is its design"));
                    }
                    return;
                }
            };
            let status = if tab.typed_at.is_some() {
                Some(ModuleStatus::Pending)
            } else {
                self.calc().module_status(module).cloned()
            };
            match status {
                Some(ModuleStatus::Loaded) => {
                    ui.label(format!("{module}: loaded"));
                }
                Some(ModuleStatus::Pending) | None => {
                    ui.label(format!("{module}: loading..."));
                }
                Some(ModuleStatus::Failed(why)) => {
                    ui.colored_label(error_color(ui), format!("{module} failed to load: {why}"));
                }
                Some(ModuleStatus::Untrusted) => {
                    ui.label(format!(
                        "{module}: not loaded, because the workbook is not trusted"
                    ));
                }
            }
        });
    }

    // ----------------------------------------------------------- dialogs

    fn rename_module_window(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.code.rename.take() else {
            return;
        };
        let problem = if dialog.name == dialog.from {
            None
        } else {
            self.calc()
                .module_name_problem(&dialog.name, Some(&dialog.from))
        };
        let mut open = true;
        let (mut apply, mut cancel) = (false, false);
        egui::Window::new(format!("Rename module {}", dialog.from))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label("New name");
                let response = ui.text_edit_singleline(&mut dialog.name);
                named(&response, "Module name");
                if !dialog.focused {
                    response.request_focus();
                    select_all(ui, &response, &dialog.name);
                    dialog.focused = true;
                }
                if response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    apply = true;
                }
                if ui.input(|i| i.key_pressed(Key::Escape)) {
                    cancel = true;
                }
                if let Some(why) = problem.as_ref().or(dialog.error.as_ref()) {
                    ui.colored_label(error_color(ui), why);
                }
                ui.label(
                    "Formulas that use the old name will show #NAME! until they are \
                     changed; names in formulas are not rewritten.",
                );
                ui.horizontal(|ui| {
                    apply |= ui
                        .add_enabled(problem.is_none(), egui::Button::new("Rename"))
                        .clicked();
                    cancel |= ui.button("Cancel").clicked();
                });
            });
        if apply && problem.is_none() {
            let (from, to) = (dialog.from.clone(), dialog.name.clone());
            if let Err(why) = self.rename_module(&from, &to) {
                dialog.error = Some(why);
                self.code.rename = Some(dialog);
            }
        } else if open && !cancel {
            self.code.rename = Some(dialog);
        }
    }

    fn remove_module_window(&mut self, ctx: &egui::Context) {
        let Some(name) = self.code.remove.take() else {
            return;
        };
        let mut open = true;
        let (mut apply, mut cancel) = (false, false);
        egui::Window::new(format!("Remove module {name}"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(format!(
                    "Formulas that use {name} or its functions will show #NAME! until it is \
                     back. Edit > Undo puts it back."
                ));
                if ui.input(|i| i.key_pressed(Key::Escape)) {
                    cancel = true;
                }
                ui.horizontal(|ui| {
                    apply = ui.button("Remove").clicked();
                    cancel |= ui.button("Cancel").clicked();
                });
            });
        if apply {
            if let Err(why) = self.remove_module(&name) {
                self.code.notice = Some(why);
            }
        } else if open && !cancel {
            self.code.remove = Some(name);
        }
    }
}

/// The number of each line, beside the first row it takes, right-aligned at
/// `right`: a wrapped line takes more than one row.
fn paint_line_numbers(
    ui: &egui::Ui,
    right: f32,
    output: &egui::text_edit::TextEditOutput,
    font: &egui::FontId,
) {
    let color = ui.visuals().weak_text_color();
    let painter = ui.painter();
    let mut line = 1;
    let mut first = true;
    for row in &output.galley.rows {
        if first {
            painter.text(
                egui::pos2(right, output.galley_pos.y + row.pos.y),
                egui::Align2::RIGHT_TOP,
                line.to_string(),
                font.clone(),
                color,
            );
        }
        first = row.ends_with_newline;
        if first {
            line += 1;
        }
    }
}

/// Puts the selection of the text box `id` from `start` to `end` (characters).
pub(crate) fn set_selection(ctx: &egui::Context, id: egui::Id, start: usize, end: usize) {
    let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
    state.cursor.set_char_range(Some(CCursorRange::two(
        CCursor::new(start),
        CCursor::new(end),
    )));
    egui::TextEdit::store_state(ctx, id, state);
}
