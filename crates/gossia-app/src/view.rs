//! Drawing the window: the menu, the formula bar, the grid, the tabs, the
//! status bar, and the Insert Function dialog.

use gossia_i18n::tr;
use std::time::Duration;

use egui::{Color32, Key, KeyboardShortcut, Modifiers, RichText, Sense, Stroke, StrokeKind};
use egui_extras::{Column, TableBuilder};
use gossia_calc::python::catalog::Group;
use gossia_core::{Kind, Path, Segment, Value};

use crate::app::{Editing, Focus, GossiaApp};
use crate::clipboard::{CopyMode, PasteMode};
use crate::rows::{Row, RowKind, display};
use crate::status::{evaluation_text, problems_text};

pub(crate) use crate::style::ROW_HEIGHT;
use crate::style::{HEADER_HEIGHT, Palette};

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
#[derive(Debug)]
pub(crate) enum Action {
    Select(Path),
    /// Shift and a click: the range from the selection to here.
    Extend(Path),
    Copy(CopyMode),
    Paste(PasteMode),
    /// Runs the formula at the path again, alone.
    RecalculateFormula(Path),
    /// Removes the formula at the path, keeping what it gave as data.
    ConvertToValues(Path),
    Toggle(Path),
    BeginEdit(Path),
    EnterText(Path, String),
    AddItem(Path),
    InsertAbove(Path),
    /// Adds an item holding the element named to an ordered list of child
    /// elements, at the end or at an index.
    AddElement(Path, Option<usize>, String),
    // The entries of a map: the dialog asks for the key.
    AddEntry(Path),
    RenameKey(Path),
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
    ShowRow(Path, gossia_core::NodeId),
    ShowAllHidden(Path),
    // Hiding in the tree view.
    HideNode(Path),
    ShowNode(Path),
    ShowAllNodes(Path),
    ShowRefusedRows,
    DismissRefusal,
    // Changing the schema from this tab.
    AddField(Path),
    /// *Force type definition*: the field's type from its formula's result.
    ForceType(Path),
    /// *Go to schema definition*: the schema tab, at where the node is defined.
    GoToDefinition(Path),
    RenameField(Path, String),
    DeleteField(Path, String),
    ReorderFields(Path, String, String),
    // Comments, on XML tabs: the box opens at the place given.
    EditComment(Path, egui::Pos2),
    DeleteComment(Path),
}

