//! Drawing the window: the menu, the formula bar, the grid, the tabs, the
//! status bar, and the Insert Function dialog.

use std::time::Duration;

use cliotree_calc::python::catalog::Group;
use cliotree_core::{Kind, Path, Segment, Value};
use egui::{Color32, Key, KeyboardShortcut, Modifiers, RichText, Sense, Stroke, StrokeKind};
use egui_extras::{Column, TableBuilder};

use crate::app::{ClioApp, Editing, Focus};
use crate::rows::{Row, RowKind, display};
use crate::status::{evaluation_text, problems_text};

pub(crate) use crate::style::ROW_HEIGHT;
use crate::style::{HEADER_HEIGHT, Palette};
const INDENT: f32 = 16.0;

/// Gives a text field a name that a screen reader (and a test) can find it by.
pub(crate) fn named(response: &egui::Response, name: &'static str) {
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, name));
}

pub(crate) fn error_color(ui: &egui::Ui) -> Color32 {
    Palette::of_ui(ui).error
}

pub(crate) fn computed_fill(ui: &egui::Ui) -> Color32 {
    Palette::of_ui(ui).computed
}

/// The outline of a cell whose data is not valid: kept and outlined, not
/// refused.
pub(crate) fn outline_invalid(ui: &egui::Ui, rect: egui::Rect) {
    ui.painter().rect_stroke(
        rect.shrink(1.0),
        2.0,
        Stroke::new(1.0, error_color(ui)),
        StrokeKind::Inside,
    );
}

/// Something a row asked for while it was being drawn. They are applied after
/// the table, because drawing borrows the workbook.
pub(crate) enum Action {
    Select(Path),
    Toggle(Path),
    BeginEdit(Path),
    EnterText(Path, String),
    AddItem(Path),
    InsertAbove(Path),
    Delete(Path),
    Commit,
    Cancel,
    // The tabular view.
    Reveal(Path),
    ToggleTabular(Path, bool),
    Reverse(Path),
    SortBy(Path, String, Option<bool>),
    SelectColumn(Path, String),
    HideColumn(Path, String),
    ShowColumn(Path, String),
    FilterDialog(Path, String),
    ClearFilter(Path, String),
    ClearAllFilters(Path),
    ReapplyFilters(Path),
    AddRow(Path, Option<usize>),
    DeleteRow(Path, usize),
    MoveRow(Path, usize, bool),
    HideRow(Path, usize),
    ShowRow(Path, cliotree_core::NodeId),
    ShowAllHidden(Path),
    ShowRefusedRows,
    DismissRefusal,
    // Changing the schema from this tab.
    AddField(Path),
    RenameField(Path, String),
    DeleteField(Path, String),
    ReorderFields(Path, String, String),
}

impl ClioApp {
    /// One frame of the window.
    pub fn show(&mut self, ui: &mut egui::Ui) {
        crate::style::prepare(ui.ctx());
        self.apply_settings();
        self.session.tick();
        self.collect_procedures();
        self.follow_renames();
        if !self.session.is_idle() {
            ui.ctx().request_repaint_after(Duration::from_millis(30));
        }
        self.refresh_rows();
        self.refresh_summary();
        self.handle_keys(ui);
        self.handle_close_request(ui.ctx());
        self.update_title(ui.ctx());

        let bar = crate::style::bar_frame(ui);
        egui::Panel::top("menu")
            .frame(bar)
            .show(ui, |ui| self.menu(ui));
        if self.banner_shown() {
            let fill = Palette::of_ui(ui).banner_fill;
            egui::Panel::top("trust_banner")
                .frame(bar.fill(fill).inner_margin(8))
                .show(ui, |ui| self.trust_banner(ui));
        }
        // A form has no formula bar: formulas are edited in the data tree.
        let form = !self.in_tree();
        if !form {
            egui::Panel::top("formula_bar")
                .frame(bar.inner_margin(egui::Margin::symmetric(8, 5)))
                .show(ui, |ui| self.formula_bar(ui));
        }
        if !form && self.calc().is_schema_sheet(&self.sheet) {
            egui::Panel::top("schema_bar")
                .frame(bar)
                .show(ui, |ui| self.schema_bar(ui));
        }
        egui::Panel::bottom("status")
            .frame(bar)
            .show(ui, |ui| self.status_bar(ui));
        egui::Panel::bottom("tabs")
            .frame(bar.inner_margin(egui::Margin::symmetric(8, 0)))
            .show(ui, |ui| self.tab_bar(ui));
        if self.show_issues {
            egui::Panel::right("issues")
                .default_size(300.0)
                .show(ui, |ui| self.issues_panel(ui));
        }
        if form && self.layout_active() {
            egui::Panel::right("form_properties")
                .resizable(true)
                .default_size(320.0)
                .show(ui, |ui| self.property_page(ui));
        }
        if form {
            egui::CentralPanel::default().show(ui, |ui| {
                if self.in_form() {
                    self.form_view(ui);
                } else {
                    let font = self.data_font;
                    ui.scope(|ui| {
                        crate::style::use_font(ui.style_mut(), Some(font.family), Some(font.size));
                        self.raw_view(ui);
                    });
                }
            });
        } else {
            self.refresh_rows();
            self.refresh_table();
            if self.layout().is_some() {
                let font = self.data_font;
                egui::Panel::bottom("table")
                    .resizable(true)
                    .default_size(280.0)
                    .show(ui, |ui| {
                        crate::style::use_font(ui.style_mut(), Some(font.family), Some(font.size));
                        self.table_pane(ui);
                    });
            }
            let font = self.data_font;
            egui::CentralPanel::default().show(ui, |ui| {
                crate::style::use_font(ui.style_mut(), Some(font.family), Some(font.size));
                self.grid(ui);
            });
        }
        self.insert_function_window(ui.ctx());
        self.filter_window(ui.ctx());
        self.schema_window(ui.ctx());
        self.form_window(ui.ctx());
        self.rename_tab_window(ui.ctx());
        self.file_windows(ui.ctx());
    }

