//! Drawing the tabular view: the pane under the tree with the table of an
//! array of objects, its toolbar, the column and row menus, and the filter
//! dialog. What the commands do is in `tables.rs`.

use cliotree_core::{Kind, Node, Path, TypeSpec};
use egui::{RichText, Sense, Stroke, StrokeKind};
use egui_extras::{Column as TableColumn, TableBuilder};

use crate::app::{ClioApp, Editing};
use crate::rows::{Row, RowKind};
use crate::schema_view::ColumnDrag;
use crate::table::Column;
use crate::tables::{Choice, Layout};
use crate::view::{Action, error_color, named};

impl ClioApp {
    /// The pane: a toolbar, and the table.
    pub(crate) fn table_pane(&mut self, ui: &mut egui::Ui) {
        let Some((key, layout)) = self.tables.layout.take() else {
            return;
        };
        let mut editing = self.editing.take();
        let mut actions: Vec<Action> = Vec::new();
        self.table_toolbar(ui, &layout, &mut actions);
        self.table_body(ui, &layout, &mut editing, &mut actions);
        self.tables.layout = Some((key, layout));
        self.editing = editing;
        for action in actions {
            self.apply(action);
        }
    }

    fn table_toolbar(&self, ui: &mut egui::Ui, layout: &Layout, actions: &mut Vec<Action>) {
        let array = &layout.array;
        let visible = &layout.visible;
        let view = self.view_of(array);
        let filtered = view.is_some_and(|v| !v.filters.is_empty());
        let hidden_columns: Vec<String> = view
            .map(|v| v.hidden_columns.iter().cloned().collect())
            .unwrap_or_default();
        let hidden_rows = self.hidden_rows(array);
        ui.horizontal_wrapped(|ui| {
            ui.strong(self.name_of(array));
            let mut counts = if visible.rows.len() == visible.total {
                format!("{} rows", visible.total)
            } else {
                format!("{} of {} rows", visible.rows.len(), visible.total)
            };
            if visible.hidden > 0 || visible.filtered > 0 {
                counts += &format!(
                    " ({} hidden, {} filtered)",
                    visible.hidden, visible.filtered
                );
            }
            ui.label(counts);
            ui.separator();
            if ui
                .add_enabled(self.can_edit(array), egui::Button::new("Add row"))
                .clicked()
            {
                actions.push(Action::AddRow(array.clone(), None));
            }
            if ui
                .add_enabled(
                    self.can_reorder(array),
                    egui::Button::new("Reverse row order"),
                )
                .clicked()
            {
                actions.push(Action::Reverse(array.clone()));
            }
            if ui
                .add_enabled(filtered, egui::Button::new("Clear all filters"))
                .clicked()
            {
                actions.push(Action::ClearAllFilters(array.clone()));
            }
            if ui
                .add_enabled(filtered, egui::Button::new("Reapply filter"))
                .clicked()
            {
                actions.push(Action::ReapplyFilters(array.clone()));
            }
            let hidden = hidden_columns.len() + hidden_rows.len();
            if hidden > 0 {
                ui.menu_button(format!("Hidden ({hidden})"), |ui| {
                    for column in &hidden_columns {
                        if ui.button(format!("Show column {column}")).clicked() {
                            actions.push(Action::ShowColumn(array.clone(), column.clone()));
                            ui.close();
                        }
                    }
                    for (index, id) in &hidden_rows {
                        if ui.button(format!("Show row [{index}]")).clicked() {
                            actions.push(Action::ShowRow(array.clone(), *id));
                            ui.close();
                        }
                    }
                    ui.separator();
                    if ui.button("Show all").clicked() {
                        actions.push(Action::ShowAllHidden(array.clone()));
                        ui.close();
                    }
                });
            }
            if ui.button("Close table").clicked() {
                actions.push(Action::ToggleTabular(array.clone(), false));
            }
        });
        if let Some(refused) = self.tables.refused.as_ref().filter(|r| &r.array == array) {
            ui.horizontal_wrapped(|ui| {
                let rows: Vec<String> = refused
                    .rows
                    .iter()
                    .take(8)
                    .map(|i| format!("[{i}]"))
                    .collect();
                let more = refused.rows.len().saturating_sub(8);
                let mut text = format!(
                    "Not sorted by {}: {} (rows {}",
                    refused.column,
                    refused.message,
                    rows.join(", ")
                );
                if more > 0 {
                    text += &format!(" and {more} more");
                }
                text.push(')');
                ui.colored_label(error_color(ui), text);
                if ui.button("Show these rows").clicked() {
                    actions.push(Action::ShowRefusedRows);
                }
                if ui.button("Dismiss").clicked() {
                    actions.push(Action::DismissRefusal);
                }
            });
        }
    }