impl GossiaApp {
    /// One frame of the window.
    pub fn show(&mut self, ui: &mut egui::Ui) {
        crate::style::prepare(ui.ctx());
        self.apply_appearance(ui.ctx());
        self.apply_settings();
        // The report shown runs its expressions; no other does.
        let report = self.in_report().then(|| self.sheet.clone());
        self.session.calc_mut().show_report(report.as_deref());
        self.session.tick();
        self.collect_procedures();
        self.follow_tabs();
        let now = ui.input(|i| i.time);
        self.sync_code(now);
        self.watch_journals(ui.ctx());
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
        if self.show_toolbar {
            egui::Panel::top("toolbar")
                .frame(bar.inner_margin(egui::Margin::symmetric(8, 6)))
                .show(ui, |ui| self.toolbar(ui));
        }
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
        // The issues and the inspector share the right side, the issues on
        // top.
        let inspector = self.show_inspector && !form;
        if self.show_issues || inspector {
            egui::Panel::right("issues")
                .default_size(300.0)
                .show(ui, |ui| {
                    if self.show_issues && inspector {
                        egui::Panel::top("issues_part")
                            .resizable(true)
                            .default_size(260.0)
                            .show(ui, |ui| self.issues_panel(ui));
                        self.inspector_panel(ui);
                    } else if inspector {
                        self.inspector_panel(ui);
                    } else {
                        self.issues_panel(ui);
                    }
                });
        }
        if form && self.layout_active() {
            egui::Panel::right("form_properties")
                .resizable(true)
                .default_size(320.0)
                .show(ui, |ui| self.property_page(ui));
        }
        if form {
            egui::CentralPanel::default().show(ui, |ui| {
                if self.in_report() {
                    self.report_view(ui);
                } else if self.in_form() {
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
        self.key_window(ui.ctx());
        self.tab_windows(ui.ctx());
        self.file_windows(ui.ctx());
        self.options_window(ui.ctx());
        self.sources_window(ui.ctx());
        self.comment_window(ui.ctx());
        self.replace_inside_window(ui.ctx());
        self.disconnect_window(ui.ctx());
        self.drop_hint(ui.ctx());
        self.receive_picked();
        self.sync_sources();
        self.sync_host_modules();
        self.forward_fetches();
        self.code_window(ui.ctx());
    }

    // ------------------------------------------------------------- keys

    fn handle_keys(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if let Some(text) = self.tables.to_clipboard.take() {
            ctx.copy_text(text);
        }
        let typing_somewhere = ctx.text_edit_focused();
        if self.picking() && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            self.cancel_pick();
        }
        let undo = KeyboardShortcut::new(Modifiers::COMMAND, Key::Z);
        let redo_y = KeyboardShortcut::new(Modifiers::COMMAND, Key::Y);
        let redo_z = KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z);
        // The file commands work wherever the keyboard is.
        let save_as = KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::S);
        let save = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
        let open = KeyboardShortcut::new(Modifiers::COMMAND, Key::O);
        let new = KeyboardShortcut::new(Modifiers::COMMAND, Key::N);
        let code_editor = KeyboardShortcut::new(Modifiers::ALT, Key::F11);
        if ui.input_mut(|i| i.consume_shortcut(&code_editor)) {
            self.open_code_editor();
        }
        let recalculate = KeyboardShortcut::new(Modifiers::NONE, Key::F9);
        if ui.input_mut(|i| i.consume_shortcut(&recalculate)) {
            self.recalculate();
        }
        // In a host only Save is the application's; the host opens, makes and
        // names files.
        let hosted = crate::host::current().is_some();
        if !hosted && ui.input_mut(|i| i.consume_shortcut(&save_as)) {
            self.begin_save_as();
        } else if ui.input_mut(|i| i.consume_shortcut(&save)) {
            self.begin_save();
        } else if !hosted && ui.input_mut(|i| i.consume_shortcut(&open)) {
            self.begin_open();
        } else if !hosted && ui.input_mut(|i| i.consume_shortcut(&new)) {
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
        // In layout mode the arrows move the selected controls of Free groups.
        if !typing_somewhere && self.options.is_none() && self.layout_active() {
            self.layout_keys(ui);
        }
        // While the Options dialog is open, keys are for it, not the grid.
        if typing_somewhere
            || self.editing.is_some()
            || !self.in_tree()
            || self.options.is_some()
            || self.comment_edit.is_some()
        {
            return;
        }
        self.handle_clipboard(ui);
        if self.table_cell_selected() {
            let key = |ui: &mut egui::Ui, k| ui.input_mut(|i| i.consume_key(Modifiers::SHIFT, k));
            let dx = isize::from(key(ui, Key::ArrowRight)) - isize::from(key(ui, Key::ArrowLeft));
            let dy = isize::from(key(ui, Key::ArrowDown)) - isize::from(key(ui, Key::ArrowUp));
            if dx != 0 || dy != 0 {
                self.table_extend_selection(dx, dy);
            }
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
                if right && GossiaApp::is_container(&row) {
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

    /// Ctrl+C and Ctrl+V on the selected cells, which egui gives as events.
    fn handle_clipboard(&mut self, ui: &mut egui::Ui) {
        if self.selection.is_none() {
            return;
        }
        let (copy, pasted) = ui.input_mut(|i| {
            let mut copy = false;
            let mut pasted = None;
            i.events.retain(|e| match e {
                egui::Event::Copy => {
                    copy = true;
                    false
                }
                egui::Event::Paste(text) => {
                    pasted = Some(text.clone());
                    false
                }
                _ => true,
            });
            (copy, pasted)
        });
        if copy {
            self.copy_cells(CopyMode::Cells);
        }
        if let Some(text) = pasted {
            // In a host the cells' clipboard is the application's own.
            #[cfg(target_family = "wasm")]
            let _ = (text, self.paste_from_clipboard(PasteMode::Cells));
            #[cfg(not(target_family = "wasm"))]
            let _ = self.paste_clip(&text, PasteMode::Cells);
        }
        // egui makes clipboard events only of Ctrl+C and Ctrl+V; with Shift
        // they are keys, and the clipboard is read here.
        let shift = Modifiers::COMMAND | Modifiers::SHIFT;
        if ui.input_mut(|i| i.consume_key(shift, Key::C)) {
            self.copy_cells(CopyMode::Formulas);
        }
        if ui.input_mut(|i| i.consume_key(shift, Key::V)) {
            let _ = self.paste_from_clipboard(PasteMode::Values);
        }
    }

    // ------------------------------------------------------------- menu

    fn menu(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button(tr!("File"), |ui| {
                // In a host the host opens, saves and closes the file.
                let hosted = crate::host::current().is_some();
                if !hosted && ui.button(tr!("New")).clicked() {
                    self.begin_new();
                    ui.close();
                }
                if !hosted && ui.button(tr!("Open...")).clicked() {
                    self.begin_open();
                    ui.close();
                }
                if !hosted {
                    self.recent_menu(ui);
                }
                if ui
                    .add_enabled(self.editing.is_none(), egui::Button::new(tr!("Import...")))
                    .clicked()
                {
                    self.begin_import();
                    ui.close();
                }
                // A report is exported as a PDF, by the item after this one.
                if ui
                    .add_enabled(
                        self.editing.is_none() && !self.in_report(),
                        egui::Button::new(tr!("Export...")),
                    )
                    .clicked()
                {
                    let tab = self.sheet.clone();
                    self.begin_export(&tab);
                    ui.close();
                }
                if ui
                    .add_enabled(
                        self.editing.is_none() && self.in_report(),
                        egui::Button::new(tr!("Export as PDF...")),
                    )
                    .on_disabled_hover_text(tr!("Show a report to export it as PDF"))
                    .clicked()
                {
                    let tab = self.sheet.clone();
                    self.begin_export_pdf(&tab);
                    ui.close();
                }
                if ui
                    .add_enabled(
                        self.editing.is_none() && self.in_report(),
                        egui::Button::new(tr!("Export as HTML...")),
                    )
                    .on_disabled_hover_text(tr!("Show a report to export it as HTML"))
                    .clicked()
                {
                    let tab = self.sheet.clone();
                    self.begin_export_html(&tab);
                    ui.close();
                }
                // A host has no printing: its PDF or HTML is saved, and
                // printed from the program that opens it.
                if crate::host::current().is_none()
                    && ui
                        .add_enabled(
                            self.editing.is_none() && self.in_report(),
                            egui::Button::new(tr!("Print...")),
                        )
                        .on_disabled_hover_text(tr!("Show a report to print it"))
                        .clicked()
                {
                    let tab = self.sheet.clone();
                    self.begin_print(&tab);
                    ui.close();
                }
                ui.separator();
                if ui
                    .add_enabled(self.editing.is_none(), egui::Button::new(tr!("Save")))
                    .clicked()
                {
                    self.begin_save();
                    ui.close();
                }
                if !hosted && ui.button(tr!("Save As...")).clicked() {
                    self.begin_save_as();
                    ui.close();
                }
                if !hosted {
                    ui.separator();
                    if ui.button(tr!("Quit")).clicked() {
                        let ctx = ui.ctx().clone();
                        self.request_quit(&ctx);
                        ui.close();
                    }
                }
            });
            ui.menu_button(tr!("Edit"), |ui| {
                let undo_label = self.calc().undo_label().map(str::to_owned);
                let redo_label = self.calc().redo_label().map(str::to_owned);
                // Undo and redo wait while a button procedure runs.
                let can_change =
                    self.editing.is_none() && self.session.running_procedure().is_none();
                let undo_text = match &undo_label {
                    Some(label) => tr!("Undo {label}", label = label),
                    None => tr!("Undo").to_owned(),
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
                    Some(label) => tr!("Redo {label}", label = label),
                    None => tr!("Redo").to_owned(),
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
                let sheet = self.sheet.clone();
                self.tab_menu_items(ui, &sheet);
            });
            ui.menu_button(tr!("View"), |ui| {
                // How the current tab is shown. A report has one view.
                let report = self.in_report();
                ui.add_enabled_ui(!report, |ui| {
                    ui.menu_button(tr!("Show as"), |ui| {
                        // The views every tab has, a line, then its forms.
                        let mut forms_begun = false;
                        for view in self.view_names() {
                            if matches!(view, crate::form::View::Form(_)) && !forms_begun {
                                forms_begun = true;
                                ui.separator();
                            }
                            let chosen = self.current_view() == &view;
                            if ui.radio(chosen, view.label()).clicked() {
                                self.set_view(view);
                                ui.close();
                            }
                        }
                    });
                    self.forms_menu(ui);
                });
                ui.menu_button(tr!("Font"), |ui| self.font_menu(ui));
                ui.separator();
                if ui
                    .checkbox(&mut self.show_toolbar, tr!("Toolbar"))
                    .clicked()
                {
                    ui.close();
                }
                if ui
                    .checkbox(&mut self.show_issues, tr!("Issues panel"))
                    .clicked()
                {
                    ui.close();
                }
                if ui
                    .checkbox(&mut self.show_inspector, tr!("Node inspector"))
                    .clicked()
                {
                    ui.close();
                }
                if ui
                    .add_enabled(!report, egui::Button::new(tr!("Collapse all")))
                    .clicked()
                {
                    let root = Path::root();
                    if let Some(set) = self.expanded.get_mut(&self.sheet) {
                        set.clear();
                        set.insert(root);
                    }
                    self.expansion_rev += 1;
                    ui.close();
                }
            });
            ui.menu_button(tr!("Tools"), |ui| {
                if ui
                    .add(egui::Button::new(tr!("Recalculate")).shortcut_text("F9"))
                    .on_hover_text(tr!(
                        "Run every formula again, reading the data sources afresh"
                    ))
                    .clicked()
                {
                    self.recalculate();
                    ui.close();
                }
                ui.separator();
                if ui
                    .add(egui::Button::new(tr!("Code editor")).shortcut_text("Alt+F11"))
                    .clicked()
                {
                    self.open_code_editor();
                    ui.close();
                }
                if ui.button(tr!("Data sources...")).clicked() {
                    self.open_sources();
                    ui.close();
                }
                if ui
                    .button(tr!("Disconnect from Data Sources..."))
                    .on_hover_text(tr!(
                        "Replace every read of a data source in the formulas and forms by what \
                         it gives now, so the workbook can go to someone without the sources"
                    ))
                    .clicked()
                {
                    ui.close();
                    self.begin_disconnect();
                }
                ui.separator();
                if ui.button(tr!("Options...")).clicked() {
                    self.begin_options(None);
                    ui.close();
                }
            });
        });
    }

    /// File > Open Recent: the file names, with the folder on hover.
    fn recent_menu(&mut self, ui: &mut egui::Ui) {
        let recent = self.user.prefs.recent.clone();
        ui.add_enabled_ui(!recent.is_empty(), |ui| {
            ui.menu_button(tr!("Open Recent"), |ui| {
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
                if ui.button(tr!("Clear the list")).clicked() {
                    self.change_prefs(|p| p.recent.clear());
                    ui.close();
                }
            });
        });
    }

    /// View > Font: the typeface and size of the Data tree and Raw views of
    /// every tab. A form's text has the font of its controls instead.
    fn font_menu(&mut self, ui: &mut egui::Ui) {
        ui.weak(tr!("Data tree and Raw views, every tab"));
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
                .radio(current.size == size, tr!("{size} pt", size = size))
                .clicked()
            {
                chosen.size = size;
            }
        }
        ui.separator();
        let default = crate::style::DataFont::default();
        if ui
            .add_enabled(current != default, egui::Button::new(tr!("Default font")))
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
            if ui
                .button(fx)
                .on_hover_text(tr!("Insert Function"))
                .clicked()
            {
                self.open_insert_function();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                self.expand_button(ui);
                self.pick_button(ui);
                self.bar_text(ui);
            });
        });
        self.formula_bar_handle(ui);
        if self.picking() {
            ui.label(RichText::new(crate::pick::pick_hint()).italics());
        } else if let Some(notice) = &self.notice {
            let color = error_color(ui);
            ui.colored_label(color, notice);
        }
    }