    // ------------------------------------------------------------- keys

    fn handle_keys(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let typing_somewhere = ctx.text_edit_focused();
        let undo = KeyboardShortcut::new(Modifiers::COMMAND, Key::Z);
        let redo_y = KeyboardShortcut::new(Modifiers::COMMAND, Key::Y);
        let redo_z = KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z);
        // The file commands work wherever the keyboard is.
        let save_as = KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::S);
        let save = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
        let open = KeyboardShortcut::new(Modifiers::COMMAND, Key::O);
        let new = KeyboardShortcut::new(Modifiers::COMMAND, Key::N);
        if ui.input_mut(|i| i.consume_shortcut(&save_as)) {
            self.begin_save_as();
        } else if ui.input_mut(|i| i.consume_shortcut(&save)) {
            self.begin_save();
        } else if ui.input_mut(|i| i.consume_shortcut(&open)) {
            self.begin_open();
        } else if ui.input_mut(|i| i.consume_shortcut(&new)) {
            self.begin_new();
        }
        if !typing_somewhere {
            // The longer shortcut first, so that Ctrl+Shift+Z is not Ctrl+Z.
            if ui.input_mut(|i| i.consume_shortcut(&redo_z) || i.consume_shortcut(&redo_y)) {
                self.redo();
            } else if ui.input_mut(|i| i.consume_shortcut(&undo)) {
                self.undo();
            }
        }
        if typing_somewhere || self.editing.is_some() || !self.in_tree() {
            return;
        }
        let down = ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowDown));
        let up = ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowUp));
        let right = ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowRight));
        let left = ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowLeft));
        if self.table_cell_selected() {
            // In a table the arrows move among the cells that are shown.
            let dx = isize::from(right) - isize::from(left);
            let dy = isize::from(down) - isize::from(up);
            if dx != 0 || dy != 0 {
                self.table_move_selection(dx, dy);
            }
        } else {
            if down {
                self.move_selection(1);
            }
            if up {
                self.move_selection(-1);
            }
            if let Some(row) = self.selected_row().cloned() {
                if right && ClioApp::is_container(&row) {
                    self.set_open(&row.path, true);
                }
                if left {
                    if row.expanded {
                        self.set_open(&row.path, false);
                    } else if let Some(parent) = row.path.parent()
                        && self.select(&parent)
                    {
                        self.scroll_to = self.row_index(&parent);
                    }
                }
            }
        }
        let Some(path) = self.selection.clone() else {
            return;
        };
        if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::F2)) {
            self.begin_editing(&path, true, None);
        } else if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Delete)) {
            if self.can_edit(&path) {
                self.enter_directly(&path, "");
            }
        } else {
            // Typing on a selected cell starts editing it with what was typed.
            let typed: Option<String> = ui.input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::Text(t) if !t.chars().all(char::is_control) => Some(t.clone()),
                    _ => None,
                })
            });
            if let Some(text) = typed
                && self.begin_editing(&path, true, Some(text))
            {
                ui.input_mut(|i| {
                    i.events.retain(|e| !matches!(e, egui::Event::Text(_)));
                });
            }
        }
    }

    // ------------------------------------------------------------- menu

    fn menu(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("New").clicked() {
                    self.begin_new();
                    ui.close();
                }
                if ui.button("Open...").clicked() {
                    self.begin_open();
                    ui.close();
                }
                self.recent_menu(ui);
                if ui
                    .add_enabled(self.editing.is_none(), egui::Button::new("Save"))
                    .clicked()
                {
                    self.begin_save();
                    ui.close();
                }
                if ui.button("Save As...").clicked() {
                    self.begin_save_as();
                    ui.close();
                }
                ui.separator();
                if ui.button("Quit").clicked() {
                    let ctx = ui.ctx().clone();
                    self.request_quit(&ctx);
                    ui.close();
                }
            });
            ui.menu_button("Edit", |ui| {
                let undo_label = self.calc().undo_label().map(str::to_owned);
                let redo_label = self.calc().redo_label().map(str::to_owned);
                // Undo and redo wait while a button procedure runs.
                let can_change =
                    self.editing.is_none() && self.session.running_procedure().is_none();
                let undo_text = match &undo_label {
                    Some(label) => format!("Undo {label}"),
                    None => "Undo".to_owned(),
                };
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
                let redo_text = match &redo_label {
                    Some(label) => format!("Redo {label}"),
                    None => "Redo".to_owned(),
                };
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
                ui.separator();
                if ui
                    .add_enabled(can_change, egui::Button::new("Rename tab..."))
                    .clicked()
                {
                    let sheet = self.sheet.clone();
                    self.begin_rename_tab(&sheet);
                    ui.close();
                }
            });
            ui.menu_button("View", |ui| {
                // How the current tab is shown.
                ui.menu_button("Show as", |ui| {
                    for view in self.view_names() {
                        let chosen = self.current_view() == &view;
                        if ui.radio(chosen, view.label()).clicked() {
                            self.set_view(view);
                            ui.close();
                        }
                    }
                });
                self.forms_menu(ui);
                ui.menu_button("Font", |ui| self.font_menu(ui));
                ui.separator();
                if ui.checkbox(&mut self.show_issues, "Issues panel").clicked() {
                    ui.close();
                }
                if ui.button("Collapse all").clicked() {
                    let root = Path::root();
                    if let Some(set) = self.expanded.get_mut(&self.sheet) {
                        set.clear();
                        set.insert(root);
                    }
                    self.expansion_rev += 1;
                    ui.close();
                }
            });
        });
    }

    /// File > Open Recent: the file names, with the folder on hover.
    fn recent_menu(&mut self, ui: &mut egui::Ui) {
        let recent = self.user.prefs.recent.clone();
        ui.add_enabled_ui(!recent.is_empty(), |ui| {
            ui.menu_button("Open Recent", |ui| {
                for path in &recent {
                    let name = path.file_name().map_or_else(
                        || path.display().to_string(),
                        |n| n.to_string_lossy().into_owned(),
                    );
                    if ui
                        .button(name)
                        .on_hover_text(path.display().to_string())
                        .clicked()
                    {
                        self.begin_open_recent(path);
                        ui.close();
                    }
                }
                ui.separator();
                if ui.button("Clear the list").clicked() {
                    self.change_prefs(|p| p.recent.clear());
                    ui.close();
                }
            });
        });
    }

    /// View > Font: the typeface and size of the Data tree and Raw views of
    /// every tab. A form's text has the font of its controls instead.
    fn font_menu(&mut self, ui: &mut egui::Ui) {
        ui.weak("Data tree and Raw views, every tab");
        let current = self.data_font;
        let mut chosen = current;
        for typeface in crate::style::Typeface::ALL {
            let label = crate::style::typeface_label(typeface);
            let text = RichText::new(label).family(crate::style::family(typeface));
            if ui.radio(current.family == typeface, text).clicked() {
                chosen.family = typeface;
            }
        }
        ui.separator();
        for size in crate::style::DATA_FONT_SIZES {
            if ui
                .radio(current.size == size, format!("{size} pt"))
                .clicked()
            {
                chosen.size = size;
            }
        }
        ui.separator();
        let default = crate::style::DataFont::default();
        if ui
            .add_enabled(current != default, egui::Button::new("Default font"))
            .clicked()
        {
            chosen = default;
        }
        if chosen != current {
            self.set_data_font(chosen);
            ui.close();
        }
    }

    // ------------------------------------------------------ formula bar

    fn formula_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let name = match self.column_selected() {
                Some(c) => format!("{}[].{}", self.name_of(&c.array), c.field),
                None => self
                    .selection
                    .as_ref()
                    .map(|p| self.name_of(p))
                    .unwrap_or_default(),
            };
            // The name box: the path of the selection, in a box like a field's.
            let palette = Palette::of_ui(ui);
            egui::Frame::new()
                .fill(palette.field)
                .stroke(Stroke::new(1.0, palette.border))
                .corner_radius(crate::style::CONTROL_RADIUS)
                .inner_margin(egui::Margin::symmetric(6, 2))
                .show(ui, |ui| {
                    ui.set_width(248.0);
                    ui.set_min_height(18.0);
                    ui.add(egui::Label::new(RichText::new(name).monospace()).truncate());
                });
            let fx = crate::style::medium("fx").italics();
            if ui.button(fx).on_hover_text("Insert Function").clicked() {
                self.open_insert_function();
            }
            self.bar_text(ui);
        });
        if let Some(notice) = &self.notice {
            let color = error_color(ui);
            ui.colored_label(color, notice);
        }
    }

    /// The text of the selected cell: its formula or its value. Clicking in it
    /// starts editing; Enter enters, Esc cancels.
    fn bar_text(&mut self, ui: &mut egui::Ui) {
        let id = egui::Id::unique("formula_bar_text");
        let column = self.column_selected().cloned();
        let Some(path) = column
            .as_ref()
            .map(|c| c.place())
            .or_else(|| self.selection.clone())
        else {
            let mut empty = String::new();
            let response = ui.add_enabled(
                false,
                egui::TextEdit::singleline(&mut empty).desired_width(f32::INFINITY),
            );
            named(&response, "Formula bar");
            return;
        };
        let editing_here = self.editing.as_ref().is_some_and(|e| {
            e.path == path && e.sheet == self.sheet && e.column.is_some() == column.is_some()
        });
        if !editing_here {
            // Showing the cell, or the column's formula. Clicking into it
            // starts editing.
            let (mut shown, interactive) = match &column {
                Some(c) => (self.column_text(c), self.column_refusal(c).is_none()),
                None => (self.cell_text(&path), self.can_edit(&path)),
            };
            let response = ui.add(
                egui::TextEdit::singleline(&mut shown)
                    .id(id)
                    .desired_width(f32::INFINITY)
                    .interactive(interactive),
            );
            named(&response, "Formula bar");
            if response.gained_focus() || response.clicked() {
                if column.is_some() {
                    self.begin_editing_column();
                } else {
                    self.begin_editing(&path, false, None);
                }
            }
            return;
        }
        let popup_open = !self.completion.items.is_empty();
        self.process_edit_keys(ui, popup_open);
        let Some(mut editing) = self.editing.take() else {
            return;
        };
        let output = egui::TextEdit::multiline(&mut editing.text)
            .id(id)
            .desired_rows(1)
            .desired_width(f32::INFINITY)
            .code_editor()
            .show(ui);
        named(&output.response, "Formula bar");
        self.finish_text_widget(ui, id, &mut editing, &output);
        let rect = output.response.rect;
        self.editing = Some(editing);
        self.update_completion();
        self.completion_popup(ui, rect);
    }

    /// Enter enters, Esc cancels, and the arrows, Tab and Enter pick from the
    /// popup of names when it is open. Consumed before the text field sees them.
    fn process_edit_keys(&mut self, ui: &mut egui::Ui, popup_open: bool) {
        if popup_open {
            let count = self.completion.items.len();
            if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowDown)) {
                self.completion.selected = (self.completion.selected + 1) % count;
            }
            if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowUp)) {
                self.completion.selected = (self.completion.selected + count - 1) % count;
            }
            let accept = ui.input_mut(|i| {
                i.consume_key(Modifiers::NONE, Key::Tab)
                    || i.consume_key(Modifiers::NONE, Key::Enter)
            });
            if accept {
                self.accept_completion(self.completion.selected);
                return;
            }
            if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
                self.completion.dismissed_for = self.editing.as_ref().map(|e| e.text.clone());
                self.completion.items.clear();
                return;
            }
        }
        if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) {
            self.commit_editing();
        } else if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            self.cancel_editing();
        }
    }

    /// After a text field was shown for the text being edited: focus, the
    /// cursor, and whether the text changed.
    fn finish_text_widget(
        &mut self,
        ui: &egui::Ui,
        id: egui::Id,
        editing: &mut Editing,
        output: &egui::text_edit::TextEditOutput,
    ) {
        let wanted = if id == egui::Id::unique("formula_bar_text") {
            Focus::Bar
        } else {
            Focus::Cell
        };
        if editing.focus == wanted {
            output.response.request_focus();
            editing.focus = Focus::None;
        }
        if output.response.changed() {
            editing.changed = true;
        }
        if let Some(chars) = editing.set_cursor.take() {
            let mut state = output.state.clone();
            state
                .cursor
                .set_char_range(Some(egui::text::CCursorRange::one(
                    egui::text::CCursor::new(chars),
                )));
            state.store(ui.ctx(), id);
            editing.cursor = chars;
        } else if let Some(range) = output.cursor_range {
            editing.cursor = range.primary.index.into();
        }
    }

    fn completion_popup(&mut self, ui: &mut egui::Ui, below: egui::Rect) {
        if self.completion.items.is_empty() {
            return;
        }
        let mut accepted = None;
        egui::Area::new(egui::Id::unique("completion_popup"))
            .order(egui::Order::Foreground)
            .fixed_pos(below.left_bottom())
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_min_width(300.0);
                    for (i, item) in self.completion.items.iter().take(10).enumerate() {
                        let text = format!("{}    {}", item.label, item.detail);
                        let response = ui.selectable_label(i == self.completion.selected, text);
                        if response.clicked() {
                            accepted = Some(i);
                        }
                    }
                });
            });
        if let Some(i) = accepted {
            self.accept_completion(i);
        }
    }

    // -------------------------------------------------------- the grid

    fn grid(&mut self, ui: &mut egui::Ui) {
        let rows = std::mem::take(&mut self.rows);
        let mut editing = self.editing.take();
        let mut actions: Vec<Action> = Vec::new();
        let scroll_to = self.scroll_to.take();
        {
            let row_height = crate::style::row_height(ui);
            let mut table = TableBuilder::new(ui)
                .striped(true)
                .resizable(true)
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .max_scroll_height(f32::INFINITY)
                .column(Column::initial(300.0).at_least(120.0).clip(true))
                .column(Column::initial(130.0).at_least(60.0).clip(true))
                .column(Column::initial(260.0).at_least(80.0).clip(true))
                .column(Column::remainder().at_least(80.0).clip(true));
            if let Some(index) = scroll_to {
                table = table.scroll_to_row(index, Some(egui::Align::Center));
            }
            table
                .header(HEADER_HEIGHT, |mut header| {
                    for title in ["Name", "Type", "Value", "Formula"] {
                        header.col(|ui| {
                            column_title(ui, title);
                        });
                    }
                })
                .body(|body| {
                    body.rows(row_height, rows.len(), |mut table_row| {
                        let row = &rows[table_row.index()];
                        let selected = self.selection.as_ref() == Some(&row.path);
                        table_row.set_selected(selected);
                        self.draw_row(&mut table_row, row, selected, &mut editing, &mut actions);
                    });
                });
        }
        self.rows = rows;
        self.editing = editing;
        for action in actions {
            self.apply(action);
        }
    }

    pub(crate) fn apply(&mut self, action: Action) {
        match action {
            Action::Select(path) => {
                self.select(&path);
            }
            Action::SelectColumn(array, name) => self.select_column(&array, &name),
            Action::Toggle(path) => {
                let open = !self.is_open(&path);
                self.set_open(&path, open);
            }
            Action::BeginEdit(path) => {
                if self.select(&path) {
                    self.begin_editing(&path, true, None);
                }
            }
            Action::EnterText(path, text) => {
                self.select(&path);
                self.enter_directly(&path, &text);
            }
            Action::AddItem(path) => self.insert_item(&path, None),
            Action::InsertAbove(path) => {
                if let (Some(parent), Some(Segment::Index(i))) =
                    (path.parent(), path.segments().last())
                {
                    self.insert_item(&parent, Some(*i));
                }
            }
            Action::Delete(path) => self.delete_node(&path),
            Action::Commit => {
                self.commit_editing();
            }
            Action::Cancel => self.cancel_editing(),
            Action::Reveal(path) => self.reveal(&path),
            Action::ToggleTabular(array, on) => self.set_tabular(&array, on),
            Action::Reverse(array) => self.reverse_rows(&array),
            Action::SortBy(array, column, descending) => {
                self.sort_table(&array, &column, descending);
            }
            Action::HideColumn(array, column) => self.hide_column(&array, &column),
            Action::ShowColumn(array, column) => self.show_column(&array, &column),
            Action::FilterDialog(array, column) => self.open_filter_dialog(&array, &column),
            Action::ClearFilter(array, column) => self.set_filter(&array, &column, None),
            Action::ClearAllFilters(array) => self.clear_all_filters(&array),
            Action::ReapplyFilters(array) => self.reapply_filters(&array),
            Action::AddRow(array, at) => self.add_row(&array, at),
            Action::DeleteRow(array, index) => self.delete_row(&array, index),
            Action::MoveRow(array, index, up) => self.move_row(&array, index, up),
            Action::HideRow(array, index) => self.hide_row(&array, index),
            Action::ShowRow(array, id) => self.show_row(&array, id),
            Action::ShowAllHidden(array) => self.show_all_hidden(&array),
            Action::ShowRefusedRows => self.show_refused_rows(),
            Action::DismissRefusal => self.tables.refused = None,
            Action::AddField(..)
            | Action::RenameField(..)
            | Action::DeleteField(..)
            | Action::ReorderFields(..) => self.begin_field_command(action),
        }
    }

    fn draw_row(
        &self,
        table_row: &mut egui_extras::TableRow<'_, '_>,
        row: &Row,
        selected: bool,
        editing: &mut Option<Editing>,
        actions: &mut Vec<Action>,
    ) {
        let calc = self.calc();
        let cell = calc.cell(&self.sheet, &row.path);
        let formula = cell.as_ref().and_then(|c| c.formula);
        let formula_error = cell.as_ref().and_then(|c| c.error);
        let computed_by = cell.as_ref().and_then(|c| c.computed_by.clone());

        // The name.
        let mut name_response = None;
        table_row.col(|ui| {
            ui.add_space(row.depth as f32 * INDENT);
            if row.expandable() {
                let (rect, response) = ui.allocate_exact_size(
                    egui::vec2(14.0, crate::style::row_height(ui) - 4.0),
                    Sense::CLICK,
                );
                paint_triangle(ui, rect, row.expanded, response.hovered());
                if response.clicked() {
                    actions.push(Action::Toggle(row.path.clone()));
                }
            } else {
                ui.add_space(14.0 + ui.spacing().item_spacing.x);
            }
            let mut text = RichText::new(&row.name);
            if row.unrecognised {
                text = text.weak().italics();
            } else if row.depth == 0 {
                text = text.strong();
            }
            if row.required && row.node.is_none() {
                text = text.color(error_color(ui));
            }
            let response = ui.add(egui::Label::new(text).sense(Sense::CLICK).truncate());
            if response.clicked() {
                actions.push(Action::Select(row.path.clone()));
            }
            if response.double_clicked() && row.expandable() {
                actions.push(Action::Toggle(row.path.clone()));
            }
            name_response = Some(response);
        });

        // The type, as a badge in the colour of its kind.
        table_row.col(|ui| {
            let kind = row
                .ty
                .zip(calc.spec(&self.sheet))
                .map(|(ty, spec)| &spec.get(ty).kind);
            let response = match kind {
                Some(kind) => {
                    let colours = Palette::of_ui(ui).badge(kind);
                    crate::style::badge_label(ui, &row.type_label, colours)
                }
                None => ui.add(
                    egui::Label::new(RichText::new(&row.type_label).weak().small())
                        .sense(Sense::CLICK)
                        .truncate(),
                ),
            };
            if response.clicked() {
                actions.push(Action::Select(row.path.clone()));
            }
        });

        // The value.
        table_row.col(|ui| self.value_column(ui, row, selected, editing, actions));

        // The formula.
        table_row.col(|ui| {
            if let Some(text) = formula {
                let response = ui.add(
                    egui::Label::new(RichText::new(format!("={text}")).monospace())
                        .sense(Sense::CLICK)
                        .truncate(),
                );
                if response.clicked() {
                    actions.push(Action::Select(row.path.clone()));
                }
                if let Some(error) = formula_error {
                    response.on_hover_text(format!("{} {}", error.kind.code(), error.message));
                }
            } else if let Some(by) = &computed_by {
                ui.label(
                    RichText::new(format!("computed by {}", self.name_of(by)))
                        .weak()
                        .italics(),
                );
            }
        });

        // Right click.
        if let Some(response) = name_response {
            response.context_menu(|ui| self.row_menu(ui, row, actions));
        }
    }

    /// The value of a node, with the marks a cell can have: the tint of a
    /// computed value, the outline of invalid data, the editor while it is
    /// being typed into, and the marker of a calculation that is waiting.
    pub(crate) fn value_column(
        &self,
        ui: &mut egui::Ui,
        row: &Row,
        selected: bool,
        editing: &mut Option<Editing>,
        actions: &mut Vec<Action>,
    ) {
        let calc = self.calc();
        let cell = calc.cell(&self.sheet, &row.path);
        let formula = cell.as_ref().and_then(|c| c.formula);
        let formula_error = cell.as_ref().and_then(|c| c.error);
        let pending = cell.as_ref().is_some_and(|c| c.pending);
        let untrusted = cell.as_ref().is_some_and(|c| c.untrusted);
        let stale = cell.as_ref().is_some_and(|c| c.stale);
        let computed_by = cell.as_ref().and_then(|c| c.computed_by.clone());
        let value = row.node.as_ref().map(|n| n.value());
        let is_error = matches!(value, Some(Value::Error(_)));
        let editable = !row.unrecognised && computed_by.is_none();

        // What is wrong with the data, for the outline.
        let invalid = match (&row.node, row.ty, calc.spec(&self.sheet)) {
            (Some(node), Some(ty), Some(spec)) if row.kind == RowKind::Scalar && !is_error => {
                !cliotree_core::validate::validate_at(spec, ty, node, &row.path).is_empty()
            }
            _ => false,
        };
        let missing = row.node.is_none() && row.required && !row.unrecognised;

        if is_error {
            ui.painter()
                .rect_filled(ui.max_rect(), 0.0, Palette::of_ui(ui).error_fill);
        } else if computed_by.is_some() || formula.is_some() {
            ui.painter()
                .rect_filled(ui.max_rect(), 0.0, computed_fill(ui));
        }
        if invalid || missing {
            outline_invalid(ui, ui.max_rect());
        }
        let editing_here = editing
            .as_ref()
            .is_some_and(|e| e.path == row.path && e.in_cell && e.sheet == self.sheet);
        if editing_here {
            let numeric = row
                .ty
                .zip(self.calc().spec(&self.sheet))
                .is_some_and(|(ty, spec)| {
                    matches!(spec.get(ty).kind, Kind::Integer(_) | Kind::Number(_))
                });
            self.cell_editor(ui, editing, actions, numeric);
            return;
        }
        // Not trusted: the value is the one that was stored, greyed when what
        // it came from has changed since.
        ui.scope(|ui| {
            if stale {
                ui.visuals_mut().override_text_color = Some(Palette::of_ui(ui).stale);
            }
            self.value_cell(
                ui,
                row,
                value,
                formula,
                formula_error,
                editable,
                selected,
                actions,
            );
        });
        if untrusted && formula.is_some() {
            ui.label(RichText::new("#UNTRUSTED!").color(error_color(ui)).small())
                .on_hover_text(if stale {
                    "This workbook is not trusted, so the formula was not run. The value is \
                     out of date: something it uses has changed."
                } else {
                    "This workbook is not trusted, so the formula was not run. The value is \
                     the one that was stored."
                });
        }
        if pending {
            crate::style::pending_marker(ui).on_hover_text("Waiting to be calculated");
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn value_cell(
        &self,
        ui: &mut egui::Ui,
        row: &Row,
        value: Option<&Value>,
        formula: Option<&str>,
        formula_error: Option<&cliotree_core::ErrorValue>,
        editable: bool,
        selected: bool,
        actions: &mut Vec<Action>,
    ) {
        let spec = self.calc().spec(&self.sheet);
        let kind = row.ty.zip(spec).map(|(ty, spec)| &spec.get(ty).kind);
        match (value, kind) {
            (Some(Value::Bool(b)), Some(Kind::Bool)) if editable && formula.is_none() => {
                let mut checked = *b;
                if ui
                    .checkbox(&mut checked, if *b { "True" } else { "False" })
                    .changed()
                {
                    actions.push(Action::EnterText(row.path.clone(), checked.to_string()));
                }
            }
            (Some(Value::Str(current)), Some(Kind::Enum { values, .. }))
                if editable && formula.is_none() && selected =>
            {
                let mut chosen = current.clone();
                egui::ComboBox::from_id_salt(("enum", row.path.to_string()))
                    .selected_text(current)
                    .show_ui(ui, |ui| {
                        for option in values {
                            ui.selectable_value(&mut chosen, option.clone(), option);
                        }
                    });
                if &chosen != current {
                    actions.push(Action::EnterText(row.path.clone(), chosen));
                }
            }
            (Some(value), _) => {
                let mut text = RichText::new(display(value));
                if matches!(value, Value::Error(_)) {
                    text = text.color(error_color(ui)).strong();
                } else if row.unrecognised {
                    text = text.weak().italics();
                }
                let label = |ui: &mut egui::Ui| {
                    ui.add(egui::Label::new(text).sense(Sense::CLICK).truncate())
                };
                // Numbers line up on the right.
                let numeric = matches!(value, Value::Int(_) | Value::Float(_))
                    || matches!(kind, Some(Kind::Integer(_) | Kind::Number(_)));
                let response = if numeric {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), label)
                        .inner
                } else {
                    label(ui)
                };
                if response.clicked() {
                    actions.push(Action::Select(row.path.clone()));
                }
                if response.double_clicked() && editable && row.kind == RowKind::Scalar {
                    actions.push(Action::BeginEdit(row.path.clone()));
                }
                if response.double_clicked() && row.kind != RowKind::Scalar && !row.tabular {
                    actions.push(Action::Reveal(row.path.clone()));
                }
                if let Value::Error(error) = value {
                    let trace = self
                        .calc()
                        .cell(&self.sheet, &row.path)
                        .and_then(|c| c.traceback.map(str::to_owned));
                    response.on_hover_ui(|ui| {
                        ui.label(format!("{} {}", error.kind.code(), error.message));
                        if let Some(trace) = trace {
                            ui.add(egui::Label::new(RichText::new(trace).monospace().small()));
                        }
                    });
                } else if let Some(error) = formula_error {
                    response.on_hover_text(format!("{} {}", error.kind.code(), error.message));
                }
            }
            (None, _) => {
                let text = if row.required && !row.unrecognised {
                    RichText::new("(missing)").italics().color(error_color(ui))
                } else {
                    RichText::new("")
                };
                let response = ui.add(egui::Label::new(text).sense(Sense::CLICK));
                let response = ui
                    .interact(
                        ui.max_rect(),
                        ui.scope_id().with(("empty", row.path.to_string())),
                        Sense::CLICK,
                    )
                    .union(response);
                if response.clicked() {
                    actions.push(Action::Select(row.path.clone()));
                }
                if response.double_clicked() && editable && row.kind == RowKind::Scalar {
                    actions.push(Action::BeginEdit(row.path.clone()));
                }
            }
        }
    }

    /// The text field in a cell being edited. Enter enters it and Esc drops it.
    fn cell_editor(
        &self,
        ui: &mut egui::Ui,
        editing: &mut Option<Editing>,
        actions: &mut Vec<Action>,
        numeric: bool,
    ) {
        let Some(e) = editing.as_mut() else {
            return;
        };
        let id = egui::Id::unique(("cell_editor", e.path.to_string()));
        let enter = ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
        let escape = ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
        let align = if numeric {
            egui::Align2::RIGHT_CENTER
        } else {
            egui::Align2::LEFT_CENTER
        };
        let output = egui::TextEdit::singleline(&mut e.text)
            .id(id)
            .align(align)
            .desired_width(f32::INFINITY)
            .show(ui);
        named(&output.response, "Cell editor");
        if e.focus == Focus::Cell {
            output.response.request_focus();
            e.focus = Focus::None;
        }
        if output.response.changed() {
            e.changed = true;
        }
        if let Some(range) = output.cursor_range {
            e.cursor = range.primary.index.into();
        }
        if enter {
            actions.push(Action::Commit);
        } else if escape {
            actions.push(Action::Cancel);
        }
    }

    fn row_menu(&self, ui: &mut egui::Ui, row: &Row, actions: &mut Vec<Action>) {
        let path = &row.path;
        if row.expandable() {
            let label = if row.expanded { "Collapse" } else { "Expand" };
            if ui.button(label).clicked() {
                actions.push(Action::Toggle(path.clone()));
                ui.close();
            }
        }
        if row.kind == RowKind::Array && ui.button("Add item").clicked() {
            actions.push(Action::AddItem(path.clone()));
            ui.close();
        }
        if row.kind == RowKind::Array {
            if self.can_tabulate(path) {
                let label = if row.tabular {
                    "Tabular view (on)"
                } else {
                    "Tabular view"
                };
                if ui.button(label).clicked() {
                    actions.push(Action::ToggleTabular(path.clone(), !row.tabular));
                    ui.close();
                }
            }
            let reorderable = self.can_reorder(path);
            let reverse = ui.add_enabled(reorderable, egui::Button::new("Reverse row order"));
            if reverse.clicked() {
                actions.push(Action::Reverse(path.clone()));
                ui.close();
            }
        }
        match path.segments().last() {
            Some(Segment::Index(_)) => {
                if ui.button("Insert item above").clicked() {
                    actions.push(Action::InsertAbove(path.clone()));
                    ui.close();
                }
                if ui.button("Delete item").clicked() {
                    actions.push(Action::Delete(path.clone()));
                    ui.close();
                }
            }
            Some(Segment::Key(_)) if row.node.is_some() && (!row.required || row.unrecognised) => {
                let label = if row.unrecognised {
                    "Delete"
                } else {
                    "Delete (leave absent)"
                };
                if ui.button(label).clicked() {
                    actions.push(Action::Delete(path.clone()));
                    ui.close();
                }
            }
            _ => {}
        }
        let add_to = match row.kind {
            RowKind::Object | RowKind::Array => self.object_of_row(path),
            _ => None,
        };
        self.field_menu_items(ui, add_to.as_ref(), self.field_of(path).as_ref(), actions);
    }

    // ------------------------------------------------- tabs and status

    fn tab_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let names: Vec<String> = self.calc().sheet_names().map(str::to_owned).collect();
            ui.spacing_mut().item_spacing.x = 2.0;
            for name in names {
                let selected = name == self.sheet;
                let response = sheet_tab(ui, &name, selected);
                if response.clicked() {
                    self.switch_to(&name);
                }
                if response.double_clicked() {
                    self.begin_rename_tab(&name);
                }
                response.context_menu(|ui| {
                    if ui.button("Rename tab...").clicked() {
                        self.begin_rename_tab(&name);
                        ui.close();
                    }
                });
            }
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(evaluation_text(&self.session.status()));
            ui.separator();
            ui.label(problems_text(self.summary.errors, self.summary.invalid));
            if !self.calc().is_trusted() {
                ui.separator();
                let stale = self.calc().stale();
                let text = if stale == 0 {
                    "Not trusted".to_owned()
                } else {
                    format!("Not trusted, {stale} stale")
                };
                ui.colored_label(error_color(ui), text);
            }
            let failed = self.calc().module_issues().len();
            if failed > 0 {
                ui.separator();
                let text = if failed == 1 {
                    "1 module failed".to_owned()
                } else {
                    format!("{failed} modules failed")
                };
                ui.colored_label(error_color(ui), text);
            }
            if self.calc().is_modified() {
                ui.separator();
                ui.label("Modified");
            }
        });
    }

    fn issues_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Issues");
        let issues = self.issue_list();
        let modules = self.calc().module_issues();
        if issues.is_empty() && modules.is_empty() {
            ui.label("No errors, no invalid data.");
            return;
        }
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (module, why) in modules {
                ui.colored_label(error_color(ui), format!("Module {module}: {why}"));
            }
            for (path, text) in issues {
                if ui
                    .selectable_label(false, format!("{}  {text}", self.name_of(&path)))
                    .clicked()
                {
                    self.reveal(&path);
                }
            }
        });
    }

    fn insert_function_window(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.insert_function.take() else {
            return;
        };
        let mut open = true;
        let mut insert: Option<&'static str> = None;
        egui::Window::new("Insert Function")
            .open(&mut open)
            .default_size([520.0, 380.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Search");
                    let response = ui.text_edit_singleline(&mut dialog.search);
                    named(&response, "Search functions");
                });
                ui.separator();
                let found = ClioApp::functions_matching(&dialog.search);
                ui.columns(2, |columns| {
                    egui::ScrollArea::vertical()
                        .id_salt("functions")
                        .show(&mut columns[0], |ui| {
                            for group in Group::ALL {
                                let members: Vec<_> =
                                    found.iter().filter(|f| f.group == group).collect();
                                if members.is_empty() {
                                    continue;
                                }
                                ui.strong(group.title());
                                for f in members {
                                    let chosen = dialog.selected == Some(f.name);
                                    let response = ui.selectable_label(chosen, f.name);
                                    if response.clicked() {
                                        dialog.selected = Some(f.name);
                                    }
                                    if response.double_clicked() {
                                        insert = Some(f.insert);
                                    }
                                }
                            }
                        });
                    let ui = &mut columns[1];
                    match found.iter().find(|f| Some(f.name) == dialog.selected) {
                        Some(f) => {
                            ui.label(RichText::new(f.signature).monospace().strong());
                            ui.label(f.description);
                            if ui.button("Insert").clicked() {
                                insert = Some(f.insert);
                            }
                        }
                        None => {
                            ui.label("Choose a function to see what it does.");
                        }
                    }
                });
            });
        if let Some(text) = insert {
            self.insert_function_text(text);
            return;
        }
        if open {
            self.insert_function = Some(dialog);
        }
    }
}