    fn table_body(
        &self,
        ui: &mut egui::Ui,
        layout: &Layout,
        editing: &mut Option<Editing>,
        actions: &mut Vec<Action>,
    ) {
        let array = &layout.array;
        let calc = self.calc();
        let (Some(spec), Some(items)) = (
            calc.spec(&self.sheet),
            calc.data(&self.sheet)
                .and_then(|d| d.get(array))
                .and_then(|n| n.value().as_array()),
        ) else {
            ui.label("The array is not there.");
            return;
        };
        let shown: Vec<&Column> = layout.shown.iter().map(|&i| &layout.columns[i]).collect();
        let range = self.table_range();
        let column_place = |column: &Column| {
            layout
                .columns
                .iter()
                .position(|c| c.name == column.name)
                .unwrap_or(usize::MAX)
        };
        let rows = &layout.visible.rows;
        let editable = self.can_edit(array);
        let reorderable = self.can_reorder(array);
        egui::ScrollArea::horizontal()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let display = self.user.prefs.display;
                let row_height = crate::style::grid_row_height(ui, display.density);
                let width = display.column_width as f32;
                let mut table = TableBuilder::new(ui)
                    .id_salt(("table", array.to_string()))
                    .striped(true)
                    .resizable(true)
                    .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                    .max_scroll_height(f32::INFINITY)
                    .column(TableColumn::initial(64.0).at_least(40.0));
                for _ in &shown {
                    table = table.column(TableColumn::initial(width).at_least(40.0).clip(true));
                }
                table
                    .header(crate::style::HEADER_HEIGHT, |mut header| {
                        header.col(|ui| {
                            crate::view::column_title(ui, "#");
                        });
                        let mut previous: Option<usize> = None;
                        for &c in &layout.shown {
                            let gap = c > previous.map_or(0, |p| p + 1);
                            previous = Some(c);
                            header.col(|ui| {
                                self.column_header(ui, array, &layout.columns[c], gap, actions);
                            });
                        }
                    })
                    .body(|body| {
                        body.rows(row_height, rows.len(), |mut table_row| {
                            let position = table_row.index();
                            let index = rows[position];
                            let item = &items[index];
                            table_row.col(|ui| {
                                let gap = if position == 0 {
                                    index > 0
                                } else {
                                    index > rows[position - 1] + 1
                                };
                                let text = if gap {
                                    format!(".. [{index}]")
                                } else {
                                    format!("[{index}]")
                                };
                                let response = ui.add(
                                    egui::Label::new(RichText::new(text).weak())
                                        .sense(Sense::CLICK),
                                );
                                if response.clicked() {
                                    actions.push(Action::Select(array.index(index)));
                                }
                                response.context_menu(|ui| {
                                    row_menu(
                                        ui,
                                        array,
                                        index,
                                        (editable, reorderable),
                                        (position > 0, position + 1 < rows.len()),
                                        actions,
                                    );
                                });
                            });
                            for column in &shown {
                                table_row.col(|ui| {
                                    let cell = cell_row(spec, array, index, item, column);
                                    if self.column_is_selected(array, &column.name) {
                                        ui.painter().rect_filled(
                                            ui.max_rect(),
                                            0.0,
                                            ui.visuals().selection.bg_fill.gamma_multiply(0.35),
                                        );
                                    }
                                    if range.as_ref().is_some_and(|r| {
                                        r.is_range() && r.contains(index, column_place(column))
                                    }) {
                                        ui.painter().rect_filled(
                                            ui.max_rect(),
                                            0.0,
                                            ui.visuals().selection.bg_fill.gamma_multiply(0.25),
                                        );
                                    }
                                    let selected = self.selection.as_ref() == Some(&cell.path);
                                    if selected {
                                        ui.painter().rect_stroke(
                                            ui.max_rect().shrink(0.5),
                                            2.0,
                                            Stroke::new(2.0, ui.visuals().selection.stroke.color),
                                            StrokeKind::Inside,
                                        );
                                    }
                                    self.value_column(ui, &cell, selected, editing, actions, false);
                                });
                            }
                        });
                    });
            });
    }

    fn column_header(
        &self,
        ui: &mut egui::Ui,
        array: &Path,
        column: &Column,
        gap: bool,
        actions: &mut Vec<Action>,
    ) {
        let filtered = self.filter_of(array, &column.name).is_some();
        let reorderable = self.can_reorder(array);
        let sortable = reorderable && !column.summary;
        ui.horizontal(|ui| {
            let text = if gap {
                format!(".. {}", column.name)
            } else {
                column.name.clone()
            };
            let chosen = self.column_is_selected(array, &column.name);
            let colour = crate::style::Palette::of_ui(ui).text_weak;
            let response = ui.add(
                egui::Label::new(
                    crate::style::medium(text)
                        .size(crate::style::SMALL_SIZE)
                        .color(colour),
                )
                .selectable(false)
                .sense(Sense::click_and_drag())
                .truncate(),
            );
            if chosen {
                ui.painter().rect_stroke(
                    response.rect.expand2(egui::vec2(2.0, 1.0)),
                    2.0,
                    Stroke::new(1.5, ui.visuals().selection.stroke.color),
                    StrokeKind::Inside,
                );
            }
            let owner = self.column_owner(array, column);
            let movable = owner
                .as_ref()
                .is_some_and(|(object, _)| self.schema_edit_allowed(object).is_ok());
            if let (true, Some((object, field))) = (movable, &owner) {
                response.dnd_set_drag_payload(ColumnDrag {
                    object: object.clone(),
                    field: field.clone(),
                });
                let here = |p: &ColumnDrag| &p.object == object && &p.field != field;
                if let Some(dragged) = response.dnd_release_payload::<ColumnDrag>()
                    && here(&dragged)
                {
                    actions.push(Action::ReorderFields(
                        object.clone(),
                        dragged.field.clone(),
                        field.clone(),
                    ));
                }
                if response
                    .dnd_hover_payload::<ColumnDrag>()
                    .is_some_and(|p| here(&p))
                {
                    let rect = response.rect;
                    ui.painter().vline(
                        rect.left(),
                        rect.y_range(),
                        Stroke::new(2.0, ui.visuals().selection.stroke.color),
                    );
                }
            }
            if filtered {
                filter_mark(ui);
            }
            if response.clicked() {
                actions.push(Action::SelectColumn(array.clone(), column.name.clone()));
            }
            let drag_note = match (&owner, movable) {
                (_, true) => " Drag to move the field.".to_owned(),
                (Some((object, _)), false) => self
                    .schema_edit_allowed(object)
                    .err()
                    .map(|why| format!(" To drag the field: {why}."))
                    .unwrap_or_default(),
                _ => String::new(),
            };
            let response = response.on_hover_text(format!(
                "Click to select the column. Right-click for more, such as sorting.{drag_note}"
            ));
            response.context_menu(|ui| {
                if ui
                    .add_enabled(sortable, egui::Button::new("Sort ascending"))
                    .clicked()
                {
                    actions.push(Action::SortBy(
                        array.clone(),
                        column.name.clone(),
                        Some(false),
                    ));
                    ui.close();
                }
                if ui
                    .add_enabled(sortable, egui::Button::new("Sort descending"))
                    .clicked()
                {
                    actions.push(Action::SortBy(
                        array.clone(),
                        column.name.clone(),
                        Some(true),
                    ));
                    ui.close();
                }
                ui.separator();
                if ui
                    .add_enabled(!column.summary, egui::Button::new("Filter..."))
                    .clicked()
                {
                    actions.push(Action::FilterDialog(array.clone(), column.name.clone()));
                    ui.close();
                }
                if ui
                    .add_enabled(filtered, egui::Button::new("Clear filter"))
                    .clicked()
                {
                    actions.push(Action::ClearFilter(array.clone(), column.name.clone()));
                    ui.close();
                }
                ui.separator();
                if ui.button("Hide column").clicked() {
                    actions.push(Action::HideColumn(array.clone(), column.name.clone()));
                    ui.close();
                }
                self.field_menu_items(ui, Some(&array.index(0)), owner.as_ref(), actions);
            });
        });
    }

    /// Whether this column is the selected one.
    fn column_is_selected(&self, array: &Path, name: &str) -> bool {
        self.column_selected()
            .is_some_and(|c| &c.array == array && c.field == name)
    }

    // ------------------------------------------------------ the filter

    /// The filter dialog of a column, when one is open.
    pub(crate) fn filter_window(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.tables.dialog.take() else {
            return;
        };
        let mut open = true;
        let (mut apply, mut cancel, mut clear) = (false, false, false);
        egui::Window::new(format!("Filter {}", dialog.column))
            .open(&mut open)
            .default_size([360.0, 420.0])
            .show(ctx, |ui| {
                ui.label("Show these values");
                ui.horizontal(|ui| {
                    let response = ui.text_edit_singleline(&mut dialog.search);
                    named(&response, "Search values");
                    if ui.button("Select all").clicked() {
                        dialog.checked = dialog.distinct.iter().cloned().collect();
                    }
                    if ui.button("Select none").clicked() {
                        dialog.checked.clear();
                    }
                });
                let needle = dialog.search.to_lowercase();
                egui::ScrollArea::vertical()
                    .max_height(180.0)
                    .id_salt("filter_values")
                    .show(ui, |ui| {
                        let listed: Vec<String> = dialog
                            .distinct
                            .iter()
                            .filter(|v| needle.is_empty() || v.to_lowercase().contains(&needle))
                            .take(500)
                            .cloned()
                            .collect();
                        for value in listed {
                            let mut on = dialog.checked.contains(&value);
                            if ui.checkbox(&mut on, &value).changed() {
                                if on {
                                    dialog.checked.insert(value);
                                } else {
                                    dialog.checked.remove(&value);
                                }
                            }
                        }
                    });
                ui.separator();
                egui::ComboBox::from_label("Condition")
                    .selected_text(dialog.choice.label())
                    .show_ui(ui, |ui| {
                        for choice in Choice::ALL {
                            ui.selectable_value(&mut dialog.choice, choice, choice.label());
                        }
                    });
                if dialog.choice.operands() >= 1 {
                    let response = ui.text_edit_singleline(&mut dialog.first);
                    named(&response, "Filter value");
                }
                if dialog.choice.operands() == 2 {
                    let response = ui.text_edit_singleline(&mut dialog.second);
                    named(&response, "Filter upper value");
                }
                ui.checkbox(&mut dialog.ignore_case, "Ignore case");
                if let Some(error) = &dialog.error {
                    ui.colored_label(error_color(ui), error);
                }
                ui.horizontal(|ui| {
                    apply = ui.button("Apply").clicked();
                    cancel = ui.button("Cancel").clicked();
                    clear = ui.button("Clear filter").clicked();
                });
            });
        if apply {
            self.tables.dialog = Some(dialog);
            self.apply_filter_dialog();
        } else if clear {
            self.set_filter(&dialog.array, &dialog.column, None);
        } else if open && !cancel {
            self.tables.dialog = Some(dialog);
        }
    }
}