    /// What the formula of the node at `path` returned, when the node is of
    /// a simple type and refused it (`#TYPE!`), laid out in one line for the
    /// formula bar: at most a third of the bar wide, cut short with `…`.
    fn refused_galley(&self, ui: &egui::Ui, path: &Path) -> Option<std::sync::Arc<egui::Galley>> {
        let text = self.refused_text(path)?;
        let mut job = egui::text::LayoutJob::simple_singleline(
            text,
            egui::TextStyle::Body.resolve(ui.style()),
            error_color(ui),
        );
        job.wrap =
            egui::text::TextWrapping::truncate_at_width((ui.available_width() / 3.0).max(40.0));
        Some(ui.fonts_mut(|f| f.layout_job(job)))
    }

    /// What a formula returned for the node at `path` that the node, of a
    /// simple type, refused, as the formula bar and the Formula column show
    /// it: text in quotes, so that "12" and 12 are told apart, on one line,
    /// very long text cut before it is laid out (and then to the width).
    fn refused_text(&self, path: &Path) -> Option<String> {
        let value = self
            .session
            .calc()
            .refused_simple_value(&self.sheet, path)?;
        let text = match value {
            Value::Str(s) => format!("\"{s}\""),
            other => display(other),
        };
        Some(
            text.chars()
                .take(200)
                .map(|c| if c.is_control() { ' ' } else { c })
                .collect(),
        )
    }

    /// The text box of the formula bar: one line, or with `height` as tall
    /// as that, scrolling inside. `add` is given the rows to show.
    fn bar_box<R>(
        ui: &mut egui::Ui,
        height: Option<f32>,
        add: impl FnOnce(&mut egui::Ui, Option<usize>) -> R,
    ) -> R {
        let Some(height) = height else {
            return add(ui, None);
        };
        let line = Self::bar_line_height(ui);
        let margin = 2.0 * ui.spacing().button_padding.y + 4.0;
        let rows = ((height / line).floor() as usize).max(1);
        egui::ScrollArea::vertical()
            .id_salt("formula_bar_scroll")
            .max_height(height + margin)
            .min_scrolled_height(height + margin)
            .auto_shrink([false, false])
            .show(ui, |ui| add(ui, Some(rows)))
            .inner
    }

