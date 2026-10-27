//! The code editor (DESIGN.md, "Python modules"): a second window, opened with
//! *Tools > Code editor* or Alt+F11, as Excel opens its Visual Basic editor. The
//! project explorer lists the workbook's modules; each open module has a tab
//! with line numbers, Python highlighting and automatic indentation; the
//! procedure box jumps to a function.
//!
//! **Typing and the history.** The text of each open module is the editor's
//! own while it is typed. It goes to the workbook, as one undoable entry, once
//! typing has paused for [`PAUSE`] seconds, and at once before anything that
//! needs the workbook as it is: undo and redo, saving, opening, closing, and
//! the module commands. So a burst of typing is one entry, and editing a
//! module does not reload it at every key. Undo in this window is the
//! workbook's, not a text box's own, so there is one history.

use egui::text::{CCursor, CCursorRange};
use egui::{Key, KeyboardShortcut, Modifiers, ViewportBuilder, ViewportClass, ViewportId};

use crate::ClioApp;
use crate::code_text::{self, Edited};
use crate::tab_rename::select_all;
use crate::view::{error_color, named};
use cliotree_calc::ModuleStatus;

/// The title of the code editor's window.
pub const TITLE: &str = "Code editor - ClioTree";

/// How long typing pauses before the text is put in the workbook, in seconds.
pub const PAUSE: f64 = 1.0;

/// Spaces per level of indentation.
const INDENT: usize = 4;

/// The size the window opens at.
const SIZE: [f32; 2] = [900.0, 600.0];

/// What the code editor keeps between frames.
pub struct CodeEditor {
    /// Whether the window is shown.
    pub open: bool,
    /// The modules open in tabs, in the order they were opened.
    pub tabs: Vec<CodeTab>,
    /// The tab shown.
    pub current: usize,
    /// The dialog of *Rename...*, while it is open.
    pub rename: Option<RenameModule>,
    /// The module *Remove...* asks about.
    pub remove: Option<String>,
    /// Why the last command was refused.
    pub notice: Option<String>,
    /// Put the cursor at the start of this line of the tab shown, and scroll
    /// to it.
    pub jump: Option<usize>,
    /// Bring the window to the front at the next frame.
    focus: bool,
    /// Whether the window was drawn in the last frame. A window's first frame
    /// only measures it, and a text box in it cannot take the focus yet.
    drawn: bool,
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
            jump: None,
            focus: false,
            drawn: false,
            pause: PAUSE,
        }
    }
}

/// A module open in a tab.
pub struct CodeTab {
    pub module: String,
    /// The text as typed, which is the module's source once it is put in the
    /// workbook.
    pub text: String,
    /// When the text was last typed, while it has not been put in the
    /// workbook yet.
    pub typed_at: Option<f64>,
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

    /// Whether some text has been typed and not yet put in the workbook.
    pub fn has_typing(&self) -> bool {
        self.tabs.iter().any(|t| t.typed_at.is_some())
    }

    /// The code editor of a workbook opened in the same window: shown if it
    /// was, with nothing open in it.
    pub fn for_next_workbook(&self) -> CodeEditor {
        CodeEditor {
            open: self.open,
            pause: self.pause,
            ..CodeEditor::default()
        }
    }
}

fn viewport_id() -> ViewportId {
    ViewportId::from_hash_of("cliotree_code_editor")
}