/// The mark of a column that is filtered, after its title: a funnel's
/// triangle.
pub(crate) fn filter_mark(ui: &mut egui::Ui) {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), Sense::hover());
    let c = rect.center();
    ui.painter().add(egui::Shape::convex_polygon(
        vec![
            egui::pos2(c.x - 5.0, c.y - 4.0),
            egui::pos2(c.x + 5.0, c.y - 4.0),
            egui::pos2(c.x, c.y + 5.0),
        ],
        ui.visuals().strong_text_color(),
        Stroke::NONE,
    ));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, "Filtered"));
    response.on_hover_text("Filtered");
}

/// The context menu of a row header.
fn row_menu(
    ui: &mut egui::Ui,
    array: &Path,
    index: usize,
    (editable, reorderable): (bool, bool),
    (has_previous, has_next): (bool, bool),
    actions: &mut Vec<Action>,
) {
    let mut item = |ui: &mut egui::Ui, enabled: bool, label: &str, action: Action| {
        if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
            actions.push(action);
            ui.close();
        }
    };
    item(
        ui,
        editable,
        "Insert row above",
        Action::AddRow(array.clone(), Some(index)),
    );
    item(
        ui,
        editable,
        "Insert row below",
        Action::AddRow(array.clone(), Some(index + 1)),
    );
    item(
        ui,
        editable,
        "Delete row",
        Action::DeleteRow(array.clone(), index),
    );
    ui.separator();
    item(
        ui,
        reorderable && has_previous,
        "Move row up",
        Action::MoveRow(array.clone(), index, true),
    );
    item(
        ui,
        reorderable && has_next,
        "Move row down",
        Action::MoveRow(array.clone(), index, false),
    );
    ui.separator();
    item(ui, true, "Hide row", Action::HideRow(array.clone(), index));
}

/// A cell of the table as a row of the tree grid, so that it is drawn and
/// edited by the same code.
fn cell_row(spec: &TypeSpec, array: &Path, index: usize, item: &Node, column: &Column) -> Row {
    let kind = match &spec.get(column.ty).kind {
        Kind::Array { .. } => RowKind::Array,
        Kind::Map { .. } => RowKind::Map,
        Kind::Object { .. } => RowKind::Object,
        _ => RowKind::Scalar,
    };
    Row {
        path: column.cell_path(array, index),
        depth: 0,
        name: column.name.clone(),
        ty: Some(column.ty),
        type_label: String::new(),
        node: column.cell(item).cloned(),
        kind,
        required: column.required,
        expanded: false,
        unrecognised: false,
        tabular: false,
        hidden: 0,
        marker: None,
    }
}