    /// A text field of the formula bar: one line, or a box of `rows` lines.
    fn bar_edit(text: &mut String, rows: Option<usize>) -> egui::TextEdit<'_> {
        match rows {
            None => egui::TextEdit::singleline(text).desired_width(f32::INFINITY),
            Some(rows) => egui::TextEdit::multiline(text)
                .desired_rows(rows)
                .desired_width(f32::INFINITY),
        }
    }

    /// The height of one line of the formula bar's text.
    fn bar_line_height(ui: &egui::Ui) -> f32 {
        ui.text_style_height(&egui::TextStyle::Monospace)
    }

    /// The tallest the formula bar can be made: half the window, and at
    /// least the lines it grows to while typing.
    fn bar_max_height(&self, ui: &egui::Ui) -> f32 {
        let line = Self::bar_line_height(ui);
        let lines = self.user.prefs.formula_bar_lines as f32 * line;
        (ui.ctx().content_rect().height() * 0.5).max(lines)
    }

    /// Expanded: as tall as the lines the bar grows to while typing (Options).
    /// Collapsed: one line.
    pub fn toggle_formula_bar(&mut self, ui: &egui::Ui) {
        self.formula_bar_height = match self.formula_bar_height {
            Some(_) => None,
            None => {
                let line = Self::bar_line_height(ui);
                Some(self.user.prefs.formula_bar_lines.max(2) as f32 * line)
            }
        };
    }

    /// The button at the right end of the formula bar that expands it to
    /// show a long or multi-line formula, and collapses it again.
    fn expand_button(&mut self, ui: &mut egui::Ui) {
        let expanded = self.formula_bar_height.is_some();
        let text = if crate::icons::ready(ui.ctx()) {
            RichText::new(if expanded {
                crate::icons::CARET_UP
            } else {
                crate::icons::CARET_DOWN
            })
            .family(egui::FontFamily::Name(crate::style::ICONS.into()))
        } else {
            RichText::new(if expanded { "^" } else { "v" })
        };
        let name = if expanded {
            tr!("Collapse formula bar")
        } else {
            tr!("Expand formula bar")
        };
        let response = ui.button(text).on_hover_text(name);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, name));
        if response.clicked() {
            self.toggle_formula_bar(ui);
        }
    }

    /// The edge under the formula bar: dragged, it makes the bar taller or
    /// shorter; double-clicked, it expands or collapses the bar.
    fn formula_bar_handle(&mut self, ui: &mut egui::Ui) {
        let width = ui.available_width();
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(width, 5.0), egui::Sense::click_and_drag());
        let response = response
            .on_hover_cursor(egui::CursorIcon::ResizeVertical)
            .on_hover_text(tr!("Drag to make the formula bar taller or shorter"));
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::Splitter, true, tr!("Formula bar height"))
        });
        if response.hovered() || response.dragged() {
            let palette = Palette::of_ui(ui);
            ui.painter().hline(
                rect.x_range(),
                rect.center().y,
                Stroke::new(2.0, palette.border),
            );
        }
        let line = Self::bar_line_height(ui);
        if response.dragged() {
            let now = self.formula_bar_height.unwrap_or(line);
            let height = (now + response.drag_delta().y).clamp(line, self.bar_max_height(ui));
            self.formula_bar_height = Some(height);
        }
        if response.drag_stopped() && self.formula_bar_height.is_some_and(|h| h < line * 1.5) {
            // Dragged back to about one line: collapsed.
            self.formula_bar_height = None;
        }
        if response.double_clicked() {
            self.toggle_formula_bar(ui);
        }
    }

    /// The button right of the formula bar that picks a place for the formula
    /// being typed; pressed while picking, when a click stops it.
    fn pick_button(&mut self, ui: &mut egui::Ui) {
        let picking = self.picking();
        let enabled = picking || self.editing.as_ref().is_some_and(|e| e.sheet == self.sheet);
        let text = if crate::icons::ready(ui.ctx()) {
            RichText::new(crate::icons::CROSSHAIR)
                .family(egui::FontFamily::Name(crate::style::ICONS.into()))
        } else {
            RichText::new("@")
        };
        let name = tr!("Pick a place");
        let response = ui
            .add_enabled(enabled, egui::Button::new(text).selected(picking))
            .on_hover_text(tr!(
                "Pick a place: click a node, on this tab or another, to put its address \
                 into the formula where the cursor is"
            ));
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, name));
        if response.clicked() {
            if picking {
                self.cancel_pick();
            } else {
                self.begin_pick();
            }
        }
    }

    /// The text of the selected cell: its formula or its value. Clicking in it
    /// starts editing; Enter enters, Esc cancels.
    fn bar_text(&mut self, ui: &mut egui::Ui) {
        let id = egui::Id::unique("formula_bar_text");
        if let Some(pick) = &self.pick {
            // The text set aside while a place is picked.
            let mut shown = pick.editing.text.clone();
            let height = self.formula_bar_height;
            let response = Self::bar_box(ui, height, |ui, rows| {
                ui.add(
                    Self::bar_edit(&mut shown, rows)
                        .id(id)
                        .code_editor()
                        .interactive(false),
                )
            });
            named(&response, tr!("Formula bar"));
            return;
        }
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
            named(&response, tr!("Formula bar"));
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
            // A node of a simple type whose formula's result it refused: that
            // result in red at the right end of the box, cut short if long.
            let refused = match &column {
                Some(_) => None,
                None => self.refused_galley(ui, &path),
            };
            let height = self.formula_bar_height;
            let response = Self::bar_box(ui, height, |ui, rows| {
                let mut edit = Self::bar_edit(&mut shown, rows)
                    .id(id)
                    .interactive(interactive);
                if let Some(galley) = &refused {
                    // A margin is at most 127 points; a wider value covers
                    // the end of a long formula instead.
                    let right = (galley.size().x + 12.0).ceil().min(127.0) as i8;
                    edit = edit.margin(egui::Margin {
                        left: 4,
                        right,
                        top: 2,
                        bottom: 2,
                    });
                }
                ui.add(edit)
            });
            if let Some(galley) = refused {
                let rect = response.rect;
                let at = egui::pos2(
                    rect.right() - 6.0 - galley.size().x,
                    rect.top()
                        + ((Self::bar_line_height(ui) + 4.0 - galley.size().y) / 2.0).max(0.0),
                );
                let behind =
                    egui::Rect::from_min_size(at, galley.size()).expand2(egui::vec2(3.0, 0.0));
                ui.painter()
                    .rect_filled(behind, 0.0, ui.visuals().text_edit_bg_color());
                ui.painter().galley(at, galley, error_color(ui));
            }
            named(&response, tr!("Formula bar"));
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
        let lines = self.user.prefs.formula_bar_lines as f32;
        let line_height = ui.text_style_height(&egui::TextStyle::Monospace);
        let margin = 2.0 * ui.spacing().button_padding.y + 4.0;
        let output = match self.formula_bar_height {
            // Expanded or dragged: that tall, scrolling inside.
            Some(height) => Self::bar_box(ui, Some(height), |ui, rows| {
                Self::bar_edit(&mut editing.text, rows)
                    .id(id)
                    .code_editor()
                    .show(ui)
            }),
            None => {
                egui::ScrollArea::vertical()
                    .id_salt("formula_bar_scroll")
                    .max_height(lines * line_height + margin)
                    // Only as tall as the text, up to the most lines.
                    .min_scrolled_height(0.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        egui::TextEdit::multiline(&mut editing.text)
                            .id(id)
                            .desired_rows(1)
                            .desired_width(f32::INFINITY)
                            .code_editor()
                            .show(ui)
                    })
                    .inner
            }
        };
        named(&output.response, tr!("Formula bar"));
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
            let display = self.user.prefs.display;
            let row_height = crate::style::grid_row_height(ui, display.density);
            let mut titles = vec![tr!("Name")];
            let mut table = TableBuilder::new(ui)
                // The widths are kept for each set of columns, so that hiding
                // one does not give its width to another.
                .id_salt(("grid", display.type_column, display.formula_column))
                .striped(true)
                .resizable(true)
                // A click anywhere in a row selects it, not only on its name.
                .sense(Sense::click())
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .max_scroll_height(f32::INFINITY)
                .column(Column::initial(300.0).at_least(120.0).clip(true));
            if display.type_column {
                titles.push(tr!("Type"));
                table = table.column(Column::initial(130.0).at_least(60.0).clip(true));
            }
            titles.push(tr!("Value"));
            table = table.column(if display.formula_column {
                Column::initial(260.0).at_least(80.0).clip(true)
            } else {
                Column::remainder().at_least(80.0).clip(true)
            });
            if display.formula_column {
                titles.push(tr!("Formula"));
                table = table.column(Column::remainder().at_least(80.0).clip(true));
            }
            if let Some(index) = scroll_to {
                table = table.scroll_to_row(index, Some(egui::Align::Center));
            }
            table
                .header(HEADER_HEIGHT, |mut header| {
                    for title in titles {
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
                        // The rest of the row, around what its cells show.
                        if table_row.response().clicked() {
                            actions.push(Action::Select(row.path.clone()));
                        }
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
        if self.picking() {
            // A click on a place picks it; opening and closing nodes still
            // works, to get to it; nothing else does.
            match action {
                Action::Select(path)
                | Action::Extend(path)
                | Action::BeginEdit(path)
                | Action::EnterText(path, _) => {
                    self.finish_pick(&path);
                    return;
                }
                Action::Toggle(_) | Action::Reveal(_) | Action::ToggleTabular(..) => {}
                _ => return,
            }
        }
        if matches!(action, Action::BeginEdit(_)) && self.just_picked() {
            return;
        }
        match action {
            Action::Select(path) => {
                self.select(&path);
            }
            Action::Extend(path) => {
                self.extend_selection(&path);
            }
            Action::Copy(mode) => {
                self.copy_cells(mode);
            }
            Action::Paste(mode) => {
                let _ = self.paste_from_clipboard(mode);
            }
            Action::RecalculateFormula(path) => self.recalculate_formula(&path),
            Action::ConvertToValues(path) => self.convert_formula_to_values(&path),
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
            Action::AddElement(list, at, element) => self.insert_element(&list, at, &element),
            Action::AddEntry(path) => self.begin_add_entry(&path),
            Action::RenameKey(path) => self.begin_rename_key(&path),
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
            Action::HideNode(path) => self.hide_node(&path),
            Action::ShowNode(path) => self.show_node(&path),
            Action::ShowAllNodes(parent) => self.show_all_nodes(&parent),
            Action::ShowRefusedRows => self.show_refused_rows(),
            Action::DismissRefusal => self.tables.refused = None,
            Action::EditComment(path, at) => {
                self.select(&path);
                self.begin_comment(&path, at);
            }
            Action::DeleteComment(path) => self.delete_comment(&path),
            Action::ForceType(path) => self.force_type(&path),
            Action::GoToDefinition(path) => self.go_to_definition(&path),
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
        let display = self.user.prefs.display;

        // The name.
        let mut name_response = None;
        table_row.col(|ui| {
            ui.add_space(row.depth as f32 * display.indent as f32);
            if row.expandable() {
                let (rect, response) = ui.allocate_exact_size(
                    egui::vec2(
                        14.0,
                        crate::style::grid_row_height(ui, display.density) - 4.0,
                    ),
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
            if row.hidden > 0 {
                let count = RichText::new(format!("({} hidden)", row.hidden))
                    .weak()
                    .small();
                ui.menu_button(count, |ui| self.hidden_list(ui, &row.path, actions));
            }
            if let Some((parent, count)) = &row.marker {
                self.hidden_marker(ui, row, parent, *count, actions);
            }
        });

        // The type, as a badge in the colour of its kind.
        let mut menu_on = Vec::new();
        if display.type_column {
            menu_on.push(self.type_cell(table_row, row, actions));
        }

        // The value.
        table_row.col(|ui| self.value_column(ui, row, selected, editing, actions, true));

        // The formula.
        if display.formula_column {
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
                    let response = match formula_error {
                        Some(error) => response.on_hover_text(format!(
                            "{} {}",
                            error.kind.code(),
                            error.message
                        )),
                        None => response,
                    };
                    menu_on.push(response);
                } else if let Some(refused) = computed_by
                    .as_ref()
                    .and_then(|_| self.refused_text(&row.path))
                {
                    // A part of another formula's result that did not fit:
                    // what it was, in red, in place of "computed by".
                    let color = error_color(ui);
                    let by = computed_by.as_ref().map(|by| self.name_of(by));
                    ui.add(egui::Label::new(RichText::new(refused).color(color)).truncate())
                        .on_hover_text(tr!("computed by {name}", name = by.unwrap_or_default()));
                } else if let Some(by) = &computed_by {
                    ui.label(
                        RichText::new(tr!("computed by {name}", name = self.name_of(by)))
                            .weak()
                            .italics(),
                    );
                }
            });
        }

        // A right click anywhere on the row opens its menu, but on the value,
        // whose menu has the cell's commands as well.
        let response = menu_on
            .into_iter()
            .chain(name_response)
            .fold(table_row.response(), |all, r| all.union(r));
        response.context_menu(|ui| self.row_menu(ui, row, actions));
    }

    /// The Type column's cell: a badge in the colour of the kind.
    fn type_cell(
        &self,
        table_row: &mut egui_extras::TableRow<'_, '_>,
        row: &Row,
        actions: &mut Vec<Action>,
    ) -> egui::Response {
        let calc = self.calc();
        let mut badge = None;
        let (_, cell) = table_row.col(|ui| {
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
            badge = Some(response);
        });
        badge.unwrap_or(cell)
    }

    /// The value of a node, with the marks a cell can have: the tint of a
    /// computed value, the outline of invalid data, the editor while it is
    /// being typed into, and the marker of a calculation that is waiting.
    /// With `row_items`, the cell's menu also has the commands of its row,
    /// as the tree's rows do.
    pub(crate) fn value_column(
        &self,
        ui: &mut egui::Ui,
        row: &Row,
        selected: bool,
        editing: &mut Option<Editing>,
        actions: &mut Vec<Action>,
        row_items: bool,
    ) {
        let calc = self.calc();
        let cell = calc.cell(&self.sheet, &row.path);
        let formula = cell.as_ref().and_then(|c| c.formula);
        let formula_error = cell.as_ref().and_then(|c| c.error);
        let pending = cell.as_ref().is_some_and(|c| c.pending);
        let untrusted = cell.as_ref().is_some_and(|c| c.untrusted);
        let stale = cell.as_ref().is_some_and(|c| c.stale);
        let computed_by = cell.as_ref().and_then(|c| c.computed_by.clone());
        // On the way to a part of a result that did not fit: the formula's
        // error, in place of what the node holds of the parts that did.
        let misfit = cell
            .as_ref()
            .and_then(|c| c.misfit)
            .filter(|_| {
                row.node
                    .as_ref()
                    .is_some_and(|n| !matches!(n.value(), Value::Error(_)))
            })
            .map(|e| Value::Error(e.clone()));
        let value = misfit.as_ref().or(row.node.as_ref().map(|n| n.value()));
        let is_error = matches!(value, Some(Value::Error(_)));
        let editable = !row.unrecognised && computed_by.is_none();

        // What is wrong with the data, for the outline.
        let invalid = match (&row.node, row.ty, calc.spec(&self.sheet)) {
            (Some(node), Some(ty), Some(spec)) if row.kind == RowKind::Scalar && !is_error => {
                !gossia_core::validate::validate_at(spec, ty, node, &row.path).is_empty()
            }
            _ => false,
        };
        let missing = row.node.is_none() && row.required && !row.unrecognised;
        let cell = ui.max_rect();
        self.comment_marks(ui, cell, &row.path);
        // Behind what the cell shows, so that its widgets keep their clicks:
        // a right click on the rest of the cell opens its comment menu, on a
        // tab whose format has comments.
        if calc.format_has_comments(&self.sheet) && row.node.is_some() {
            let background = ui.interact(
                cell,
                ui.scope_id().with(("cell_menu", row.path.to_string())),
                Sense::click(),
            );
            if background.clicked() {
                actions.push(Action::Select(row.path.clone()));
            }
            background.context_menu(|ui| {
                if row_items {
                    self.row_menu(ui, row, actions);
                } else {
                    self.comment_menu_items(ui, &row.path, cell.right_top(), actions, true);
                }
            });
        }

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
                row_items,
            );
        });
        if untrusted && formula.is_some() {
            untrusted_mark(ui, stale);
        }
        if pending {
            crate::style::pending_marker(ui).on_hover_text(tr!("Waiting to be calculated"));
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn value_cell(
        &self,
        ui: &mut egui::Ui,
        row: &Row,
        value: Option<&Value>,
        formula: Option<&str>,
        formula_error: Option<&gossia_core::ErrorValue>,
        editable: bool,
        selected: bool,
        actions: &mut Vec<Action>,
        row_items: bool,
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
                    actions.push(select_action(ui, &row.path));
                }
                if response.double_clicked() && editable && row.kind == RowKind::Scalar {
                    actions.push(Action::BeginEdit(row.path.clone()));
                }
                if response.double_clicked() && row.kind != RowKind::Scalar && !row.tabular {
                    actions.push(Action::Reveal(row.path.clone()));
                }
                let at = ui.max_rect().right_top();
                let response =
                    self.cell_menu(response, &row.path, at, row_items.then_some(row), actions);
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
                    actions.push(select_action(ui, &row.path));
                }
                if response.double_clicked() && editable && row.kind == RowKind::Scalar {
                    actions.push(Action::BeginEdit(row.path.clone()));
                }
                let at = ui.max_rect().right_top();
                self.cell_menu(response, &row.path, at, row_items.then_some(row), actions);
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
        named(&output.response, tr!("Cell editor"));
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

    /// The hidden children of `parent`, each with *Show*, and *Show all*.
    fn hidden_list(&self, ui: &mut egui::Ui, parent: &Path, actions: &mut Vec<Action>) {
        let children = self.hidden_children(parent);
        for child in &children {
            ui.horizontal(|ui| {
                ui.label(&child.name);
                let label = tr!("Show {name}", name = child.name);
                let show = ui.small_button(tr!("Show"));
                show.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
                if show.clicked() {
                    actions.push(Action::ShowNode(child.path.clone()));
                    ui.close();
                }
            });
        }
        if !children.is_empty() && ui.button(tr!("Show all")).clicked() {
            actions.push(Action::ShowAllNodes(parent.clone()));
            ui.close();
        }
    }

    /// The thin line under a row where hidden nodes were; a click on it opens
    /// the list that shows them.
    fn hidden_marker(
        &self,
        ui: &mut egui::Ui,
        row: &Row,
        parent: &Path,
        count: usize,
        actions: &mut Vec<Action>,
    ) {
        let cell = ui.max_rect();
        let indent = (parent.len() + 1) as f32 * self.user.prefs.display.indent as f32;
        let left = cell.left() + indent + 14.0;
        let line = egui::Rect::from_min_max(
            egui::pos2(left, cell.bottom() - 2.0),
            egui::pos2((left + 60.0).min(cell.right()), cell.bottom()),
        );
        let id = ui.scope_id().with(("hidden_marker", row.path.to_string()));
        let response = ui
            .interact(line.expand2(egui::vec2(0.0, 2.0)), id, Sense::click())
            .on_hover_text(format!("{count} hidden"));
        let colour = if response.hovered() {
            ui.visuals().strong_text_color()
        } else {
            ui.visuals().weak_text_color()
        };
        ui.painter().rect_filled(line, 1.0, colour);
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::Button, true, format!("{count} hidden"))
        });
        egui::Popup::menu(&response).show(|ui| self.hidden_list(ui, parent, actions));
    }

    fn row_menu(&self, ui: &mut egui::Ui, row: &Row, actions: &mut Vec<Action>) {
        let path = &row.path;
        if row.expandable() {
            let label = if row.expanded {
                tr!("Collapse")
            } else {
                tr!("Expand")
            };
            if ui.button(label).clicked() {
                actions.push(Action::Toggle(path.clone()));
                ui.close();
            }
        }
        // The element of an item of an ordered list stands for the item.
        let item = row.item.as_ref().unwrap_or(path);
        if !path.is_empty() && ui.button(tr!("Hide")).clicked() {
            actions.push(Action::HideNode(item.clone()));
            ui.close();
        }
        if row.hidden > 0 {
            ui.menu_button(tr!("Hidden ({hidden})", hidden = row.hidden), |ui| {
                self.hidden_list(ui, path, actions);
            });
        }
        let elements = (row.kind == RowKind::Array)
            .then(|| self.list_elements(path))
            .flatten();
        match elements {
            // An ordered list asks which element.
            Some(elements) => {
                ui.menu_button(tr!("Add item"), |ui| {
                    for element in elements {
                        if ui.button(&element).clicked() {
                            actions.push(Action::AddElement(path.clone(), None, element));
                            ui.close();
                        }
                    }
                });
            }
            None if row.kind == RowKind::Array && ui.button(tr!("Add item")).clicked() => {
                actions.push(Action::AddItem(path.clone()));
                ui.close();
            }
            None => {}
        }
        if row.kind == RowKind::Map && ui.button(tr!("Add entry...")).clicked() {
            actions.push(Action::AddEntry(path.clone()));
            ui.close();
        }
        let map_entry = row.node.is_some()
            && matches!(path.segments().last(), Some(Segment::Key(_)))
            && path.parent().is_some_and(|map| self.is_map(&map));
        if map_entry && ui.button(tr!("Rename key...")).clicked() {
            actions.push(Action::RenameKey(path.clone()));
            ui.close();
        }
        if row.kind == RowKind::Array {
            if self.can_tabulate(path) {
                let label = if row.tabular {
                    tr!("Tabular view (on)")
                } else {
                    tr!("Tabular view")
                };
                if ui.button(label).clicked() {
                    actions.push(Action::ToggleTabular(path.clone(), !row.tabular));
                    ui.close();
                }
            }
            let reorderable = self.can_reorder(path);
            let reverse = ui.add_enabled(reorderable, egui::Button::new(tr!("Reverse row order")));
            if reverse.clicked() {
                actions.push(Action::Reverse(path.clone()));
                ui.close();
            }
        }
        match item.segments().last() {
            Some(Segment::Index(i)) => {
                let list = item.parent().unwrap_or_default();
                match self.list_elements(&list) {
                    Some(elements) => {
                        ui.menu_button(tr!("Insert item above"), |ui| {
                            for element in elements {
                                if ui.button(&element).clicked() {
                                    actions.push(Action::AddElement(
                                        list.clone(),
                                        Some(*i),
                                        element,
                                    ));
                                    ui.close();
                                }
                            }
                        });
                    }
                    None => {
                        if ui.button(tr!("Insert item above")).clicked() {
                            actions.push(Action::InsertAbove(item.clone()));
                            ui.close();
                        }
                    }
                }
                if ui.button(tr!("Delete item")).clicked() {
                    actions.push(Action::Delete(item.clone()));
                    ui.close();
                }
            }
            Some(Segment::Key(_)) if row.node.is_some() && (!row.required || row.unrecognised) => {
                let label = if row.unrecognised {
                    tr!("Delete")
                } else {
                    tr!("Delete (leave absent)")
                };
                if ui.button(label).clicked() {
                    actions.push(Action::Delete(path.clone()));
                    ui.close();
                }
            }
            _ => {}
        }
        // An array has no fields of its own: its items do, and their rows
        // offer Add field. It is itself a field of its record, though, so it
        // is renamed and deleted like any other.
        let add_to = match row.kind {
            RowKind::Object => self.object_of_row(path),
            _ => None,
        };
        let field = self.field_of(path);
        self.field_menu_items(ui, add_to.as_ref(), field.as_ref(), actions);
        self.force_type_item(ui, path, actions);
        self.definition_item(ui, path, actions);
        if row.node.is_some() {
            let at = ui.ctx().pointer_interact_pos().unwrap_or_default();
            self.comment_menu_items(ui, path, at, actions, true);
        }
    }

    // ------------------------------------------------- tabs and status

    fn tab_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let names = self.calc().tab_names();
            ui.spacing_mut().item_spacing.x = 2.0;
            // A tab dragged along the bar goes where it is let go: before the
            // first tab whose middle is right of the pointer.
            let mut rects = Vec::with_capacity(names.len());
            let mut dragged = None;
            for (i, name) in names.iter().enumerate() {
                let selected = *name == self.sheet;
                let report = self.calc().is_report(name);
                let response = sheet_tab(ui, name, selected, report);
                if response.clicked() {
                    self.switch_to(name);
                }
                if response.double_clicked() {
                    self.begin_rename_tab(name);
                }
                if response.dragged() || response.drag_stopped() {
                    dragged = Some((i, response.drag_stopped()));
                }
                rects.push(response.rect);
                response.context_menu(|ui| self.tab_menu_items(ui, name));
            }
            if let (Some((from, dropped)), Some(x)) =
                (dragged, ui.ctx().pointer_latest_pos().map(|p| p.x))
            {
                let place = rects
                    .iter()
                    .position(|r| r.center().x > x)
                    .unwrap_or(rects.len());
                if place != from && place != from + 1 {
                    // The mark where it would go.
                    let edge = rects
                        .get(place)
                        .map_or_else(|| rects[rects.len() - 1].right(), |r| r.left() - 1.0);
                    let rect = rects[from];
                    ui.painter().vline(
                        edge,
                        rect.y_range(),
                        egui::Stroke::new(2.0, Palette::of_ui(ui).accent),
                    );
                    if dropped {
                        // Taking it out first shifts the places after it.
                        let to = if place > from { place - 1 } else { place };
                        self.move_tab(&names[from], to);
                    }
                }
            }
            let add = ui
                .add(egui::Button::new(crate::icons::PLUS).frame(false))
                .on_hover_text(tr!("New tab"));
            named(&add, tr!("New tab"));
            if add.clicked() {
                self.begin_new_tab();
            }
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let status = self.session.status();
            if status.suspended {
                // The Evaluation page has the limit and *Save and restart*.
                if ui
                    .link(evaluation_text(&status))
                    .on_hover_text(tr!("Options: Evaluation"))
                    .clicked()
                {
                    self.begin_options(Some(crate::options::Page::Evaluation));
                }
            } else {
                ui.label(evaluation_text(&status));
            }
            ui.separator();
            ui.label(problems_text(self.summary.errors, self.summary.invalid));
            if !self.calc().is_trusted() {
                ui.separator();
                let stale = self.calc().stale();
                let text = if stale == 0 {
                    tr!("Not trusted").to_owned()
                } else {
                    tr!("Not trusted, {stale} stale", stale = stale)
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
                ui.label(tr!("Modified"));
            }
        });
    }

    fn issues_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading(tr!("Issues"));
        let issues = self.issue_list();
        let modules = self.calc().module_issues();
        if issues.is_empty() && modules.is_empty() {
            ui.label(tr!("No errors, no invalid data."));
            return;
        }
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (module, why) in modules {
                ui.colored_label(
                    error_color(ui),
                    tr!("Module {module}: {why}", module = module, why = why),
                );
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
        egui::Window::new(tr!("Insert Function"))
            .open(&mut open)
            .default_size([520.0, 380.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(tr!("Search"));
                    let response = ui.text_edit_singleline(&mut dialog.search);
                    named(&response, tr!("Search functions"));
                });
                ui.separator();
                let found = GossiaApp::functions_matching(&dialog.search);
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
                            ui.label(gossia_i18n::tr(f.description));
                            if ui.button(tr!("Insert")).clicked() {
                                insert = Some(f.insert);
                            }
                        }
                        None => {
                            ui.label(tr!("Choose a function to see what it does."));
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

/// The mark of a formula that was not run because the workbook is not
/// trusted, in the grid and on a form.
pub(crate) fn untrusted_mark(ui: &mut egui::Ui, stale: bool) -> egui::Response {
    ui.label(RichText::new("#UNTRUSTED!").color(error_color(ui)).small())
        .on_hover_text(if stale {
            tr!(
                "This workbook is not trusted, so the formula was not run. The value is \
             out of date: something it uses has changed."
            )
        } else {
            tr!(
                "This workbook is not trusted, so the formula was not run. The value is \
             the one that was stored."
            )
        })
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
/// A report's tab has a chart before its name.
fn sheet_tab(ui: &mut egui::Ui, name: &str, selected: bool, report: bool) -> egui::Response {
    let palette = Palette::of_ui(ui);
    let shown = if report {
        format!("{} {name}", crate::icons::CHART_BAR)
    } else {
        name.to_owned()
    };
    let text = if selected {
        crate::style::medium(shown).color(palette.text)
    } else {
        RichText::new(shown).color(palette.text_weak)
    };
    let galley = egui::WidgetText::from(text).into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        egui::TextStyle::Button,
    );
    let size = egui::vec2(galley.size().x + 20.0, 30.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
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

/// A click selects; with Shift it makes a range in a table.
fn select_action(ui: &egui::Ui, path: &Path) -> Action {
    if ui.input(|i| i.modifiers.shift) {
        Action::Extend(path.clone())
    } else {
        Action::Select(path.clone())
    }
}

impl GossiaApp {
    /// The context menu of a cell's value: copy and paste, and on XML tabs
    /// the comment commands. A right click outside the selection selects the
    /// cell first, so that the commands act on it.
    fn cell_menu(
        &self,
        response: egui::Response,
        path: &Path,
        at: egui::Pos2,
        row: Option<&Row>,
        actions: &mut Vec<Action>,
    ) -> egui::Response {
        if response.secondary_clicked() && !self.is_in_selection(path) {
            actions.push(Action::Select(path.clone()));
        }
        let comments = self.calc().format_has_comments(&self.sheet);
        let formula = self.calc().formula_text(&self.sheet, path).is_some();
        response.context_menu(|ui| {
            let mut item = |ui: &mut egui::Ui, label: &str, keys: &str, action: Action| {
                if ui
                    .add(egui::Button::new(label).shortcut_text(keys))
                    .clicked()
                {
                    actions.push(action);
                    ui.close();
                }
            };
            item(ui, tr!("Copy"), "Ctrl+C", Action::Copy(CopyMode::Cells));
            item(
                ui,
                tr!("Copy formulas"),
                "Ctrl+Shift+C",
                Action::Copy(CopyMode::Formulas),
            );
            item(ui, tr!("Paste"), "Ctrl+V", Action::Paste(PasteMode::Cells));
            item(
                ui,
                tr!("Paste values"),
                "Ctrl+Shift+V",
                Action::Paste(PasteMode::Values),
            );
            if formula {
                ui.separator();
                item(
                    ui,
                    tr!("Recalculate this formula"),
                    "",
                    Action::RecalculateFormula(path.clone()),
                );
                item(
                    ui,
                    tr!("Convert formula to values"),
                    "",
                    Action::ConvertToValues(path.clone()),
                );
            }
            match row {
                // The row's commands, which take in its comments.
                Some(row) => {
                    ui.separator();
                    self.row_menu(ui, row, actions);
                }
                None if comments => {
                    ui.separator();
                    self.comment_menu_items(ui, path, at, actions, true);
                }
                None => {}
            }
        });
        response
    }
}