fn editor_id(module: &str) -> egui::Id {
    egui::Id::unique(("code_text", module))
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
        if let Some(i) = self.code.tabs.iter().position(|t| t.module == name) {
            self.code.current = i;
            return;
        }
        let Some(source) = self.calc().module_source(name) else {
            return;
        };
        self.code.tabs.push(CodeTab {
            module: name.to_owned(),
            text: source.to_owned(),
            typed_at: None,
        });
        self.code.current = self.code.tabs.len() - 1;
        self.code.jump = Some(0);
    }

    /// Closes the tab of a module, putting what was typed in it in the
    /// workbook first.
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
        self.code.tabs.retain(|t| t.module != name);
        self.code.current = self
            .code
            .current
            .min(self.code.tabs.len().saturating_sub(1));
        Ok(())
    }

    /// Puts what has been typed in the workbook: one history entry for each
    /// module that was typed in.
    pub fn commit_code(&mut self) {
        let typed: Vec<(String, String)> = self
            .code
            .tabs
            .iter_mut()
            .filter_map(|t| {
                t.typed_at
                    .take()
                    .map(|_| (t.module.clone(), t.text.clone()))
            })
            .collect();
        for (module, text) in typed {
            if let Err(e) = self.session.calc_mut().set_module_source(&module, &text) {
                self.code.notice = Some(e.to_string());
            }
        }
    }

    /// Keeps the tabs in step with the workbook: what was typed goes in after
    /// a pause, renames are followed, a tab whose module has gone is closed,
    /// and a source changed by undo or redo is shown.
    pub fn sync_code(&mut self, now: f64) {
        self.follow_module_renames();
        if self
            .code
            .tabs
            .iter()
            .any(|t| t.typed_at.is_some_and(|at| now - at >= self.code.pause))
        {
            self.commit_code();
        }
        let calc = self.session.calc();
        self.code
            .tabs
            .retain(|t| t.typed_at.is_some() || calc.module_source(&t.module).is_some());
        for tab in &mut self.code.tabs {
            if tab.typed_at.is_none()
                && let Some(source) = calc.module_source(&tab.module)
                && source != tab.text
            {
                tab.text = source.to_owned();
            }
        }
        self.code.current = self
            .code
            .current
            .min(self.code.tabs.len().saturating_sub(1));
    }

    fn follow_module_renames(&mut self) {
        for (from, to) in self.session.calc_mut().take_module_renames() {
            for tab in &mut self.code.tabs {
                if tab.module == from {
                    tab.module = to.clone();
                }
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
            .with_min_inner_size([480.0, 320.0]);
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
        egui::Panel::left("code_explorer")
            .resizable(true)
            .default_size(200.0)
            .show(ui, |ui| self.explorer(ui));
        egui::CentralPanel::default().show(ui, |ui| self.code_pane(ui));
        self.rename_module_window(&ctx);
        self.remove_module_window(&ctx);
        self.code.drawn = true;
    }

    /// The keys of the code editor's window: the workbook's undo and redo,
    /// saving, and Alt+F11 back to the workbook.
    fn code_keys(&mut self, ui: &mut egui::Ui) {
        let undo = KeyboardShortcut::new(Modifiers::COMMAND, Key::Z);
        let redo_y = KeyboardShortcut::new(Modifiers::COMMAND, Key::Y);
        let redo_z = KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z);
        let save = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
        let back = KeyboardShortcut::new(Modifiers::ALT, Key::F11);
        // The longer shortcut first, so that Ctrl+Shift+Z is not Ctrl+Z.
        if ui.input_mut(|i| i.consume_shortcut(&redo_z) || i.consume_shortcut(&redo_y)) {
            self.redo();
        } else if ui.input_mut(|i| i.consume_shortcut(&undo)) {
            self.undo();
        }
        if ui.input_mut(|i| i.consume_shortcut(&save)) {
            self.begin_save();
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
            ui.menu_button("Insert", |ui| {
                if ui.button("Module").clicked() {
                    self.insert_module();
                    ui.close();
                }
            });
        });
    }

    /// The project explorer: the workbook's modules, a failed one in red.
    fn explorer(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.strong("Project");
        ui.separator();
        let names: Vec<String> = self.calc().module_names().map(str::to_owned).collect();
        let shown = self.code.current_tab().map(|t| t.module.clone());
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
                    let selected = shown.as_deref() == Some(name.as_str());
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
        if ui.button("Insert module").clicked() {
            self.insert_module();
            ui.close();
        }
    }

    /// The tabs of the open modules, the procedure box and the code.
    fn code_pane(&mut self, ui: &mut egui::Ui) {
        if self.code.tabs.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.weak("No module is open. Insert > Module makes one.");
            });
            return;
        }
        let mut close = None;
        ui.horizontal(|ui| {
            for (i, tab) in self.code.tabs.iter().enumerate() {
                if ui
                    .selectable_label(i == self.code.current, &tab.module)
                    .clicked()
                {
                    self.code.current = i;
                }
                let label = format!("Close {}", tab.module);
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
        self.procedure_box(ui);
        ui.add_space(4.0);
        self.code_text(ui);
    }

    /// The procedure box: the module's top-level functions; choosing one
    /// jumps to it. It shows the function the cursor is in.
    fn procedure_box(&mut self, ui: &mut egui::Ui) {
        let Some(tab) = self.code.current_tab() else {
            return;
        };
        let functions = code_text::functions(&tab.text);
        let id = editor_id(&tab.module);
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
                            jump = Some(*line);
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

    /// The code of the tab shown, with line numbers and highlighting.
    fn code_text(&mut self, ui: &mut egui::Ui) {
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
        let id = editor_id(&tab.module);
        let ctx = ui.ctx().clone();
        let font = egui::TextStyle::Monospace.resolve(ui.style());
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
                    Some(code_text::dedent(&tab.text, start, end, INDENT))
                } else if i.consume_key(Modifiers::NONE, Key::Tab) {
                    Some(code_text::indent(&tab.text, start, end, INDENT))
                } else if i.consume_key(Modifiers::NONE, Key::Enter) {
                    Some(code_text::newline(&tab.text, start, end, INDENT))
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
        if let Some(line) = jump {
            set_selection(&ctx, id, code_text::start_of_line(&tab.text, line), {
                code_text::start_of_line(&tab.text, line)
            });
            ctx.memory_mut(|m| m.request_focus(id));
        }

        let row_height = ui.fonts_mut(|f| f.row_height(&font));
        let numbers = code_text::line_numbers(&tab.text);
        let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, _wrap: f32| {
            let mut job = egui_extras::syntax_highlighting::highlight(
                ui.ctx(),
                ui.style(),
                &theme,
                text.as_str(),
                "py",
            );
            // Long lines are not wrapped: the pane scrolls sideways.
            job.wrap.max_width = f32::INFINITY;
            ui.fonts_mut(|f| f.layout_job(job))
        };
        egui::ScrollArea::both()
            .id_salt(("code_scroll", &tab.module))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    ui.add(
                        egui::Label::new(egui::RichText::new(numbers).font(font.clone()).weak())
                            .selectable(false),
                    );
                    let output = egui::TextEdit::multiline(&mut tab.text)
                        .id(id)
                        .code_editor()
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
                    if let Some(line) = jump {
                        let top = output.response.rect.top() + line as f32 * row_height;
                        let rect = egui::Rect::from_min_size(
                            egui::pos2(output.response.rect.left(), top),
                            egui::vec2(1.0, row_height),
                        );
                        ui.scroll_to_rect(rect, Some(egui::Align::TOP));
                    }
                });
            });
    }

    /// The status line: what loading the module shown came to.
    fn code_status(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if let Some(notice) = &self.code.notice {
                ui.colored_label(error_color(ui), notice);
                return;
            }
            let Some(tab) = self.code.current_tab() else {
                ui.label(format!("{} modules", self.calc().module_names().count()));
                return;
            };
            let status = if tab.typed_at.is_some() {
                Some(ModuleStatus::Pending)
            } else {
                self.calc().module_status(&tab.module).cloned()
            };
            match status {
                Some(ModuleStatus::Loaded) => {
                    ui.label(format!("{}: loaded", tab.module));
                }
                Some(ModuleStatus::Pending) | None => {
                    ui.label(format!("{}: loading...", tab.module));
                }
                Some(ModuleStatus::Failed(why)) => {
                    ui.colored_label(
                        error_color(ui),
                        format!("{} failed to load: {why}", tab.module),
                    );
                }
                Some(ModuleStatus::Untrusted) => {
                    ui.label(format!(
                        "{}: not loaded, because the workbook is not trusted",
                        tab.module
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

/// Puts the selection of the text box `id` from `start` to `end` (characters).
fn set_selection(ctx: &egui::Context, id: egui::Id, start: usize, end: usize) {
    let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
    state.cursor.set_char_range(Some(CCursorRange::two(
        CCursor::new(start),
        CCursor::new(end),
    )));
    egui::TextEdit::store_state(ctx, id, state);
}