/// The title of a grid column.
pub(crate) fn column_title(ui: &mut egui::Ui, title: &str) -> egui::Response {
    let colour = Palette::of_ui(ui).text_weak;
    ui.label(
        crate::style::medium(title)
            .size(crate::style::SMALL_SIZE)
            .color(colour),
    )
}

/// A sheet's tab at the bottom of the window. The current sheet's tab has the
/// panel's fill, as if the sheet above went on into it, and an accent line.
fn sheet_tab(ui: &mut egui::Ui, name: &str, selected: bool) -> egui::Response {
    let palette = Palette::of_ui(ui);
    let text = if selected {
        crate::style::medium(name).color(palette.text)
    } else {
        RichText::new(name).color(palette.text_weak)
    };
    let galley = egui::WidgetText::from(text).into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        egui::TextStyle::Button,
    );
    let size = egui::vec2(galley.size().x + 20.0, 30.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::CLICK);
    if ui.is_rect_visible(rect) {
        let fill = if selected {
            palette.panel
        } else if response.hovered() {
            palette.hover
        } else {
            Color32::TRANSPARENT
        };
        let radius = egui::CornerRadius {
            nw: 0,
            ne: 0,
            sw: crate::style::CONTROL_RADIUS,
            se: crate::style::CONTROL_RADIUS,
        };
        ui.painter().rect_filled(rect, radius, fill);
        if selected {
            let line =
                egui::Rect::from_min_max(rect.left_top(), rect.right_top() + egui::vec2(0.0, 2.0));
            ui.painter().rect_filled(line, 0.0, palette.accent);
        }
        ui.painter()
            .galley(rect.center() - galley.size() / 2.0, galley, palette.text);
    }
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, selected, name));
    response
}

fn paint_triangle(ui: &egui::Ui, rect: egui::Rect, open: bool, hovered: bool) {
    let color = if hovered {
        ui.visuals().strong_text_color()
    } else {
        ui.visuals().text_color()
    };
    let c = rect.center();
    let r = 4.0;
    let points = if open {
        vec![
            egui::pos2(c.x - r, c.y - r * 0.5),
            egui::pos2(c.x + r, c.y - r * 0.5),
            egui::pos2(c.x, c.y + r * 0.7),
        ]
    } else {
        vec![
            egui::pos2(c.x - r * 0.5, c.y - r),
            egui::pos2(c.x - r * 0.5, c.y + r),
            egui::pos2(c.x + r * 0.7, c.y),
        ]
    };
    ui.painter()
        .add(egui::Shape::convex_polygon(points, color, Stroke::NONE));
}
