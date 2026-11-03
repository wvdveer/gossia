//! Named forms and customising them (DESIGN.md, "Forms"): the commands that
//! make, copy, rename and delete forms, layout mode where the form being shown
//! is edited on the form itself, and the property page of the selected control.
//!
//! A form that has not been customised is generated from the schema and is not
//! stored. The first change to it **materialises** it: the generated design is
//! stored, and edited from then on. Edits to a design are not in the
//! workbook's undo history, like module edits and tab operations; layout mode
//! has its own undo and redo, which last while it is on.

use cliotree_calc::{ExprKind, REPORT_FORM, entry};
use cliotree_core::form::{
    self as design, Address, Alignment, Anchor, CONTROL_HEIGHTS, CONTROL_WIDTHS, ColumnDesign,
    Control, ControlKind, DEFAULT_LABEL_WIDTH, Editor, FormDesign, GroupLayout, LABEL_WIDTHS,
    LabelPosition, MAX_TRACKS, POSITIONS, RepeaterMode, SEPARATOR_THICKNESS, SPACINGS,
    TRACK_POINTS, TRACK_SHARES, TrackSize, flow_rows, free_place, grid_cells, rebase_free,
};
use cliotree_core::{Kind, Path, Value};

use crate::app::ClioApp;
use crate::form::{DEFAULT_FORM, View, control_name, last_name};
use crate::form_free::{Arrange, FormRects, GRID, Placement, arranged, last_rects};
use crate::icons;
use crate::view::{error_color, named};

/// What layout mode keeps while it is on.
#[derive(Debug, Clone)]
pub struct LayoutState {
    /// The tab and the form being edited.
    pub sheet: String,
    pub form: String,
    pub selected: Option<Address>,
    /// The other controls selected with Ctrl and a click, for the align
    /// tools; `selected` is the one they follow.
    pub also: Vec<Address>,
    /// A column of the table of the selected control, when one is selected
    /// rather than the control.
    pub column: Option<String>,
    /// What the form was before each change: `None` is a form that was not
    /// stored, a generated one.
    pub undo: Vec<Option<FormDesign>>,
    pub redo: Vec<Option<FormDesign>>,
    /// The property last typed into, so that typing is one change and not one
    /// for each letter.
    merge: Option<String>,
    /// The formula being typed for the node of the selected field.
    formula: Option<(Path, String)>,
    /// Counts the drags of resize handles, so that each drag is one change.
    gesture: u64,
}

impl LayoutState {
    fn new(sheet: &str, form: &str) -> LayoutState {
        LayoutState {
            sheet: sheet.to_owned(),
            form: form.to_owned(),
            selected: None,
            also: Vec::new(),
            column: None,
            undo: Vec::new(),
            redo: Vec::new(),
            merge: None,
            formula: None,
            gesture: 0,
        }
    }
}

/// A command on a control, from its menu or the property page.
#[derive(Debug, Clone)]
pub(crate) enum ControlAction {
    Select(Address),
    /// Adds a control to the selection, or takes it out (Ctrl and a click).
    Toggle(Address),
    Shift(Address, bool),
    /// Into the group above (`true`), or out of the group.
    Nest(Address, bool),
    Delete(Address),
    /// Select a column of the table of a repeater, by its name.
    SelectColumn(Address, String),
    /// Move or show a column of the table of a repeater.
    Column(Address, String, ColumnEdit),
    /// What was dragged, put down at a place in the list of a group (the top
    /// level for an empty address), counted as the list was before; in a
    /// Free group, at a point of its inside, which is the size given.
    Drop(DragItem, Address, usize, Option<DropAt>),
    /// A resize handle, or a control in a Free group, starts to be dragged.
    ResizeStart,
    /// Where controls are and how big, as seen; edits with the same merge key
    /// (`move` and `resize` are per drag) are one step of undo.
    Place(Vec<Placement>, Option<String>),
    /// A new width and height, each left alone when `None`, made automatic
    /// with `Some(None)`.
    Resize(Address, Option<Option<u32>>, Option<Option<u32>>),
}

/// Where in a Free or Grid group something dragged is put down.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum DropAt {
    Free(FreeDrop),
    /// The row and column of a cell, from 0.
    Cell(usize, usize),
}

/// Where in a Free group something dragged is put down: the point in its
/// inside, and the size of the inside.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct FreeDrop {
    pub point: (f32, f32),
    pub size: (f32, f32),
}

/// What is dragged in layout mode: a control of the form, or a new one from
/// the palette. The payload of egui's drag and drop is this with the name to
/// show beside the pointer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DragItem {
    Control(Address),
    New(ControlKind),
}

/// A new control of a kind, as the palette adds it.
pub(crate) fn new_control(kind: ControlKind) -> Control {
    let mut c = Control::new(kind);
    match kind {
        ControlKind::Label => c.text = Some("Label".to_owned()),
        ControlKind::Button => c.label = Some("Button".to_owned()),
        _ => {}
    }
    c
}

/// Whether putting `item` down at `index` of `parent` changes anything and
/// can be done: a control does not go into itself, and putting it back
/// where it is is no move.
pub(crate) fn drop_allowed(item: &DragItem, parent: &[usize], index: usize) -> bool {
    match item {
        DragItem::New(_) => true,
        DragItem::Control(from) => {
            let Some((&last, from_parent)) = from.split_last() else {
                return false;
            };
            !parent.starts_with(from)
                && !(parent == from_parent && (index == last || index == last + 1))
        }
    }
}

/// A change to the place of a column, or to whether it is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ColumnEdit {
    /// One place earlier (`true`) or later in the list of the table.
    Step(bool),
    /// To the left (`true`) or the right of the next column that is shown.
    Move(bool),
    Show(bool),
    /// A width of its own, in points, as dragging the edge of its title
    /// gives it.
    Width(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormCommand {
    New,
    Duplicate,
    Rename,
    /// Delete a form that is not Default.
    Delete,
    /// Discard the design of Default, so that it is generated again.
    Reset,
}

/// A command on forms that waits for a name, or for a yes.
#[derive(Debug, Clone)]
pub struct FormDialog {
    pub command: FormCommand,
    pub name: String,
    pub error: Option<String>,
}

impl ClioApp {
    // ----------------------------------------------------------- designs

    /// The design of a form of the current tab: the one that is stored, or the
    /// one generated from the schema.
    pub fn design_of(&self, form: &str) -> FormDesign {
        if let Some(stored) = self.stored_design(form) {
            return stored;
        }
        self.calc()
            .spec(&self.sheet)
            .map(design::generate)
            .unwrap_or_default()
    }

    /// Whether the form has been customised (stored), as opposed to generated.
    pub fn is_customised(&self, form: &str) -> bool {
        self.stored_design(form).is_some()
    }

    /// The design stored for a form of the current tab; on a report tab,
    /// [`REPORT_FORM`] is the report's layout, which is always stored.
    pub(crate) fn stored_design(&self, form: &str) -> Option<FormDesign> {
        if form == REPORT_FORM {
            return self.calc().report(&self.sheet).map(|r| r.layout.clone());
        }
        self.calc().forms().get(&self.sheet, form).cloned()
    }

    fn put_design(&mut self, form: &str, design: Option<FormDesign>) {
        let sheet = self.sheet.clone();
        if form == REPORT_FORM {
            if let Some(mut report) = self.calc().report(&sheet).cloned() {
                report.layout = design.unwrap_or_default();
                // The report is there: it was just read.
                let _ = self.session.calc_mut().set_report(&sheet, report);
            }
            return;
        }
        let forms = self.session.calc_mut().forms_mut();
        match design {
            Some(design) => forms.set(&sheet, form, design),
            None => {
                forms.remove(&sheet, form);
            }
        }
    }

    /// Changes the design of the form being shown. `change` returns what to
    /// select afterwards, or None if it changed nothing. Returns whether a
    /// change was made. The first change to a generated form stores it.
    pub fn edit_design(
        &mut self,
        change: impl FnOnce(&mut FormDesign) -> Option<Option<Address>>,
    ) -> bool {
        self.edit_design_merging(None, change)
    }

    pub(crate) fn edit_design_merging(
        &mut self,
        merge: Option<String>,
        change: impl FnOnce(&mut FormDesign) -> Option<Option<Address>>,
    ) -> bool {
        let form = self.form_name();
        let before = self.stored_design(&form);
        let mut design = before.clone().unwrap_or_else(|| self.design_of(&form));
        let Some(selection) = change(&mut design) else {
            return false;
        };
        if before.as_ref() == Some(&design) {
            return false;
        }
        // The bands of a multipage report stay its bands: only adding or
        // removing a section changes them.
        if form == REPORT_FORM
            && !self.report.restructure
            && let Some(report) = self.calc().report(&self.sheet)
            && report.bands().is_some()
            && cliotree_core::report::band_sections(&design) != report.bands()
        {
            return false;
        }
        self.put_design(&form, Some(design));
        let sheet = self.sheet.clone();
        if let Some(layout) = self
            .form
            .layout
            .as_mut()
            .filter(|l| l.sheet == sheet && l.form == form)
        {
            let joins = merge.is_some() && layout.merge == merge;
            if !joins {
                layout.undo.push(before);
            }
            layout.redo.clear();
            layout.merge = merge;
            if layout.selected != selection {
                layout.column = None;
                layout.also.clear();
            }
            layout.selected = selection;
        }
        true
    }

    // ------------------------------------------------------- layout mode

    /// In layout mode on the form being shown: the selected control, if any.
    pub(crate) fn layout_selection(&self, form: &str) -> Option<Option<Address>> {
        self.form
            .layout
            .as_ref()
            .filter(|l| l.sheet == self.sheet && l.form == form)
            .map(|l| l.selected.clone())
    }

    /// Whether the form being shown, or the report, is in layout mode.
    pub fn layout_active(&self) -> bool {
        (self.in_form() || self.in_report()) && self.layout_selection(&self.form_name()).is_some()
    }

    /// Turns layout mode on or off for the form being shown, or the report
    /// (*Edit report*).
    pub fn set_layout(&mut self, on: bool) {
        if on && (self.in_form() || self.in_report()) {
            let form = self.form_name();
            self.form.layout = Some(LayoutState::new(&self.sheet, &form));
        } else {
            self.form.layout = None;
        }
    }

    /// The control selected in layout mode.
    pub fn selected_control(&self) -> Option<Address> {
        self.layout_selection(&self.form_name()).flatten()
    }

    /// Selects a control in layout mode.
    pub fn select_control(&mut self, address: Option<Address>) {
        if let Some(layout) = self.form.layout.as_mut() {
            layout.selected = address;
            layout.also.clear();
            layout.column = None;
            layout.merge = None;
        }
    }

    /// Adds a control to the selection in layout mode, or takes it out if it
    /// is in it: the first selected stays the one the others follow.
    pub fn toggle_control(&mut self, at: Address) {
        let Some(layout) = self.form.layout.as_mut() else {
            return;
        };
        layout.column = None;
        layout.merge = None;
        if layout.selected.as_ref() == Some(&at) {
            layout.selected = (!layout.also.is_empty()).then(|| layout.also.remove(0));
        } else if let Some(i) = layout.also.iter().position(|a| a == &at) {
            layout.also.remove(i);
        } else if layout.selected.is_none() {
            layout.selected = Some(at);
        } else {
            layout.also.push(at);
        }
    }

    /// Every control selected in layout mode, the one the others follow first.
    pub fn selected_controls(&self) -> Vec<Address> {
        let Some(layout) = self.form.layout.as_ref() else {
            return Vec::new();
        };
        layout
            .selected
            .iter()
            .chain(&layout.also)
            .cloned()
            .collect()
    }

    /// The column selected in layout mode: the repeater, and the column's name.
    pub fn selected_column(&self) -> Option<(Address, String)> {
        let at = self.selected_control()?;
        let column = self.form.layout.as_ref()?.column.clone()?;
        Some((at, column))
    }

    /// Selects a column of the table of a repeater in layout mode.
    pub fn select_table_column(&mut self, at: Address, name: &str) {
        if let Some(layout) = self.form.layout.as_mut() {
            layout.selected = Some(at);
            layout.also.clear();
            layout.column = Some(name.to_owned());
            layout.merge = None;
        }
    }

    pub fn can_layout_undo(&self) -> bool {
        self.form
            .layout
            .as_ref()
            .is_some_and(|l| !l.undo.is_empty())
    }

    pub fn can_layout_redo(&self) -> bool {
        self.form
            .layout
            .as_ref()
            .is_some_and(|l| !l.redo.is_empty())
    }

    /// Takes back the last change to the design.
    pub fn layout_undo(&mut self) {
        self.step_layout(true);
    }

    pub fn layout_redo(&mut self) {
        self.step_layout(false);
    }

    fn step_layout(&mut self, undo: bool) {
        let Some(layout) = self.form.layout.as_mut() else {
            return;
        };
        let target = if undo {
            layout.undo.pop()
        } else {
            layout.redo.pop()
        };
        let Some(target) = target else {
            return;
        };
        layout.selected = None;
        layout.also.clear();
        layout.column = None;
        layout.merge = None;
        let form = layout.form.clone();
        let current = self.stored_design(&form);
        if let Some(layout) = self.form.layout.as_mut() {
            if undo {
                layout.redo.push(current);
            } else {
                layout.undo.push(current);
            }
        }
        self.put_design(&form, target);
    }

    pub(crate) fn apply_control(&mut self, action: ControlAction) {
        match action {
            ControlAction::Select(at) => self.select_control(Some(at)),
            ControlAction::Toggle(at) => self.toggle_control(at),
            ControlAction::Shift(at, up) => {
                self.edit_design(|d| d.shift(&at, up).map(Some));
            }
            ControlAction::Nest(at, into) => {
                self.edit_design(|d| d.nest(&at, into).map(Some));
            }
            ControlAction::Delete(at) => {
                self.edit_design(|d| d.remove(&at).map(|_| None));
            }
            ControlAction::SelectColumn(at, name) => self.select_table_column(at, &name),
            ControlAction::Column(at, name, edit) => {
                self.edit_column_place(&at, &name, edit);
            }
            ControlAction::Drop(item, parent, index, at_point) => {
                if !drop_allowed(&item, &parent, index) {
                    return;
                }
                self.edit_design(|d| {
                    let at = match item {
                        DragItem::Control(from) => d.move_to(&from, &parent, index)?,
                        DragItem::New(kind) => d.insert(&parent, Some(index), new_control(kind))?,
                    };
                    match at_point {
                        Some(DropAt::Free(free)) => {
                            let group = d.get_mut(&parent)?;
                            rebase_free(group, free.size);
                            let c = d.get_mut(&at)?;
                            c.x = Some(position(snap_grid(free.point.0)));
                            c.y = Some(position(snap_grid(free.point.1)));
                        }
                        Some(DropAt::Cell(row, column)) => {
                            let c = d.get_mut(&at)?;
                            c.grid_row = Some(track_number(row));
                            c.grid_column = Some(track_number(column));
                        }
                        None => {}
                    }
                    Some(Some(at))
                });
            }
            ControlAction::Place(placements, merge) => {
                let gesture = self.form.layout.as_ref().map_or(0, |l| l.gesture);
                let merge = merge.map(|m| match m.as_str() {
                    "move" | "resize" => format!("{m}:{gesture}"),
                    _ => m,
                });
                self.place_controls(&placements, merge);
            }
            ControlAction::ResizeStart => {
                if let Some(layout) = self.form.layout.as_mut() {
                    layout.gesture += 1;
                }
            }
            ControlAction::Resize(at, width, height) => {
                let gesture = self.form.layout.as_ref().map_or(0, |l| l.gesture);
                self.resize_control(&at, width, height, Some(format!("resize:{gesture}")));
            }
        }
    }

    /// Puts controls where the placements say, as they are seen: the Free
    /// group of each is first made to keep every control where it is seen
    /// at its size (so that the others do not move), then the control is
    /// given its position, size and anchors. A width ends `fill`. The
    /// selection is kept.
    pub fn place_controls(&mut self, placements: &[Placement], merge: Option<String>) -> bool {
        let keep = self.selected_control();
        self.edit_design_merging(merge, |d| {
            let mut rebased: Vec<Address> = Vec::new();
            for p in placements {
                let (_, parent) = p.at.split_last()?;
                if let Some(size) = p.size
                    && !rebased.iter().any(|r| r == parent)
                    && d.get(parent).is_some_and(Control::is_free)
                {
                    rebase_free(d.get_mut(parent)?, size);
                    rebased.push(parent.to_vec());
                }
                let c = d.get_mut(&p.at)?;
                if let Some(x) = p.x {
                    c.x = Some(position(x));
                }
                if let Some(y) = p.y {
                    c.y = Some(position(y));
                }
                if let Some(w) = p.width {
                    c.width = Some(clamped(w, &CONTROL_WIDTHS));
                    c.fill = false;
                }
                if let Some(h) = p.height {
                    c.height = Some(clamped(h, &CONTROL_HEIGHTS));
                }
                if let Some(anchor) = p.anchor_x {
                    c.anchor_x = (anchor != Anchor::Start).then_some(anchor);
                }
                if let Some(anchor) = p.anchor_y {
                    c.anchor_y = (anchor != Anchor::Start).then_some(anchor);
                }
                if let Some((row, column)) = p.cell {
                    c.grid_row = Some(track_number(row));
                    c.grid_column = Some(track_number(column));
                }
            }
            Some(keep)
        })
    }

    /// In layout mode, with no text being typed: the arrow keys move the
    /// selected controls that are in Free groups by one point, or by one step
    /// of the grid with Shift. The presses are one step of undo.
    pub(crate) fn layout_keys(&mut self, ui: &mut egui::Ui) {
        // From the events, which say what was held with each key.
        let (dx, dy) = ui.input_mut(|i| {
            let (mut dx, mut dy) = (0.0, 0.0);
            i.events.retain(|e| {
                let egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } = e
                else {
                    return true;
                };
                if modifiers.command || modifiers.alt {
                    return true;
                }
                let step = if modifiers.shift { GRID } else { 1.0 };
                match key {
                    egui::Key::ArrowRight => dx += step,
                    egui::Key::ArrowLeft => dx -= step,
                    egui::Key::ArrowDown => dy += step,
                    egui::Key::ArrowUp => dy -= step,
                    _ => return true,
                }
                false
            });
            (dx, dy)
        });
        if dx == 0.0 && dy == 0.0 {
            return;
        }
        let rects = last_rects(ui.ctx(), &self.sheet, &self.form_name());
        self.nudge(&rects, dx, dy);
    }

    /// Moves the selected controls that are in Free groups, as they were
    /// drawn, by `dx` and `dy` points.
    pub(crate) fn nudge(&mut self, rects: &FormRects, dx: f32, dy: f32) {
        let placements: Vec<Placement> = self
            .selected_controls()
            .into_iter()
            .filter_map(|at| {
                let rect = rects.controls.get(&at)?;
                let region = rects.free.get(at.split_last()?.1)?;
                Some(Placement {
                    x: Some((rect.left() - region.left() + dx).max(0.0)),
                    y: Some((rect.top() - region.top() + dy).max(0.0)),
                    size: Some((region.width(), region.height())),
                    at,
                    ..Placement::default()
                })
            })
            .collect();
        if !placements.is_empty() {
            self.place_controls(&placements, Some("nudge".to_owned()));
        }
    }

    /// Sets the width and height of a control: each is left alone when
    /// `None`, and made automatic with `Some(None)`. A width also ends
    /// `fill`. Edits with the same `merge` are one step of undo.
    pub fn resize_control(
        &mut self,
        at: &Address,
        width: Option<Option<u32>>,
        height: Option<Option<u32>>,
        merge: Option<String>,
    ) -> bool {
        let at = at.clone();
        self.edit_design_merging(merge, |d| {
            let control = d.get_mut(&at)?;
            if let Some(width) = width {
                control.width = width;
                control.fill = false;
            }
            if let Some(height) = height {
                control.height = height;
            }
            Some(Some(at))
        })
    }

    /// Adds a control to the form: into the selected group, or after the
    /// selected control, or at the end. The new control is selected.
    pub fn add_control(&mut self, control: Control) -> bool {
        let selected = self.selected_control();
        self.edit_design(|d| {
            let at = match selected {
                Some(at) if d.takes_children(&at) => d.insert(&at, None, control),
                Some(at) => {
                    let (last, parent) = at.split_last()?;
                    d.insert(parent, Some(last + 1), control)
                }
                None => d.insert(&[], None, control),
            };
            at.map(Some)
        })
    }

    /// Adds the control the generated form has for a field that is not on the
    /// form.
    pub fn add_field_to_form(&mut self, path: &Path) -> bool {
        let control = self
            .calc()
            .spec(&self.sheet)
            .and_then(|spec| design::control_for(spec, path));
        let Some(control) = control else {
            return false;
        };
        // Its path is the whole path, so it goes after the stacked repeater
        // whose items the selection is in, or that is selected.
        let design = self.design_of(&self.form_name());
        let selected = self.selected_control();
        let outside = selected.as_ref().and_then(|at| {
            design.item_scope(at).or_else(|| {
                design
                    .get(at)
                    .filter(|c| c.kind == ControlKind::Repeater)
                    .map(|_| at.clone())
            })
        });
        match outside {
            Some(repeater) => self.edit_design(|d| {
                let (last, parent) = repeater.split_last()?;
                d.insert(parent, Some(last + 1), control).map(Some)
            }),
            None => self.add_control(control),
        }
    }

    // ------------------------------------------------------- the columns

    /// The names of the columns of the table of the repeater at `at`, in the
    /// order of the schema, if it is a table.
    fn column_names(&self, design: &FormDesign, at: &[usize]) -> Option<Vec<String>> {
        let control = design.get(at)?;
        if control.lists_items() {
            // For the first item of each repeater around.
            let around = (1..at.len())
                .filter(|&n| {
                    design
                        .get(&at[..n])
                        .is_some_and(|c| c.kind == ControlKind::Repeater)
                })
                .count();
            let state = self.calc().control_expression_at(
                &self.sheet,
                &self.form_name(),
                at,
                &vec![0; around],
                ExprKind::Items,
            );
            let items = match state.and_then(|s| s.result) {
                Some(Ok(Value::Array(items))) => items.iter().map(|n| n.value().clone()).collect(),
                _ => Vec::new(),
            };
            return Some(crate::form::listed_names(control, &items));
        }
        let spec = self.calc().spec(&self.sheet)?;
        let columns = crate::form::columns_of(spec, design, at)?;
        let mut names: Vec<String> = columns.into_iter().map(|c| c.name).collect();
        // And the columns of the design that show an expression.
        for column in &control.table_columns {
            if column.value.is_some() && !names.contains(&column.name) {
                names.push(column.name.clone());
            }
        }
        Some(names)
    }

    /// Changes the columns of the table of the repeater at `at`, as the
    /// control arranges them. `change` returns whether it changed anything.
    fn edit_columns(
        &mut self,
        merge: Option<String>,
        at: &Address,
        change: impl FnOnce(&mut Vec<ColumnDesign>) -> bool,
    ) -> bool {
        let design = self.design_of(&self.form_name());
        let Some(names) = self.column_names(&design, at) else {
            return false;
        };
        let at = at.clone();
        self.edit_design_merging(merge, |d| {
            let control = d.get_mut(&at)?;
            let mut columns = control.arrange_columns(&names);
            if !change(&mut columns) {
                return None;
            }
            control.set_columns(columns, &names);
            Some(Some(at))
        })
    }

    /// Moves a column, or shows or hides it. The last column of the items
    /// that is shown cannot be hidden.
    pub(crate) fn edit_column_place(&mut self, at: &Address, name: &str, edit: ColumnEdit) -> bool {
        self.edit_columns(None, at, |columns| {
            let Some(i) = columns.iter().position(|c| c.name == name) else {
                return false;
            };
            match edit {
                ColumnEdit::Width(width) => {
                    if columns[i].width == Some(width) {
                        return false;
                    }
                    columns[i].width = Some(width);
                    true
                }
                ColumnEdit::Show(on) => {
                    let shown = columns
                        .iter()
                        .filter(|c| !c.hidden && !c.is_row_number())
                        .count();
                    let last = !columns[i].is_row_number() && !columns[i].hidden && shown == 1;
                    if columns[i].hidden != on || (!on && last) {
                        return false;
                    }
                    columns[i].hidden = !on;
                    true
                }
                ColumnEdit::Step(up) | ColumnEdit::Move(up) => {
                    let past_hidden = matches!(edit, ColumnEdit::Move(_));
                    let target = if up {
                        (0..i).rev().find(|&j| !past_hidden || !columns[j].hidden)
                    } else {
                        (i + 1..columns.len()).find(|&j| !past_hidden || !columns[j].hidden)
                    };
                    let Some(j) = target else {
                        return false;
                    };
                    if up {
                        columns[j..=i].rotate_right(1);
                    } else {
                        columns[i..=j].rotate_left(1);
                    }
                    true
                }
            }
        })
    }

    /// Changes the properties of one column of the table at `at`. A column
    /// that is selected stays selected.
    pub fn set_column(
        &mut self,
        merge: Option<String>,
        at: &Address,
        column: ColumnDesign,
    ) -> bool {
        self.edit_columns(merge, at, |columns| {
            match columns.iter_mut().find(|c| c.name == column.name) {
                Some(slot) if *slot != column => {
                    *slot = column;
                    true
                }
                _ => false,
            }
        })
    }

    // ------------------------------------------------------- the commands

    fn check_form_name(&self, name: &str, renaming: Option<&str>) -> Result<(), String> {
        if name.eq_ignore_ascii_case("Raw") {
            return Err("Raw is the name of a view of every tab".to_owned());
        }
        match self
            .calc()
            .forms()
            .name_problem(&self.sheet, name, renaming)
        {
            Some(why) => Err(why),
            None => Ok(()),
        }
    }

    /// Makes a form, starting from the generated one, and shows it.
    pub fn new_form(&mut self, name: &str) -> Result<(), String> {
        self.check_form_name(name, None)?;
        let design = self
            .calc()
            .spec(&self.sheet)
            .map(design::generate)
            .ok_or("there is no such tab")?;
        self.put_design(name, Some(design));
        self.show_form(name);
        Ok(())
    }

    /// Makes a copy of the form being shown, and shows the copy.
    pub fn duplicate_form(&mut self, name: &str) -> Result<(), String> {
        self.check_form_name(name, None)?;
        let design = self.design_of(&self.form_name());
        self.put_design(name, Some(design));
        self.show_form(name);
        Ok(())
    }

    /// Renames the form being shown. Default cannot be renamed.
    pub fn rename_form(&mut self, to: &str) -> Result<(), String> {
        let from = self.form_name();
        self.check_form_name(to, Some(&from))?;
        let sheet = self.sheet.clone();
        self.session
            .calc_mut()
            .forms_mut()
            .rename(&sheet, &from, to)?;
        self.views.insert(sheet, View::Form(to.to_owned()));
        self.form.layout = None;
        Ok(())
    }

    /// Deletes the form being shown, and shows Default. Default is not
    /// deleted: see [`ClioApp::reset_form`].
    pub fn delete_form(&mut self) -> Result<(), String> {
        let form = self.form_name();
        if form == DEFAULT_FORM {
            return Err("the Default form cannot be deleted; reset it instead".to_owned());
        }
        self.put_design(&form, None);
        self.show_form(DEFAULT_FORM);
        Ok(())
    }

    /// Discards the design of Default, so that it is generated again.
    pub fn reset_form(&mut self) -> Result<(), String> {
        if self.form_name() != DEFAULT_FORM {
            return Err("only the Default form is reset; delete the others".to_owned());
        }
        self.put_design(DEFAULT_FORM, None);
        self.form.layout = None;
        Ok(())
    }

    fn show_form(&mut self, name: &str) {
        self.form.layout = None;
        self.raw.error = None;
        self.views
            .insert(self.sheet.clone(), View::Form(name.to_owned()));
        self.notice = None;
    }

    /// Opens the dialog of a command on forms.
    pub fn begin_form_command(&mut self, command: FormCommand) {
        let form = self.form_name();
        let name = match command {
            FormCommand::New => String::new(),
            FormCommand::Duplicate => format!("{form}_copy"),
            FormCommand::Rename => form,
            FormCommand::Delete | FormCommand::Reset => String::new(),
        };
        self.form.dialog = Some(FormDialog {
            command,
            name,
            error: None,
        });
    }

    /// Does what the open dialog says. If it is refused, the dialog stays open
    /// with the reason.
    pub fn apply_form_dialog(&mut self) {
        let Some(mut dialog) = self.form.dialog.take() else {
            return;
        };
        let name = dialog.name.trim().to_owned();
        let result = match dialog.command {
            FormCommand::New => self.new_form(&name),
            FormCommand::Duplicate => self.duplicate_form(&name),
            FormCommand::Rename => self.rename_form(&name),
            FormCommand::Delete => self.delete_form(),
            FormCommand::Reset => self.reset_form(),
        };
        if let Err(message) = result {
            dialog.error = Some(message);
            self.form.dialog = Some(dialog);
        }
    }

    /// The dialog of a command on forms, when one is waiting.
    pub(crate) fn form_window(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.form.dialog.take() else {
            return;
        };
        let form = self.form_name();
        let title = match dialog.command {
            FormCommand::New => "New form".to_owned(),
            FormCommand::Duplicate => format!("Duplicate form {form}"),
            FormCommand::Rename => format!("Rename form {form}"),
            FormCommand::Delete => format!("Delete form {form}"),
            FormCommand::Reset => "Reset the Default form".to_owned(),
        };
        let mut open = true;
        let (mut apply, mut cancel) = (false, false);
        egui::Window::new(title)
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                match dialog.command {
                    FormCommand::New | FormCommand::Duplicate | FormCommand::Rename => {
                        ui.label(match dialog.command {
                            FormCommand::New => "Name of the new form",
                            FormCommand::Duplicate => "Name of the copy",
                            _ => "New name",
                        });
                        let response = ui.text_edit_singleline(&mut dialog.name);
                        named(&response, "Form name");
                        if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            apply = true;
                        }
                        ui.label(
                            "A form name follows the rules of a tab name, and is not used \
                             by another form of this tab.",
                        );
                    }
                    FormCommand::Delete => {
                        ui.label(format!("Delete the form {form}? Its design is lost."));
                    }
                    FormCommand::Reset => {
                        ui.label(
                            "Discard the design of the Default form? It is generated from \
                             the schema again.",
                        );
                    }
                }
                if let Some(error) = &dialog.error {
                    ui.colored_label(error_color(ui), error);
                }
                ui.horizontal(|ui| {
                    let label = match dialog.command {
                        FormCommand::New => "Create",
                        FormCommand::Duplicate => "Duplicate",
                        FormCommand::Rename => "Rename",
                        FormCommand::Delete => "Delete",
                        FormCommand::Reset => "Reset",
                    };
                    apply |= ui.button(label).clicked();
                    cancel = ui.button("Cancel").clicked();
                });
            });
        if apply {
            self.form.dialog = Some(dialog);
            self.apply_form_dialog();
        } else if open && !cancel {
            self.form.dialog = Some(dialog);
        }
    }

    // --------------------------------------------------------- the menu

    /// The Forms submenu of the View menu.
    pub(crate) fn forms_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("Forms", |ui| {
            let in_form = self.in_form();
            let form = self.form_name();
            let is_default = form == DEFAULT_FORM;
            let mut command = None;
            let mut item = |ui: &mut egui::Ui, label: &str, enabled: bool, c: FormCommand| {
                if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                    command = Some(c);
                    ui.close();
                }
            };
            item(ui, "New form...", true, FormCommand::New);
            item(ui, "Duplicate form...", in_form, FormCommand::Duplicate);
            item(
                ui,
                "Rename form...",
                in_form && !is_default,
                FormCommand::Rename,
            );
            item(
                ui,
                "Delete form...",
                in_form && !is_default,
                FormCommand::Delete,
            );
            item(
                ui,
                "Reset Default form...",
                in_form && is_default && self.is_customised(&form),
                FormCommand::Reset,
            );
            ui.separator();
            let mut on = self.layout_active();
            if ui
                .add_enabled(in_form, egui::Checkbox::new(&mut on, "Edit form"))
                .clicked()
            {
                self.set_layout(on);
                ui.close();
            }
            if let Some(command) = command {
                self.begin_form_command(command);
            }
        });
    }

    // ------------------------------------------------------- the strip

    /// The strip above a form: the switch to layout mode, and in layout mode
    /// the palette, undo and redo, and discarding the design.
    pub(crate) fn form_bar(&mut self, ui: &mut egui::Ui, form: &str) {
        let report = form == REPORT_FORM;
        ui.horizontal_wrapped(|ui| {
            let mut on = self.layout_active();
            let switch = if report { "Edit report" } else { "Edit form" };
            if ui.checkbox(&mut on, switch).changed() {
                self.set_layout(on);
            }
            if report {
                self.report_tools(ui);
            }
            if !on {
                return;
            }
            ui.separator();
            // A click adds where the selection says; dragging one onto the
            // form puts it where it is dropped. A report has no buttons.
            for kind in ControlKind::ALL {
                // A report has no buttons, and no repeaters: a list is the
                // detail band of a multipage report.
                if report && matches!(kind, ControlKind::Button | ControlKind::Repeater) {
                    continue;
                }
                let label = format!("Add {}", kind.label().to_lowercase());
                let button = ui
                    .add(egui::Button::new(label).sense(egui::Sense::click_and_drag()))
                    .on_hover_text("Click to add, or drag onto the form");
                button.dnd_set_drag_payload((DragItem::New(kind), kind.label().to_owned()));
                if button.clicked() {
                    if report {
                        self.add_to_canvas(kind);
                    } else {
                        self.add_control(new_control(kind));
                    }
                }
            }
            ui.separator();
            if ui
                .add_enabled(self.can_layout_undo(), egui::Button::new("Undo layout"))
                .clicked()
            {
                self.layout_undo();
            }
            if ui
                .add_enabled(self.can_layout_redo(), egui::Button::new("Redo layout"))
                .clicked()
            {
                self.layout_redo();
            }
            if report {
                return;
            }
            ui.separator();
            if form == DEFAULT_FORM {
                let stored = self.is_customised(form);
                if ui
                    .add_enabled(stored, egui::Button::new("Reset to default"))
                    .clicked()
                {
                    self.begin_form_command(FormCommand::Reset);
                }
            } else if ui.button("Delete form").clicked() {
                self.begin_form_command(FormCommand::Delete);
            }
        });
    }

    // --------------------------------------------------- the property page

    /// The property page of layout mode: the selected control, the fields
    /// that are not on the form, and the controls that are broken.
    pub(crate) fn property_page(&mut self, ui: &mut egui::Ui) {
        let form = self.form_name();
        let design = self.design_of(&form);
        let selected = self.selected_control();
        let column = self.selected_column();
        ui.heading("Properties");
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let column = column.and_then(|(at, name)| {
                    let control = design.get(&at)?;
                    let names = self.column_names(&design, &at)?;
                    let column = control
                        .arrange_columns(&names)
                        .into_iter()
                        .find(|c| c.name == name)?;
                    Some((at, control.clone(), column))
                });
                if let Some((at, control, column)) = column {
                    self.column_properties(ui, &at, &control, column);
                } else {
                    self.arrange_tools(ui);
                    match selected
                        .as_ref()
                        .and_then(|at| design.get(at).map(|c| (at, c)))
                    {
                        Some((at, _)) if self.is_band(at) => self.band_properties(ui, at[0]),
                        Some((at, control)) => self.control_properties(ui, &design, at, control),
                        None if self.in_report() => self.report_properties(ui),
                        None => {
                            ui.weak("Select a control on the form to change it.");
                        }
                    }
                }
                ui.separator();
                self.unplaced_list(ui, &design);
                self.broken_list(ui, &design);
            });
    }

    fn control_properties(
        &mut self,
        ui: &mut egui::Ui,
        design: &FormDesign,
        at: &Address,
        control: &Control,
    ) {
        let mut edited = control.clone();
        let spec = self.calc().spec(&self.sheet).cloned();
        // A report shows values and edits nothing (DESIGN.md, The report
        // designer): no paths, editors, choices or procedures.
        let report = self.in_report();
        ui.strong(control_name(control));
        let key = |property: &str| format!("{at:?}:{property}");
        let mut merge = None;
        let mut column_edit = None;
        let mut tab_action: Option<(Address, bool)> = None;
        let mut placement = None;
        let kind = control.kind;
        let bound = matches!(
            kind,
            ControlKind::Field | ControlKind::Group | ControlKind::Repeater
        );

        // The stacked repeater whose items the control is designed in, and
        // the type of those items: its path is relative to the item.
        let item_ty = spec.as_ref().and_then(|spec| {
            let scope = design.item_scope(at)?;
            let array = design.get(&scope)?.bound_path()?;
            match &spec.get(spec.resolve(&array)?.ty).kind {
                Kind::Array { items, .. } => Some(*items),
                _ => None,
            }
        });
        if item_ty.is_some() {
            ui.weak("In the design of the items: paths are read from the item.");
        }

        // A field shows a node (its path) or an expression (its value), not
        // both. Choosing one clears the other; the choice of a value is
        // remembered until the value is typed.
        let source_id = egui::Id::unique(("field_source", at.clone()));
        let mut by_value = kind == ControlKind::Field
            && (report
                || edited.value.is_some()
                || (edited.path.is_none()
                    && ui.data(|d| d.get_temp::<bool>(source_id)).unwrap_or(false)));
        if kind == ControlKind::Field && !report {
            let before = by_value;
            ui.horizontal(|ui| {
                ui.label("Source");
                let response = egui::ComboBox::from_id_salt("property_source")
                    .selected_text(if by_value { "Value" } else { "Path" })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut by_value, false, "Path");
                        ui.selectable_value(&mut by_value, true, "Value");
                    })
                    .response;
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::Role::ComboBox, true, "Source of the field")
                });
            });
            if by_value != before {
                ui.data_mut(|d| d.insert_temp(source_id, by_value));
                if by_value {
                    edited.path = None;
                } else {
                    edited.value = None;
                    edited.write_procedure = None;
                }
            }
        }

        // What the control is bound to.
        if bound
            && !by_value
            && !edited.lists_items()
            && let Some(spec) = &spec
        {
            let options = match item_ty {
                Some(ty) => design::bindable_from(spec, ty, kind),
                None => design::bindable(spec, kind),
            };
            let shown = |path: &str| match (path.is_empty(), item_ty) {
                (true, Some(_)) => "(the item)".to_owned(),
                _ => path.to_owned(),
            };
            let current = edited.path.clone();
            let mut chosen = current.clone();
            ui.horizontal(|ui| {
                ui.label("Path");
                let text = current
                    .as_deref()
                    .map_or_else(|| "(none)".to_owned(), shown);
                let response = egui::ComboBox::from_id_salt("property_path")
                    .selected_text(text)
                    .width(200.0)
                    .show_ui(ui, |ui| {
                        if kind == ControlKind::Group {
                            ui.selectable_value(&mut chosen, None, "(none)");
                        }
                        for path in &options {
                            let text = path.to_string();
                            ui.selectable_value(&mut chosen, Some(text.clone()), shown(&text));
                        }
                    })
                    .response;
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::Role::ComboBox, true, "Path of the control")
                });
            });
            if chosen != current {
                edited.path = chosen;
            }
        }
        if matches!(
            kind,
            ControlKind::Field | ControlKind::Group | ControlKind::Repeater | ControlKind::Button
        ) {
            let hint = edited
                .bound_path()
                .and_then(|p| last_name(&p))
                .unwrap_or_default();
            if option_text(ui, "Label", &mut edited.label, &hint, false) {
                merge = Some(key("label"));
            }
        }
        if bound && !report && option_text(ui, "Help", &mut edited.help, "", true) {
            merge = Some(key("help"));
        }
        match kind {
            ControlKind::Field if by_value => {
                let expr = ExprKind::Value;
                if expr_text(
                    ui,
                    expr_label(expr),
                    &mut edited.value,
                    expr_hint(expr),
                    draft_id(at, None, expr),
                ) {
                    merge = Some(key(expr_property(expr)));
                }
                self.expression_error(ui, at, None, expr);
                if report {
                    ui.weak("The field shows what the value comes to.");
                } else {
                    let names = self.calc().procedure_names();
                    procedure_choice(
                        ui,
                        "Write procedure",
                        "Write procedure of the field",
                        &mut edited.write_procedure,
                        &names,
                    );
                    ui.weak(
                        "The field shows what the value comes to. Without a write procedure it is \
                     read-only; with one, what is typed is passed to it, as text, as \
                     name(form, ctx, value).",
                    );
                }
            }
            ControlKind::Repeater => {
                let expr = ExprKind::Items;
                if expr_text(
                    ui,
                    expr_label(expr),
                    &mut edited.items,
                    expr_hint(expr),
                    draft_id(at, None, expr),
                ) {
                    merge = Some(key(expr_property(expr)));
                }
                self.expression_error(ui, at, None, expr);
                if edited.lists_items() {
                    if option_text(ui, "Item name", &mut edited.item_name, "this", false) {
                        merge = Some(key("item_name"));
                    }
                    if option_text(ui, "List name", &mut edited.list_name, "", false) {
                        merge = Some(key("list_name"));
                    }
                    for name in [&edited.item_name, &edited.list_name].into_iter().flatten() {
                        if !is_name(name.trim()) && !name.trim().is_empty() {
                            ui.label(
                                egui::RichText::new(format!("{name:?} is not a name: use letters, digits and _, not starting with a digit."))
                                    .small()
                                    .color(error_color(ui)),
                            );
                        }
                    }
                    ui.weak(
                        "The items are read-only. The controls inside see each item as this \
                         and by its item name, and the whole list by its list name.",
                    );
                }
                if !edited.is_stacked() && ui.button("Add a column with an expression").clicked() {
                    add_listed_column(
                        self.column_names(design, at).unwrap_or_default(),
                        &mut edited,
                    );
                }
            }
            ControlKind::Label => {
                ui.checkbox(&mut edited.template, "Text is a template")
                    .on_hover_text("Each {expression} in the text is filled in, as in a Python f-string: {total:,.2f}. Write {{ and }} for the braces themselves.");
                if edited.is_template() {
                    self.expression_error(ui, at, None, ExprKind::Template);
                }
            }
            _ => {}
        }
        match kind {
            ControlKind::Field if report => {
                // No schema says what the value is: a date shows the date
                // formats, anything else the number formats.
                let shown = self
                    .calc()
                    .report_expression(&self.sheet, at, &[], ExprKind::Value)
                    .and_then(|s| s.result)
                    .and_then(Result::ok);
                let kind = match shown {
                    Some(Value::Date(_)) => Kind::Date,
                    Some(Value::DateTime(_)) => Kind::DateTime,
                    _ => Kind::Number(Default::default()),
                };
                if format_properties(
                    ui,
                    &kind,
                    &mut edited.thousands,
                    &mut edited.decimals,
                    &mut edited.date_format,
                    &mut edited.percent,
                    &mut edited.prefix,
                    &mut edited.suffix,
                    "field",
                ) {
                    merge = Some(key("date_format"));
                }
            }
            ControlKind::Field => {
                ui.horizontal(|ui| {
                    ui.label("Editor");
                    let mut editor = edited.editor.unwrap_or(Editor::Auto);
                    let response = egui::ComboBox::from_id_salt("property_editor")
                        .selected_text(editor.label())
                        .show_ui(ui, |ui| {
                            for e in Editor::ALL {
                                ui.selectable_value(&mut editor, e, e.label());
                            }
                        })
                        .response;
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::Role::ComboBox, true, "Editor of the field")
                    });
                    edited.editor = (editor != Editor::Auto).then_some(editor);
                });
                ui.checkbox(&mut edited.read_only, "Read only");
                let kind = spec.as_ref().and_then(|spec| {
                    let path = design.path_in_item(at, &edited, 0)?;
                    Some(spec.get(spec.resolve(&path)?.ty).kind.clone())
                });
                // A password box: only a string's text can be dots.
                if matches!(kind, Some(Kind::String(_))) || edited.masked {
                    ui.checkbox(&mut edited.masked, "Masked")
                        .on_hover_text("Dots instead of the text, as in a password box");
                }
                if let Some(kind) = kind
                    && format_properties(
                        ui,
                        &kind,
                        &mut edited.thousands,
                        &mut edited.decimals,
                        &mut edited.date_format,
                        &mut edited.percent,
                        &mut edited.prefix,
                        &mut edited.suffix,
                        "field",
                    )
                {
                    merge = Some(key("date_format"));
                }
            }
            ControlKind::Group => {
                ui.horizontal(|ui| {
                    ui.label("Layout");
                    let mut layout = edited.layout.unwrap_or(GroupLayout::Flow);
                    let response = egui::ComboBox::from_id_salt("property_group_layout")
                        .selected_text(layout.label())
                        .show_ui(ui, |ui| {
                            for l in GroupLayout::ALL {
                                ui.selectable_value(&mut layout, l, l.label());
                            }
                        })
                        .response;
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::Role::ComboBox, true, "Layout of the group")
                    });
                    if layout != edited.layout.unwrap_or(GroupLayout::Flow) {
                        let rects = last_rects(ui.ctx(), &self.sheet, &self.form_name());
                        set_layout(&mut edited, at, layout, &rects);
                    }
                });
                if edited.is_free() || edited.is_grid() {
                    if edited.is_grid() {
                        grid_tracks(ui, &mut edited);
                    }
                    if let Some(action) = tab_order(ui, &edited, at) {
                        tab_action = Some(action);
                    }
                } else {
                    ui.horizontal(|ui| {
                        ui.label("Columns");
                        let mut columns = edited.columns.unwrap_or(1);
                        let response = ui.add(egui::DragValue::new(&mut columns).range(1..=6));
                        response.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::Role::SpinButton,
                                true,
                                "Columns of the group",
                            )
                        });
                        edited.columns = (columns > 1).then_some(columns);
                    });
                }
                if border_properties(ui, &mut edited) {
                    merge = Some(key("border"));
                }
            }
            ControlKind::Repeater => {
                ui.horizontal(|ui| {
                    ui.label("Shown as");
                    let mut mode = edited.mode;
                    let text = mode.map_or("Automatic", RepeaterMode::label);
                    let response = egui::ComboBox::from_id_salt("property_mode")
                        .selected_text(text)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut mode, None, "Automatic");
                            ui.selectable_value(
                                &mut mode,
                                Some(RepeaterMode::Table),
                                RepeaterMode::Table.label(),
                            );
                            ui.selectable_value(
                                &mut mode,
                                Some(RepeaterMode::Stacked),
                                RepeaterMode::Stacked.label(),
                            );
                        })
                        .response;
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::Role::ComboBox,
                            true,
                            "How the array is shown",
                        )
                    });
                    edited.mode = mode;
                });
                self.item_design_buttons(ui, design, at, &mut edited, spec.as_ref());
                column_edit = self.column_list(ui, design, at);
                font_properties(
                    ui,
                    &mut edited.title_font,
                    &mut edited.title_font_size,
                    "Title ",
                    "table title",
                );
                font_properties(
                    ui,
                    &mut edited.header_font,
                    &mut edited.header_font_size,
                    "Column title ",
                    "column titles",
                );
            }
            ControlKind::Label => {
                let typed = if edited.is_template() {
                    expr_text(
                        ui,
                        "Text",
                        &mut edited.text,
                        "",
                        draft_id(at, None, ExprKind::Template),
                    )
                } else {
                    option_text(ui, "Text", &mut edited.text, "", true)
                };
                if typed {
                    merge = Some(key("text"));
                }
            }
            ControlKind::Button => {
                let names = self.calc().procedure_names();
                procedure_choice(
                    ui,
                    "Procedure",
                    "Procedure of the button",
                    &mut edited.procedure,
                    &names,
                );
                if names.is_empty() {
                    ui.weak(
                        "No module has a function yet. Write one, def name(form, ctx):, \
                         in the code editor (Alt+F11).",
                    );
                } else {
                    ui.weak("A function of a module, called as name(form, ctx).");
                }
            }
            ControlKind::Separator => {
                ui.checkbox(&mut edited.vertical, "Vertical")
                    .on_hover_text("A vertical line, as tall as its row or cell");
                let before = edited.thickness;
                optional_number(
                    ui,
                    "Thickness",
                    &mut edited.thickness,
                    &SEPARATOR_THICKNESS,
                    2,
                    "Thickness of the line in points",
                );
                if edited.thickness != before {
                    merge = Some(key("thickness"));
                }
            }
        }
        // On a report's canvas, the top level is placed as in a Free group.
        let canvas = report.then(|| {
            let mut group = Control::new(ControlKind::Group);
            group.layout = Some(GroupLayout::Free);
            group
        });
        let parent = match at.split_last() {
            Some((_, [])) if report => canvas.as_ref(),
            Some((_, parent)) => design.get(parent),
            None => None,
        };
        let in_free = parent.is_some_and(Control::is_free);
        let in_grid = parent.is_some_and(Control::is_grid);
        let parent_columns = parent
            .filter(|_| !in_free && !in_grid)
            .and_then(|parent| parent.columns)
            .unwrap_or(1);
        if size_properties(ui, &mut edited, parent_columns) {
            merge = Some(key("size"));
        }
        if in_free && let Some(parent) = parent {
            let rects = last_rects(ui.ctx(), &self.sheet, &self.form_name());
            placement = free_properties(ui, parent, control, at, &rects);
        }
        if in_grid && let Some(parent) = parent {
            cell_properties(ui, parent, &mut edited, at);
        }
        if kind != ControlKind::Separator {
            font_properties(ui, &mut edited.font, &mut edited.font_size, "", "control");
            emphasis_properties(ui, &mut edited.bold, &mut edited.italic, "control");
        }
        let coloured = kind != ControlKind::Separator;
        let fields = [
            (ExprKind::Visible, &mut edited.visible_when, true),
            (ExprKind::Enabled, &mut edited.enabled_when, !report),
            (ExprKind::TextColour, &mut edited.text_colour, coloured),
            (ExprKind::FillColour, &mut edited.fill_colour, coloured),
        ];
        for (expr, value, shown) in fields {
            if !shown {
                continue;
            }
            if expression_box(ui, expr, value, draft_id(at, None, expr)) {
                merge = Some(key(expr_property(expr)));
            }
            self.expression_error(ui, at, None, expr);
        }
        if kind == ControlKind::Field && !report {
            let expr = ExprKind::Choices;
            if expr_text(
                ui,
                expr_label(expr),
                &mut edited.choices,
                expr_hint(expr),
                draft_id(at, None, expr),
            ) {
                merge = Some(key(expr_property(expr)));
            }
            self.expression_error(ui, at, None, expr);
            let node_type = spec.as_ref().and_then(|spec| {
                let path = design.path_in_item(at, &edited, 0)?;
                Some(spec.get(spec.resolve(&path)?.ty).clone())
            });
            self.choices_note(ui, at, None, node_type.as_ref().map(|t| &t.kind));
            // A field whose type names values (COBOL condition names) has
            // them as its choices when it has none of its own.
            let has_named = node_type.as_ref().is_some_and(|t| !t.choices.is_empty());
            if edited.choices.is_some() || has_named {
                ui.checkbox(&mut edited.allow_other, ALLOW_OTHER)
                    .on_hover_text(ALLOW_OTHER_HELP);
            }
        }
        ui.weak(
            "Python expressions, run where the control is bound. A colour is Fixed (picked; \
             no Python, so it shows in an untrusted workbook too) or an expression giving a \
             name, \"#rrggbb\" or (r, g, b); None is the usual colour. Choices is a list of \
             values, (code, text) pairs or {\"value\": .., \"label\": ..} dicts.",
        );

        if kind == ControlKind::Field && !report {
            self.formula_editor(ui, design, at, &edited);
        }

        // Where the control is.
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            let index = at.last().copied().unwrap_or(0);
            let siblings = sibling_count(design, at);
            let above_group = index > 0
                && at.split_last().is_some_and(|(_, parent)| {
                    let mut above = parent.to_vec();
                    above.push(index - 1);
                    design.takes_children(&above)
                });
            let mut act = |ui: &mut egui::Ui, label: &str, enabled: bool, action: ControlAction| {
                if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                    self.apply_control(action);
                }
            };
            act(
                ui,
                "Move up",
                index > 0,
                ControlAction::Shift(at.clone(), true),
            );
            act(
                ui,
                "Move down",
                index + 1 < siblings,
                ControlAction::Shift(at.clone(), false),
            );
            act(
                ui,
                "Move into the group above",
                above_group,
                ControlAction::Nest(at.clone(), true),
            );
            act(
                ui,
                "Move out of the group",
                at.len() > 1,
                ControlAction::Nest(at.clone(), false),
            );
            act(
                ui,
                "Delete control",
                true,
                ControlAction::Delete(at.clone()),
            );
        });

        if &edited != control {
            let to = edited;
            let at = at.clone();
            self.edit_design_merging(merge, |d| {
                *d.get_mut(&at)? = to;
                Some(Some(at))
            });
        } else if let Some(action) = column_edit {
            self.apply_control(action);
        } else if let Some((child, up)) = tab_action {
            // The group stays selected.
            let group = at.clone();
            self.edit_design(|d| d.shift(&child, up).map(|_| Some(group)));
        } else if let Some((placement, merge)) = placement {
            self.place_controls(&[placement], Some(format!("{at:?}:{merge}")));
        }
    }

    /// With several controls selected: how many, and the align tools, each
    /// enabled when it can be used on them.
    fn arrange_tools(&mut self, ui: &mut egui::Ui) {
        let selected = self.selected_controls();
        if selected.len() < 2 {
            return;
        }
        ui.strong(format!("{} controls selected", selected.len()));
        ui.weak("They follow the first one selected. Ctrl and a click adds or removes one.");
        let rects = last_rects(ui.ctx(), &self.sheet, &self.form_name());
        let mut chosen = None;
        ui.horizontal_wrapped(|ui| {
            for how in Arrange::ALL {
                let placements = arranged(how, &selected, &rects);
                let response = ui.add_enabled(placements.is_some(), egui::Button::new(how.label()));
                let response = response.on_disabled_hover_text(
                    "Moving needs the controls in one Free group; distributing needs three.",
                );
                if response.clicked() {
                    chosen = placements;
                }
            }
        });
        if let Some(placements) = chosen {
            self.place_controls(&placements, None);
        }
        ui.separator();
    }

    /// The columns of the table of a repeater, the row numbers among them, in
    /// their order: whether each is shown, buttons that move it, and one that
    /// opens its properties. Returns what was asked for.
    fn column_list(
        &self,
        ui: &mut egui::Ui,
        design: &FormDesign,
        at: &Address,
    ) -> Option<ControlAction> {
        let control = design.get(at)?;
        let names = self.column_names(design, at)?;
        let columns = control.arrange_columns(&names);
        let shown = columns
            .iter()
            .filter(|c| !c.hidden && !c.is_row_number())
            .count();
        let mut action = None;
        ui.label("Columns of the table");
        let last = columns.len().saturating_sub(1);
        egui::Grid::new("table_columns")
            .num_columns(3)
            .show(ui, |ui| {
                for (i, column) in columns.iter().enumerate() {
                    let name = column.name.clone();
                    let title = if column.is_row_number() {
                        "# (row numbers)".to_owned()
                    } else {
                        name.clone()
                    };
                    let mut on = !column.hidden;
                    // The last column of the items that is shown stays shown.
                    let can = column.hidden || column.is_row_number() || shown > 1;
                    let response = ui.add_enabled(can, egui::Checkbox::new(&mut on, &title));
                    let show_name = format!("Show column {name}");
                    response.widget_info(|| {
                        egui::WidgetInfo::selected(egui::Role::CheckBox, can, on, show_name.clone())
                    });
                    if response.changed() {
                        action = Some(ControlAction::Column(
                            at.clone(),
                            name.clone(),
                            ColumnEdit::Show(on),
                        ));
                    }
                    ui.horizontal(|ui| {
                        let up = format!("Move column {name} up");
                        if icons::button(ui, i > 0, icons::ARROW_UP, "^", &up).clicked() {
                            action = Some(ControlAction::Column(
                                at.clone(),
                                name.clone(),
                                ColumnEdit::Step(true),
                            ));
                        }
                        let down = format!("Move column {name} down");
                        if icons::button(ui, i < last, icons::ARROW_DOWN, "v", &down).clicked() {
                            action = Some(ControlAction::Column(
                                at.clone(),
                                name.clone(),
                                ColumnEdit::Step(false),
                            ));
                        }
                    });
                    let open = ui.button("Properties");
                    let open_name = format!("Properties of column {name}");
                    open.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::Role::Button, true, open_name.clone())
                    });
                    if open.clicked() {
                        action = Some(ControlAction::SelectColumn(at.clone(), name.clone()));
                    }
                    ui.end_row();
                }
            });
        ui.weak("A click on the title of a column on the form also selects it.");
        action
    }

    /// The property page of a column of a table.
    fn column_properties(
        &mut self,
        ui: &mut egui::Ui,
        at: &Address,
        control: &Control,
        column: ColumnDesign,
    ) {
        let mut edited = column.clone();
        let row_number = column.is_row_number();
        ui.strong(crate::form::column_label(&column.name, control));
        if ui.button("Back to the table").clicked() {
            self.select_control(Some(at.clone()));
            return;
        }
        let key = |property: &str| format!("{at:?}:{}:{property}", column.name);
        let mut merge = None;
        if option_text(ui, "Label", &mut edited.label, &column.name, false) {
            merge = Some(key("label"));
        }
        if option_text(ui, "Help", &mut edited.help, "", true) {
            merge = Some(key("help"));
        }
        // As a control's width: automatic, or ticked and then a number of
        // points. (A number box that started at 0 for automatic swallowed the
        // first 24 points of a drag under the narrowest width.) An id of its
        // own, since the first width makes the column a designed one, which
        // changes the page above, and a drag must not be lost then.
        ui.push_id(("column_width", at, &column.name), |ui| {
            ui.horizontal(|ui| {
                let mut fixed = edited.width.is_some();
                let on = fixed;
                let response = ui.checkbox(&mut fixed, "Width");
                response.widget_info(|| {
                    egui::WidgetInfo::selected(
                        egui::Role::CheckBox,
                        true,
                        on,
                        "Fixed width of the column",
                    )
                });
                if fixed != on {
                    edited.width = fixed.then_some(DEFAULT_COLUMN_WIDTH);
                }
                match edited.width.as_mut() {
                    Some(width) => {
                        let response = ui.add(
                            egui::DragValue::new(width)
                                .range(design::COLUMN_WIDTHS)
                                .speed(1.0)
                                .suffix(" pt"),
                        );
                        response.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::Role::SpinButton,
                                true,
                                "Width of the column",
                            )
                        });
                        if response.changed() {
                            merge = Some(key("width"));
                        }
                    }
                    None => {
                        ui.weak("Automatic: as wide as its widest text");
                    }
                }
            })
        });
        alignment_choice(ui, &mut edited.title_align, "Title alignment", "the title");
        alignment_choice(ui, &mut edited.align, "Cell alignment", "the cells");
        if row_number {
            row_numbering(ui, &mut edited);
        }
        let kind = (!row_number)
            .then(|| {
                let spec = self.calc().spec(&self.sheet)?;
                let design = self.design_of(&self.form_name());
                let columns = crate::form::columns_of(spec, &design, at)?;
                let ty = columns.iter().find(|c| c.name == column.name)?.ty;
                Some((spec.get(ty).kind.clone(), !spec.get(ty).choices.is_empty()))
            })
            .flatten();
        let (kind, has_named) = match kind {
            Some((kind, has_named)) => (Some(kind), has_named),
            None => (None, false),
        };
        if !row_number {
            ui.horizontal(|ui| {
                ui.label("Editor");
                // A cell has one line: several lines count as one.
                let mut editor = match edited.editor.unwrap_or(Editor::Auto) {
                    Editor::Multiline => Editor::Text,
                    e => e,
                };
                let response = egui::ComboBox::from_id_salt("property_column_editor")
                    .selected_text(editor.label())
                    .show_ui(ui, |ui| {
                        for e in Editor::ALL.into_iter().filter(|e| *e != Editor::Multiline) {
                            ui.selectable_value(&mut editor, e, e.label());
                        }
                    })
                    .response;
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::Role::ComboBox, true, "Editor of the column")
                });
                if Some(editor) != edited.editor.or(Some(Editor::Auto)) {
                    edited.editor = (editor != Editor::Auto).then_some(editor);
                }
            });
            ui.checkbox(&mut edited.read_only, "Read only");
        }
        if let Some(kind) = &kind
            && format_properties(
                ui,
                kind,
                &mut edited.thousands,
                &mut edited.decimals,
                &mut edited.date_format,
                &mut edited.percent,
                &mut edited.prefix,
                &mut edited.suffix,
                "column",
            )
        {
            merge = Some(key("date_format"));
        }
        font_properties(
            ui,
            &mut edited.font,
            &mut edited.font_size,
            "Cell ",
            "cells",
        );
        emphasis_properties(ui, &mut edited.bold, &mut edited.italic, "cells");
        // A column that shows an expression: in a table over an expression
        // every column, in a table bound to an array one that is not a field
        // of the items.
        let computed = !row_number
            && (control.lists_items() || {
                let design = self.design_of(&self.form_name());
                self.calc()
                    .spec(&self.sheet)
                    .and_then(|spec| crate::form::columns_of(spec, &design, at))
                    .is_some_and(|columns| !columns.iter().any(|c| c.name == column.name))
            });
        if computed {
            let fields = [
                (ExprKind::Value, &mut edited.value),
                (ExprKind::Visible, &mut edited.visible_when),
                (ExprKind::TextColour, &mut edited.text_colour),
                (ExprKind::FillColour, &mut edited.fill_colour),
            ];
            for (expr, value) in fields {
                if expression_box(ui, expr, value, draft_id(at, Some(&column.name), expr)) {
                    merge = Some(key(expr_property(expr)));
                }
                self.expression_error(ui, at, Some(&column.name), expr);
            }
            let names = self.calc().procedure_names();
            procedure_choice(
                ui,
                "Write procedure",
                "Write procedure of the column",
                &mut edited.write_procedure,
                &names,
            );
            ui.weak(
                "Value runs for each row, with the item as this (and by the item name, in a \
                 table over an expression); without one the cell is the field of the item of \
                 this name. Without a write procedure the cells are read-only; with one, what \
                 is typed into a cell is passed to it as name(form, ctx, value), with \
                 ctx.index the row.",
            );
        } else if !row_number {
            let fields = [
                (ExprKind::Visible, &mut edited.visible_when),
                (ExprKind::Enabled, &mut edited.enabled_when),
                (ExprKind::TextColour, &mut edited.text_colour),
                (ExprKind::FillColour, &mut edited.fill_colour),
            ];
            for (expr, value) in fields {
                if expression_box(ui, expr, value, draft_id(at, Some(&column.name), expr)) {
                    merge = Some(key(expr_property(expr)));
                }
                self.expression_error(ui, at, Some(&column.name), expr);
            }
            let expr = ExprKind::Choices;
            if expr_text(
                ui,
                expr_label(expr),
                &mut edited.choices,
                expr_hint(expr),
                draft_id(at, Some(&column.name), expr),
            ) {
                merge = Some(key(expr_property(expr)));
            }
            self.expression_error(ui, at, Some(&column.name), expr);
            self.choices_note(ui, at, Some(&column.name), kind.as_ref());
            if edited.choices.is_some() || has_named {
                ui.checkbox(&mut edited.allow_other, ALLOW_OTHER)
                    .on_hover_text(ALLOW_OTHER_HELP);
            }
            ui.weak(
                "Visible when runs once, where the table is bound, and hides the whole \
                 column. The others run for each row, with this as the cell and parent \
                 as the row.",
            );
        }
        ui.weak("Where the column is, and whether it is shown, are properties of the table.");
        if edited != column {
            self.set_column(merge, at, edited);
        }
    }

    /// Under the field of an expression of the control at `at`, or of its
    /// column `column`: why it failed, the first line of the message, with
    /// the full message on hover. For one that runs for each row, the first
    /// row that failed. Nothing before it has run, when it worked, or in a
    /// workbook that is not trusted.
    pub(crate) fn expression_error(
        &self,
        ui: &mut egui::Ui,
        at: &Address,
        column: Option<&str>,
        kind: ExprKind,
    ) {
        if ui.data(|d| d.get_temp::<String>(draft_id(at, column, kind)).is_some()) {
            return;
        }
        let calc = self.calc();
        let form = self.form_name();
        let sheet = &self.sheet;
        // In the design of the items of repeaters, and in the rows of a
        // table, it runs for each: the first that failed.
        let Some((rows, message)) = calc.first_expression_error(sheet, &form, at, column, kind)
        else {
            return;
        };
        let first = message.lines().next().unwrap_or_default();
        let place: String = rows.iter().map(|i| format!("[{i}]")).collect();
        let text = match (rows.is_empty(), column.is_some() && kind.per_row()) {
            (true, _) => first.to_owned(),
            (false, true) => format!("Row {place}: {first}"),
            (false, false) => format!("Item {place}: {first}"),
        };
        let color = error_color(ui);
        let response = ui.add(egui::Label::new(
            egui::RichText::new(text).small().color(color),
        ));
        response.on_hover_text(message);
    }

    /// Under the *choices* of the control at `at`, or of its column `column`
    /// (its first row): items that are not of a shape it takes, and on a node
    /// with an enumeration, how many items are left out because their code is
    /// not one of its values.
    fn choices_note(
        &self,
        ui: &mut egui::Ui,
        at: &Address,
        column: Option<&str>,
        kind: Option<&Kind>,
    ) {
        let calc = self.calc();
        let form = self.form_name();
        let state = match column {
            None => calc
                .control_expression(&self.sheet, &form, at, ExprKind::Choices)
                .or_else(|| {
                    calc.control_expression_in(&self.sheet, &form, at, Some(0), ExprKind::Choices)
                }),
            Some(name) => {
                calc.column_expression(&self.sheet, &form, at, name, Some(0), ExprKind::Choices)
            }
        };
        let Some(Ok(value)) = state.and_then(|s| s.result) else {
            return;
        };
        match crate::form_exprs::choice_items(&value) {
            Err(error) => {
                ui.label(egui::RichText::new(error).small().color(error_color(ui)));
            }
            Ok(items) => {
                if let Some(Kind::Enum { values, .. }) = kind {
                    let dropped = items
                        .iter()
                        .filter(|item| !values.contains(&item.code))
                        .count();
                    if dropped > 0 {
                        ui.label(
                            egui::RichText::new(match dropped {
                                1 => "1 item is not a value of the enumeration and is left out"
                                    .to_owned(),
                                n => format!(
                                    "{n} items are not values of the enumeration and are left out"
                                ),
                            })
                            .small()
                            .weak(),
                        );
                    }
                }
            }
        }
    }

    /// On the page of a repeater: designing its items, or going back to the
    /// generated look. Designing shows the repeater stacked and starts from
    /// the controls the generated form has for an item, which can then be
    /// arranged as on the rest of the form.
    fn item_design_buttons(
        &self,
        ui: &mut egui::Ui,
        design: &FormDesign,
        at: &Address,
        edited: &mut Control,
        spec: Option<&cliotree_core::TypeSpec>,
    ) {
        // A repeater in an item shows its items as generated.
        if design.item_scope(at).is_some() {
            return;
        }
        let item_ty = spec.and_then(|spec| {
            let resolved = spec.resolve(&edited.bound_path()?)?;
            match &spec.get(resolved.ty).kind {
                Kind::Array { items, .. } => Some((spec, *items)),
                _ => None,
            }
        });
        let Some((spec, item_ty)) = item_ty else {
            return;
        };
        if edited.children.is_empty() {
            let design_them = ui
                .button("Design the items")
                .on_hover_text(
                    "Show each item with controls you arrange, as on the rest of the form,                      one item at a time",
                )
                .clicked();
            if design_them {
                edited.mode = Some(RepeaterMode::Stacked);
                edited.children = design::item_controls(spec, item_ty);
            }
            return;
        }
        ui.horizontal_wrapped(|ui| {
            ui.label("The items are designed.");
            if ui
                .button("Use the generated items")
                .on_hover_text("Take away the design of the items: they are drawn from the schema")
                .clicked()
            {
                edited.children.clear();
            }
        });
        if !edited.is_stacked() {
            ui.colored_label(
                error_color(ui),
                "The design of the items is used only when they are shown stacked.",
            );
        }
    }

    /// The formula of the node a field is bound to, edited here, since a form
    /// has no formula bar. It is the same entry as in the formula bar of the
    /// data tree, an undoable edit of the tab.
    fn formula_editor(
        &mut self,
        ui: &mut egui::Ui,
        design: &FormDesign,
        at: &Address,
        control: &Control,
    ) {
        if design.item_scope(at).is_some() {
            ui.weak(
                "A formula for this field in every item is a column formula: \
                 set it in the data tree, on the array.",
            );
            return;
        }
        let Some(path) = control.bound_path().filter(|_| control.binds()) else {
            return;
        };
        let sheet = self.sheet.clone();
        let current = self.calc().formula_text(&sheet, &path).unwrap_or_default();
        let draft = self
            .form
            .layout
            .as_ref()
            .and_then(|l| l.formula.as_ref())
            .filter(|(p, _)| p == &path)
            .map(|(_, t)| t.clone());
        let mut text = draft.clone().unwrap_or_else(|| current.clone());
        ui.horizontal(|ui| {
            ui.label("Formula =");
            let response = ui.add(
                egui::TextEdit::singleline(&mut text)
                    .hint_text("no formula")
                    .desired_width(f32::INFINITY),
            );
            named(&response, "Formula of the bound node");
            if response.changed()
                && let Some(layout) = self.form.layout.as_mut()
            {
                layout.formula = Some((path.clone(), text.clone()));
            }
            if response.lost_focus() && text != current {
                self.commit_formula(&path, &text);
            }
        });
        if let Some(notice) = &self.notice {
            let color = error_color(ui);
            ui.colored_label(color, notice);
        }
    }

    /// Enters the formula of a node, or removes it when the text is empty.
    pub fn commit_formula(&mut self, path: &Path, text: &str) {
        let sheet = self.sheet.clone();
        let formula = format!("={}", text.trim());
        if !text.trim().is_empty()
            && self.ask_before_erasing(
                &sheet,
                crate::replace_inside::Target::Node(path.clone()),
                &formula,
                true,
            )
        {
            return;
        }
        let result = if text.trim().is_empty() {
            self.session
                .calc_mut()
                .remove_formula(&sheet, path)
                .map(|_| ())
                .map_err(|e| e.to_string())
        } else {
            entry::enter(self.session.calc_mut(), &sheet, path, &formula, false)
                .map_err(|e| e.to_string())
        };
        match result {
            Ok(()) => {
                self.notice = None;
                self.clear_form_formula();
                self.session.tick();
            }
            Err(message) => self.notice = Some(message),
        }
    }

    /// Lets the formula box of the form designer show the node's formula
    /// again, instead of what was typed.
    pub(crate) fn clear_form_formula(&mut self) {
        if let Some(layout) = self.form.layout.as_mut() {
            layout.formula = None;
        }
    }

    fn unplaced_list(&mut self, ui: &mut egui::Ui, design: &FormDesign) {
        let Some(spec) = self.calc().spec(&self.sheet) else {
            return;
        };
        let unplaced = design.unplaced(spec);
        egui::CollapsingHeader::new(format!("Fields not on the form ({})", unplaced.len()))
            .id_salt("unplaced")
            .default_open(true)
            .show(ui, |ui| {
                if unplaced.is_empty() {
                    ui.weak("Every field is on the form.");
                }
                for path in &unplaced {
                    ui.horizontal(|ui| {
                        ui.label(path.to_string());
                        let response = ui.button("Add");
                        response.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::Role::Button,
                                true,
                                format!("Add {path} to the form"),
                            )
                        });
                        if response.clicked() {
                            self.add_field_to_form(path);
                        }
                    });
                }
            });
    }

    fn broken_list(&mut self, ui: &mut egui::Ui, design: &FormDesign) {
        let Some(spec) = self.calc().spec(&self.sheet) else {
            return;
        };
        let broken = design.broken(spec);
        egui::CollapsingHeader::new(format!("Broken controls ({})", broken.len()))
            .id_salt("broken")
            .default_open(true)
            .show(ui, |ui| {
                if broken.is_empty() {
                    ui.weak("No control is broken.");
                }
                for (at, problem) in &broken {
                    let name = design.get(at).map(control_name).unwrap_or_default();
                    ui.label(format!("{name}: {problem}"));
                    ui.horizontal(|ui| {
                        let rebind = ui.button("Re-bind");
                        rebind.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::Role::Button,
                                true,
                                format!("Re-bind {name}"),
                            )
                        });
                        if rebind.clicked() {
                            self.select_control(Some(at.clone()));
                        }
                        let remove = ui.button("Remove");
                        remove.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::Role::Button,
                                true,
                                format!("Remove {name}"),
                            )
                        });
                        if remove.clicked() {
                            self.apply_control(ControlAction::Delete(at.clone()));
                        }
                    });
                }
            });
    }
}

/// How many controls are in the list `at` is in.
fn sibling_count(design: &FormDesign, at: &[usize]) -> usize {
    match at.split_last() {
        Some((_, parent)) if !parent.is_empty() => {
            design.get(parent).map_or(0, |c| c.children.len())
        }
        Some(_) => design.controls.len(),
        None => 0,
    }
}

/// The sizes the property page offers, in points.
/// The property that lets a field with choices take a code not in the list.
/// The width a column is given when its width is ticked, in points.
const DEFAULT_COLUMN_WIDTH: u32 = 80;

const ALLOW_OTHER: &str = "Allow other values (write-in codes)";
const ALLOW_OTHER_HELP: &str = "A dropdown offers Other... to type a code that is not listed; a typeahead enters typed text that matches no item as it is";

const FONT_SIZE_CHOICES: [u32; 16] = [8, 9, 10, 11, 12, 13, 14, 16, 18, 20, 24, 28, 32, 36, 48, 72];

/// The font of a control or a column: a typeface and a size, each *Inherited*
/// (from the group or table it is in, or the interface font) unless chosen.
/// `what` names the two for a screen reader: "Font of the control".
fn font_properties(
    ui: &mut egui::Ui,
    font: &mut Option<cliotree_core::form::FontFamily>,
    font_size: &mut Option<u32>,
    prefix: &str,
    what: &str,
) {
    use crate::style::{Typeface, family, typeface_label};
    let (font_label, size_label) = if prefix.is_empty() {
        ("Font".to_owned(), "Size".to_owned())
    } else {
        (format!("{prefix}font"), format!("{prefix}size"))
    };
    ui.horizontal(|ui| {
        ui.label(font_label);
        let mut chosen = *font;
        let response = egui::ComboBox::from_id_salt(("property_font", what))
            .selected_text(chosen.map_or_else(|| "Inherited".to_owned(), typeface_label))
            .width(200.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut chosen, None, "Inherited");
                for typeface in Typeface::ALL {
                    let text =
                        egui::RichText::new(typeface_label(typeface)).family(family(typeface));
                    ui.selectable_value(&mut chosen, Some(typeface), text);
                }
            })
            .response;
        let name = format!("Font of the {what}");
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, &name));
        *font = chosen;
    });
    ui.horizontal(|ui| {
        ui.label(size_label);
        let mut size = *font_size;
        let response = egui::ComboBox::from_id_salt(("property_font_size", what))
            .selected_text(size.map_or_else(|| "Inherited".to_owned(), |s| format!("{s} pt")))
            // Tall enough for every size, with no scrolling.
            .height(480.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut size, None, "Inherited");
                for choice in FONT_SIZE_CHOICES {
                    ui.selectable_value(&mut size, Some(choice), format!("{choice} pt"));
                }
            })
            .response;
        let name = format!("Font size of the {what}");
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, &name));
        *font_size = size;
    });
}

/// Bold and italic, each inherited, on or off. `what` names them for a
/// screen reader: "Bold of the control".
fn emphasis_properties(
    ui: &mut egui::Ui,
    bold: &mut Option<bool>,
    italic: &mut Option<bool>,
    what: &str,
) {
    ui.horizontal(|ui| {
        for (label, value) in [("Bold", bold), ("Italic", italic)] {
            ui.label(label);
            let mut chosen = *value;
            let text = |v: Option<bool>| match v {
                None => "Inherited",
                Some(true) => "Yes",
                Some(false) => "No",
            };
            let response = egui::ComboBox::from_id_salt(("property_emphasis", label, what))
                .width(84.0)
                .selected_text(text(chosen))
                .show_ui(ui, |ui| {
                    for v in [None, Some(true), Some(false)] {
                        ui.selectable_value(&mut chosen, v, text(v));
                    }
                })
                .response;
            let name = format!("{label} of the {what}");
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, &name));
            *value = chosen;
        }
    });
}

/// A group's border: its width in points (automatic is the usual look, 0
/// none) and its fixed colour. Returns whether either changed.
fn border_properties(ui: &mut egui::Ui, edited: &mut Control) -> bool {
    let before = (edited.border_width, edited.border_colour.clone());
    ui.horizontal(|ui| {
        ui.label("Border");
        let mut width = edited.border_width;
        let response = egui::ComboBox::from_id_salt("property_border_width")
            .width(84.0)
            .selected_text(width.map_or_else(|| "Automatic".to_owned(), |w| format!("{w} pt")))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut width, None, "Automatic");
                for w in 0..=8 {
                    ui.selectable_value(&mut width, Some(w), format!("{w} pt"));
                }
            })
            .response;
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::ComboBox, true, "Border width of the group")
        });
        edited.border_width = width;
        let mut own = edited.border_colour.is_some();
        let response = ui.checkbox(&mut own, "Colour");
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::Role::CheckBox,
                true,
                own,
                "Border colour of the group",
            )
        });
        if own != edited.border_colour.is_some() {
            edited.border_colour = own.then(|| "#808080".to_owned());
        }
        if let Some(colour) = &mut edited.border_colour {
            let mut rgb = cliotree_core::form::fixed_colour(colour).unwrap_or([128, 128, 128]);
            if ui.color_edit_button_srgb(&mut rgb).changed() {
                *colour = hex_colour(rgb);
            }
        }
    });
    before != (edited.border_width, edited.border_colour.clone())
}

/// Where the text of a column's title or cells sits: automatic, left, centre
/// or right. `what` names it for a screen reader: "Alignment of the title".
/// The alignment of a control in its row or cell: left unless it says.
fn control_alignment(ui: &mut egui::Ui, edited: &mut Control) {
    ui.horizontal(|ui| {
        ui.label("Alignment");
        let mut align = edited.align.unwrap_or(Alignment::Left);
        let response = egui::ComboBox::from_id_salt("property_control_align")
            .selected_text(align.label())
            .show_ui(ui, |ui| {
                for a in Alignment::ALL {
                    ui.selectable_value(&mut align, a, a.label());
                }
            })
            .response;
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::ComboBox, true, "Alignment in its cell")
        });
        edited.align = (align != Alignment::Left).then_some(align);
    });
}

fn alignment_choice(ui: &mut egui::Ui, align: &mut Option<Alignment>, label: &str, what: &str) {
    ui.horizontal(|ui| {
        ui.label(label);
        let mut chosen = *align;
        let response = egui::ComboBox::from_id_salt(("property_align", what))
            .selected_text(chosen.map_or("Automatic", Alignment::label))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut chosen, None, "Automatic");
                for a in Alignment::ALL {
                    ui.selectable_value(&mut chosen, Some(a), a.label());
                }
            })
            .response;
        let name = format!("Alignment of {what}");
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, &name));
        *align = chosen;
    });
}

/// How the row numbers are written, and the number of the first row. A start
/// that is the numbering's own is stored as nothing.
fn row_numbering(ui: &mut egui::Ui, edited: &mut ColumnDesign) {
    use cliotree_core::form::Numbering;
    let mut numbering = edited.numbering.unwrap_or(Numbering::Index);
    ui.horizontal(|ui| {
        ui.label("Numbering");
        let response = egui::ComboBox::from_id_salt("property_numbering")
            .selected_text(numbering.label())
            .show_ui(ui, |ui| {
                for n in Numbering::ALL {
                    ui.selectable_value(&mut numbering, n, n.label());
                }
            })
            .response;
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::ComboBox, true, "Numbering of the rows")
        });
    });
    let mut start = edited
        .start
        .unwrap_or_else(|| edited.numbering.unwrap_or(Numbering::Index).default_start());
    ui.horizontal(|ui| {
        ui.label("Start at");
        let response = ui.add(egui::DragValue::new(&mut start).speed(0.1));
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::SpinButton, true, "Number of the first row")
        });
    });
    // A start that was not given follows the numbering; one that was is kept.
    let start = if edited.start.is_none()
        && start == edited.numbering.unwrap_or(Numbering::Index).default_start()
    {
        numbering.default_start()
    } else {
        start
    };
    edited.numbering = (numbering != Numbering::Index).then_some(numbering);
    edited.start = (start != numbering.default_start()).then_some(start);
    let first: Vec<String> = (0..3).map(|i| edited.row_label(i)).collect();
    ui.weak(format!("The rows are numbered {}, ...", first.join(", ")));
}

/// The format of a number (percent, a prefix and suffix, a thousands
/// separator, decimal places) or of a
/// date (a pattern), for a field or column of that type; nothing for another
/// type. Returns whether the pattern was typed into.
#[allow(clippy::too_many_arguments)]
fn format_properties(
    ui: &mut egui::Ui,
    kind: &Kind,
    thousands: &mut bool,
    decimals: &mut Option<u32>,
    date_format: &mut Option<String>,
    percent: &mut bool,
    prefix: &mut Option<String>,
    suffix: &mut Option<String>,
    what: &str,
) -> bool {
    use cliotree_core::format::{DATE_PRESETS, MAX_DECIMALS, date, is_date, is_number};
    if is_number(kind) {
        let response = ui.checkbox(percent, "Percent");
        let name = format!("Percent of the {what}");
        let on = *percent;
        response.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, on, &name));
        response.on_hover_text("The value times 100, with %: 0.25 is shown as 25%");
        let typed = option_text(ui, "Prefix", prefix, "such as € ", false)
            | option_text(ui, "Suffix", suffix, "such as  kg", false);
        if typed {
            // As the schema allows.
            for text in [&mut *prefix, &mut *suffix].into_iter().flatten() {
                if text.chars().count() > 20 {
                    *text = text.chars().take(20).collect();
                }
            }
        }
        let response = ui.checkbox(thousands, "Thousands separator");
        let name = format!("Thousands separator of the {what}");
        let on = *thousands;
        response.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, on, &name));
        ui.horizontal(|ui| {
            ui.label("Decimal places");
            let mut chosen = *decimals;
            let response = egui::ComboBox::from_id_salt("property_decimals")
                .selected_text(chosen.map_or_else(|| "Automatic".to_owned(), |d| d.to_string()))
                .height(400.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut chosen, None, "Automatic");
                    for d in 0..=MAX_DECIMALS {
                        ui.selectable_value(&mut chosen, Some(d), d.to_string());
                    }
                })
                .response;
            let name = format!("Decimal places of the {what}");
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, &name));
            *decimals = chosen;
        });
        typed
    } else if is_date(kind) {
        let sample = chrono::NaiveDate::from_ymd_opt(2026, 3, 7).expect("a valid date");
        ui.horizontal(|ui| {
            ui.label("Date format");
            let mut chosen = date_format.clone();
            let text = chosen.as_deref().map_or_else(
                || "Automatic (yyyy-mm-dd)".to_owned(),
                |p| format!("{p} ({})", date(sample, p)),
            );
            let response = egui::ComboBox::from_id_salt("property_date_format")
                .selected_text(text)
                .width(200.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut chosen, None, "Automatic (yyyy-mm-dd)");
                    for preset in DATE_PRESETS {
                        let label = format!("{preset} ({})", date(sample, preset));
                        ui.selectable_value(&mut chosen, Some(preset.to_owned()), label);
                    }
                })
                .response;
            let name = format!("Date format of the {what}");
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, &name));
            *date_format = chosen;
        });
        let typed = option_text(ui, "Pattern", date_format, "yyyy-mm-dd", false);
        ui.weak("d dd ddd dddd: the day; m mm mmm mmmm: the month; yy yyyy: the year.");
        typed
    } else {
        false
    }
}

/// Where the text of an expression box is kept while it is typed: the box of
/// that expression of the control at `at`, or of its column `column`.
pub(crate) fn draft_id(at: &[usize], column: Option<&str>, kind: ExprKind) -> egui::Id {
    egui::Id::unique((
        "expression_draft",
        at.to_vec(),
        column.map(str::to_owned),
        kind,
    ))
}

/// A box for an expression on the property page. What is typed is kept
/// aside, and is neither stored nor run, until the box loses the focus (a
/// click elsewhere, Tab, or Enter): a half-typed expression is no error.
/// Returns whether the stored expression changed.
pub(crate) fn expr_text(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut Option<String>,
    hint: &str,
    draft: egui::Id,
) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(label);
        changed = expr_box(ui, label, value, hint, draft);
    });
    changed
}

/// The box of [`expr_text`], without its label; `name` is what a screen
/// reader calls it.
fn expr_box(
    ui: &mut egui::Ui,
    name: &str,
    value: &mut Option<String>,
    hint: &str,
    draft: egui::Id,
) -> bool {
    let kept: Option<String> = ui.data(|d| d.get_temp(draft));
    let mut text = kept
        .clone()
        .unwrap_or_else(|| value.clone().unwrap_or_default());
    let response = ui.add(
        egui::TextEdit::singleline(&mut text)
            .id(draft.with("box"))
            .hint_text(hint)
            .desired_width(f32::INFINITY),
    );
    let name = name.to_owned();
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, &name));
    if response.lost_focus() {
        ui.data_mut(|d| d.remove::<String>(draft));
        let typed = (!text.trim().is_empty()).then_some(text);
        if typed != *value {
            *value = typed;
            return true;
        }
    } else if response.has_focus() {
        if response.changed() || kept.is_some() {
            ui.data_mut(|d| d.insert_temp(draft, text));
        }
    } else if kept.is_some() {
        // Typed into, then left in a way the box did not see (another
        // control was selected): what was typed is dropped.
        ui.data_mut(|d| d.remove::<String>(draft));
    }
    false
}

/// The box of an expression of a control or column: a colour, or another.
pub(crate) fn expression_box(
    ui: &mut egui::Ui,
    kind: ExprKind,
    value: &mut Option<String>,
    draft: egui::Id,
) -> bool {
    match kind {
        ExprKind::TextColour | ExprKind::FillColour => colour_text(ui, kind, value, draft),
        _ => expr_text(ui, expr_label(kind), value, expr_hint(kind), draft),
    }
}

/// A colour on the property page: an expression (empty for the usual look),
/// or a fixed colour, picked, which runs no Python and so is shown in an
/// untrusted workbook too. Returns whether it changed.
fn colour_text(
    ui: &mut egui::Ui,
    kind: ExprKind,
    value: &mut Option<String>,
    draft: egui::Id,
) -> bool {
    let label = expr_label(kind);
    let first = if kind == ExprKind::TextColour {
        [0x00, 0x00, 0x00]
    } else {
        [0xff, 0xf2, 0xcc]
    };
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(label);
        let was_fixed = value
            .as_deref()
            .and_then(cliotree_core::form::fixed_colour)
            .is_some();
        let mut fixed = was_fixed;
        let response = egui::ComboBox::from_id_salt(draft.with("kind"))
            .width(96.0)
            .selected_text(if fixed { "Fixed" } else { "Expression" })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut fixed, false, "Expression");
                ui.selectable_value(&mut fixed, true, "Fixed");
            })
            .response;
        let name = format!("Kind of {}", label.to_lowercase());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, &name));
        if fixed != was_fixed {
            *value = fixed.then(|| hex_colour(first));
            ui.data_mut(|d| d.remove::<String>(draft));
            changed = true;
        }
        if fixed {
            let mut rgb = value
                .as_deref()
                .and_then(cliotree_core::form::fixed_colour)
                .unwrap_or(first);
            let response = ui.color_edit_button_srgb(&mut rgb);
            let name = label.to_owned();
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &name));
            if response.changed() {
                *value = Some(hex_colour(rgb));
                changed = true;
            }
            ui.weak(value.as_deref().unwrap_or_default());
        } else {
            changed |= expr_box(ui, label, value, expr_hint(kind), draft);
        }
    });
    changed
}

fn hex_colour([r, g, b]: [u8; 3]) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// Whether a name can be an item or list name: a Python identifier.
pub(crate) fn is_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Adds a column that shows an expression to a table over an expression,
/// keeping the columns it shows.
fn add_listed_column(names: Vec<String>, edited: &mut Control) {
    let mut columns = edited.arrange_columns(&names);
    let mut n = 1;
    let name = loop {
        let name = format!("column{n}");
        if !columns.iter().any(|c| c.name == name) {
            break name;
        }
        n += 1;
    };
    let mut column = ColumnDesign::named(&name);
    column.value = Some("this".to_owned());
    columns.push(column);
    edited.table_columns = columns;
    edited.show = None;
}

/// The name of an expression's field on the property page.
fn expr_label(kind: ExprKind) -> &'static str {
    match kind {
        ExprKind::Visible => "Visible when",
        ExprKind::Enabled => "Enabled when",
        ExprKind::TextColour => "Text colour",
        ExprKind::FillColour => "Fill colour",
        ExprKind::Choices => "Choices",
        ExprKind::Value => "Value",
        ExprKind::Template => "Text",
        ExprKind::Items => "Items",
    }
}

fn expr_hint(kind: ExprKind) -> &'static str {
    match kind {
        ExprKind::Visible | ExprKind::Enabled => "expression",
        ExprKind::TextColour | ExprKind::FillColour => "expression giving a colour",
        ExprKind::Choices => "expression giving a list",
        ExprKind::Value => "expression",
        ExprKind::Template => "text with {expressions}",
        ExprKind::Items => "expression giving a list",
    }
}

/// The property an expression is stored in, for merging the typing in its
/// field into one change of the design.
fn expr_property(kind: ExprKind) -> &'static str {
    match kind {
        ExprKind::Visible => "visible_when",
        ExprKind::Enabled => "enabled_when",
        ExprKind::TextColour => "text_colour",
        ExprKind::FillColour => "fill_colour",
        ExprKind::Choices => "choices",
        ExprKind::Value => "value",
        ExprKind::Template => "text",
        ExprKind::Items => "items",
    }
}

/// The procedure of a button, chosen from the functions of the modules. A
/// name that no module has any more stays chosen, marked, until another is
/// chosen, so that fixing the module brings the button back.
fn procedure_choice(
    ui: &mut egui::Ui,
    label: &str,
    name: &str,
    procedure: &mut Option<String>,
    names: &[String],
) {
    let missing = procedure
        .as_ref()
        .filter(|name| {
            // `module.name`, written by hand, runs too.
            let function = name.rsplit_once('.').map_or(name.as_str(), |(_, f)| f);
            !names.iter().any(|n| n == function)
        })
        .cloned();
    let shown = match (&*procedure, &missing) {
        (_, Some(name)) => format!("{name} (not in any module)"),
        (Some(name), None) => name.clone(),
        (None, _) => "(none)".to_owned(),
    };
    ui.horizontal(|ui| {
        ui.label(label);
        let response = egui::ComboBox::from_id_salt(("property_procedure", label))
            .selected_text(shown)
            .width(200.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(procedure, None, "(none)");
                if let Some(name) = &missing {
                    ui.selectable_value(
                        procedure,
                        Some(name.clone()),
                        format!("{name} (not in any module)"),
                    );
                }
                for name in names {
                    ui.selectable_value(procedure, Some(name.clone()), name);
                }
            })
            .response;
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, name));
    });
}

/// A labelled text field for an optional property: empty is absent. Returns
/// whether it was typed into.
pub(crate) fn option_text(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut Option<String>,
    hint: &str,
    multiline: bool,
) -> bool {
    let mut text = value.clone().unwrap_or_default();
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(label);
        let edit = if multiline {
            egui::TextEdit::multiline(&mut text).desired_rows(2)
        } else {
            egui::TextEdit::singleline(&mut text)
        };
        let response = ui.add(edit.hint_text(hint).desired_width(f32::INFINITY));
        let name = label.to_owned();
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, &name));
        changed = response.changed();
    });
    if changed {
        *value = (!text.is_empty()).then_some(text);
    }
    changed
}

/// The size and place of a control on its property page: width (automatic,
/// points, or the whole width), height, alignment in its cell, and as its
/// kind and its group allow, the label of a field, the columns it spans and
/// a new row, and a group's label width, spacing and padding. Returns whether
/// a number was changed, so that dragging one is one change.
fn size_properties(ui: &mut egui::Ui, edited: &mut Control, parent_columns: u32) -> bool {
    let before = edited.clone();
    let kind = edited.kind;
    ui.add_space(4.0);
    ui.strong("Size and place");
    ui.horizontal(|ui| {
        let mut fill = edited.fill;
        ui.checkbox(&mut fill, "Fill the width")
            .on_hover_text("The whole width of its cell or row");
        if fill != edited.fill {
            edited.fill = fill;
            if fill {
                edited.width = None;
            }
        }
    });
    if !edited.fill {
        optional_number(
            ui,
            "Width",
            &mut edited.width,
            &CONTROL_WIDTHS,
            200,
            "Width in points",
        );
    }
    if kind != ControlKind::Separator || edited.vertical {
        optional_number(
            ui,
            "Height",
            &mut edited.height,
            &CONTROL_HEIGHTS,
            100,
            "Height in points",
        );
    }
    // A field's value is aligned automatically, numbers on the right, until
    // it is given an alignment; other controls are on the left.
    if kind == ControlKind::Field {
        alignment_choice(ui, &mut edited.align, "Alignment", "the field");
    } else {
        control_alignment(ui, edited);
    }
    if kind == ControlKind::Field {
        ui.horizontal(|ui| {
            ui.label("Label");
            let mut position = edited.label_position.unwrap_or(LabelPosition::Left);
            let response = egui::ComboBox::from_id_salt("property_label_position")
                .selected_text(position.label())
                .show_ui(ui, |ui| {
                    for p in LabelPosition::ALL {
                        ui.selectable_value(&mut position, p, p.label());
                    }
                })
                .response;
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::ComboBox, true, "Position of the label")
            });
            edited.label_position = (position != LabelPosition::Left).then_some(position);
        });
    }
    if matches!(kind, ControlKind::Field | ControlKind::Group) {
        let name = if kind == ControlKind::Group {
            "Label width of its fields"
        } else {
            "Label width"
        };
        optional_number(
            ui,
            "Label width",
            &mut edited.label_width,
            &LABEL_WIDTHS,
            DEFAULT_LABEL_WIDTH,
            name,
        );
    }
    if parent_columns > 1 {
        ui.horizontal(|ui| {
            ui.label("Columns it takes");
            let mut span = edited.span.unwrap_or(1).min(parent_columns);
            let response = ui.add(egui::DragValue::new(&mut span).range(1..=parent_columns));
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::SpinButton, true, "Columns it takes")
            });
            edited.span = (span > 1).then_some(span);
        });
        ui.checkbox(&mut edited.new_row, "Start a new row");
    }
    if kind == ControlKind::Group {
        optional_number(
            ui,
            "Spacing",
            &mut edited.spacing,
            &SPACINGS,
            8,
            "Spacing in points",
        );
        optional_number(
            ui,
            "Padding",
            &mut edited.padding,
            &SPACINGS,
            6,
            "Padding in points",
        );
    }
    *edited != before
}

/// A number that is automatic unless ticked: a tick box with the label, and
/// beside it the number in points. `name` names the number for a screen
/// reader.
fn optional_number(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut Option<u32>,
    range: &std::ops::RangeInclusive<u32>,
    default: u32,
    name: &str,
) {
    ui.horizontal(|ui| {
        let mut set = value.is_some();
        ui.checkbox(&mut set, label)
            .on_hover_text("Automatic unless ticked");
        match (set, *value) {
            (false, _) => *value = None,
            (true, None) => *value = Some(default.clamp(*range.start(), *range.end())),
            (true, Some(_)) => {}
        }
        if let Some(number) = value.as_mut() {
            let response = ui.add(
                egui::DragValue::new(number)
                    .range(range.clone())
                    .suffix(" pt"),
            );
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::SpinButton, true, name));
        }
    });
}

fn position(points: f32) -> u32 {
    clamped(points, &POSITIONS)
}

fn clamped(points: f32, range: &std::ops::RangeInclusive<u32>) -> u32 {
    (points.round().max(0.0) as u32).clamp(*range.start(), *range.end())
}

fn snap_grid(points: f32) -> f32 {
    (points / GRID).round() * GRID
}

/// Changes the layout of a group. Its controls going into a Free group are
/// put where they were drawn, from the top left of the first of them, so
/// that nothing jumps; the group's size is learnt when it is next drawn.
fn set_layout(group: &mut Control, at: &Address, layout: GroupLayout, rects: &FormRects) {
    let was = group.layout.unwrap_or(GroupLayout::Flow);
    group.layout = (layout != GroupLayout::Flow).then_some(layout);
    group.base_width = None;
    group.base_height = None;
    if layout == GroupLayout::Grid {
        // From a flow, the rows and columns it had; from a Free group, one
        // column in the order of the controls.
        let columns = match was {
            GroupLayout::Flow => group.columns.unwrap_or(1).clamp(1, MAX_TRACKS) as usize,
            _ => 1,
        };
        if group.grid_columns.is_empty() {
            group.grid_columns = vec![TrackSize::Share(1); columns];
        }
        for (row, cells) in flow_rows(&group.children, columns).into_iter().enumerate() {
            for (i, column, span) in cells.cells {
                let c = &mut group.children[i];
                c.grid_row = Some(track_number(row));
                c.grid_column = Some(track_number(column));
                c.span = (span > 1).then_some(span as u32);
            }
        }
        return;
    }
    if layout != GroupLayout::Free {
        return;
    }
    let drawn: Vec<Option<egui::Rect>> = (0..group.children.len())
        .map(|i| {
            let mut child = at.clone();
            child.push(i);
            rects.controls.get(&child).copied()
        })
        .collect();
    let left = drawn
        .iter()
        .flatten()
        .map(|r| r.left())
        .fold(f32::INFINITY, f32::min);
    let top = drawn
        .iter()
        .flatten()
        .map(|r| r.top())
        .fold(f32::INFINITY, f32::min);
    let mut next = 0.0;
    for (c, rect) in group.children.iter_mut().zip(drawn) {
        let (x, y) = match rect {
            Some(r) => (r.left() - left, r.top() - top),
            // Not drawn (hidden, say): under the others.
            None => (0.0, next),
        };
        c.x = Some(position(snap_grid(x)));
        c.y = Some(position(snap_grid(y)));
        next = next.max(y + rect.map_or(24.0, |r| r.height()) + GRID * 2.0);
    }
}

/// The controls of a Free group in their order, which is the tab order, with
/// buttons that move one earlier or later. Returns the move asked for.
fn tab_order(ui: &mut egui::Ui, group: &Control, at: &Address) -> Option<(Address, bool)> {
    ui.add_space(4.0);
    ui.strong("Tab order");
    let mut asked = None;
    let n = group.children.len();
    for (i, child) in group.children.iter().enumerate() {
        ui.horizontal(|ui| {
            let mut child_at = at.clone();
            child_at.push(i);
            let name = control_name(child);
            let up = ui
                .add_enabled(i > 0, egui::Button::new("\u{2191}").small())
                .on_hover_text("Earlier in the tab order");
            up.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::Button, i > 0, format!("Earlier: {name}"))
            });
            let down = ui
                .add_enabled(i + 1 < n, egui::Button::new("\u{2193}").small())
                .on_hover_text("Later in the tab order");
            down.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::Button, i + 1 < n, format!("Later: {name}"))
            });
            ui.label(format!("{}. {name}", i + 1));
            if up.clicked() {
                asked = Some((child_at.clone(), true));
            }
            if down.clicked() {
                asked = Some((child_at, false));
            }
        });
    }
    ui.weak("Moving a control on the form does not change its place in the tab order.");
    asked
}

/// The position of a control in a Free group, as it is seen, and its
/// anchors. Returns the change asked for, with the property it is of (so
/// that dragging a number is one change).
fn free_properties(
    ui: &mut egui::Ui,
    group: &Control,
    control: &Control,
    at: &Address,
    rects: &FormRects,
) -> Option<(Placement, &'static str)> {
    let parent = at.split_last()?.1;
    let size = rects
        .free
        .get(parent)
        .map(|r| (r.width(), r.height()))
        .unwrap_or_else(|| {
            let w = group.base_width.unwrap_or(0) as f32;
            let h = group.base_height.unwrap_or(0) as f32;
            (w, h)
        });
    let place = free_place(group, control, size);
    let mut asked = None;
    ui.add_space(4.0);
    ui.strong("Place in the group");
    ui.horizontal(|ui| {
        let mut x = place.left.round() as u32;
        let mut y = place.top.round() as u32;
        ui.label("X");
        let rx = ui.add(egui::DragValue::new(&mut x).range(POSITIONS).suffix(" pt"));
        rx.widget_info(|| egui::WidgetInfo::labeled(egui::Role::SpinButton, true, "X position"));
        ui.label("Y");
        let ry = ui.add(egui::DragValue::new(&mut y).range(POSITIONS).suffix(" pt"));
        ry.widget_info(|| egui::WidgetInfo::labeled(egui::Role::SpinButton, true, "Y position"));
        if x != place.left.round() as u32 || y != place.top.round() as u32 {
            asked = Some((
                Placement {
                    at: at.clone(),
                    x: Some(x as f32),
                    y: Some(y as f32),
                    size: Some(size),
                    ..Placement::default()
                },
                "position",
            ));
        }
    });
    for across in [true, false] {
        let current = if across {
            control.anchor_x
        } else {
            control.anchor_y
        }
        .unwrap_or(Anchor::Start);
        let mut anchor = current;
        ui.horizontal(|ui| {
            ui.label(if across {
                "Anchored across"
            } else {
                "Anchored down"
            });
            let response = egui::ComboBox::from_id_salt(("property_anchor", across))
                .selected_text(anchor.label(across))
                .show_ui(ui, |ui| {
                    for a in Anchor::ALL {
                        ui.selectable_value(&mut anchor, a, a.label(across));
                    }
                })
                .response;
            let name = if across {
                "Anchor across"
            } else {
                "Anchor down"
            };
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, name));
        });
        if anchor != current {
            let mut p = Placement {
                at: at.clone(),
                size: Some(size),
                ..Placement::default()
            };
            // Stretching needs a size to stretch: the one it is drawn at.
            let drawn = rects.controls.get(at);
            if across {
                p.anchor_x = Some(anchor);
                if anchor == Anchor::Both && control.width.is_none() {
                    p.width = drawn.map(|r| r.width());
                }
            } else {
                p.anchor_y = Some(anchor);
                if anchor == Anchor::Both && control.height.is_none() {
                    p.height = drawn.map(|r| r.height());
                }
            }
            asked = Some((p, "anchor"));
        }
    }
    if group.height.is_none() {
        ui.weak("Anchoring to the bottom needs a height on the group.");
    }
    asked
}

/// A row or column number as it is stored, from 1.
fn track_number(index: usize) -> u32 {
    (index as u32 + 1).min(MAX_TRACKS)
}

/// The columns and rows of a Grid group, each with its size, a button that
/// removes it, and buttons that add one.
fn grid_tracks(ui: &mut egui::Ui, group: &mut Control) {
    let (_, rows, columns) = grid_cells(group);
    for (what, tracks, shown) in [
        ("Column", &mut group.grid_columns, columns),
        ("Row", &mut group.grid_rows, rows),
    ] {
        ui.add_space(4.0);
        ui.strong(format!("{what}s"));
        let mut remove = None;
        for i in 0..tracks.len().max(shown) {
            ui.horizontal(|ui| {
                ui.label(format!("{what} {}", i + 1));
                let mut size = tracks.get(i).copied().unwrap_or(TrackSize::Auto);
                let before = size;
                let name = format!("Size of {} {}", what.to_lowercase(), i + 1);
                let response = egui::ComboBox::from_id_salt(("property_track", what, i))
                    .selected_text(size.label())
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut size, TrackSize::Auto, "Automatic");
                        let points = match size {
                            TrackSize::Points(p) => p,
                            _ => 100,
                        };
                        ui.selectable_value(&mut size, TrackSize::Points(points), "Points");
                        let share = match size {
                            TrackSize::Share(s) => s,
                            _ => 1,
                        };
                        ui.selectable_value(&mut size, TrackSize::Share(share), "Share");
                    })
                    .response;
                response
                    .widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, &name));
                match &mut size {
                    TrackSize::Points(p) => {
                        ui.add(egui::DragValue::new(p).range(TRACK_POINTS).suffix(" pt"));
                    }
                    TrackSize::Share(s) => {
                        ui.add(egui::DragValue::new(s).range(TRACK_SHARES).suffix(" fr"));
                    }
                    TrackSize::Auto => {}
                }
                if size != before {
                    while tracks.len() <= i {
                        tracks.push(TrackSize::Auto);
                    }
                    tracks[i] = size;
                }
                if i < tracks.len() {
                    let label = format!("Remove {} {}", what.to_lowercase(), i + 1);
                    let r = ui.small_button("\u{2715}").on_hover_text(&label);
                    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
                    if r.clicked() {
                        remove = Some(i);
                    }
                }
            });
        }
        if let Some(i) = remove {
            tracks.remove(i);
        }
        let add = format!("Add {}", what.to_lowercase());
        if ui
            .add_enabled(
                (tracks.len() as u32) < MAX_TRACKS,
                egui::Button::new(add.as_str()),
            )
            .clicked()
        {
            while tracks.len() < shown {
                tracks.push(TrackSize::Auto);
            }
            tracks.push(TrackSize::Auto);
        }
    }
    ui.weak(
        "A control placed past the last row or column adds an automatic one. \
         A share divides what the others leave.",
    );
}

/// The cell of a control in a Grid group, as it is placed (a control with
/// none is in the next free cell), and the rows and columns it takes.
fn cell_properties(ui: &mut egui::Ui, group: &Control, edited: &mut Control, at: &Address) {
    let Some(&index) = at.last() else {
        return;
    };
    let (cells, ..) = grid_cells(group);
    let Some(cell) = cells.iter().find(|c| c.index == index) else {
        return;
    };
    ui.add_space(4.0);
    ui.strong("Cell in the grid");
    let max = MAX_TRACKS;
    ui.horizontal(|ui| {
        let mut row = cell.row as u32 + 1;
        let mut column = cell.column as u32 + 1;
        ui.label("Row");
        let r = ui.add(egui::DragValue::new(&mut row).range(1..=max));
        r.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::SpinButton, true, "Row of the cell")
        });
        ui.label("Column");
        let c = ui.add(egui::DragValue::new(&mut column).range(1..=max));
        c.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::SpinButton, true, "Column of the cell")
        });
        if row != cell.row as u32 + 1 || column != cell.column as u32 + 1 {
            edited.grid_row = Some(row);
            edited.grid_column = Some(column);
        }
    });
    ui.horizontal(|ui| {
        let mut rows = edited.row_span.unwrap_or(1);
        let mut columns = edited.span.unwrap_or(1);
        ui.label("Rows it takes");
        let r = ui.add(egui::DragValue::new(&mut rows).range(1..=max));
        r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::SpinButton, true, "Rows it takes"));
        ui.label("Columns");
        let c = ui.add(egui::DragValue::new(&mut columns).range(1..=max));
        c.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::SpinButton, true, "Columns it takes")
        });
        edited.row_span = (rows > 1).then_some(rows);
        edited.span = (columns > 1).then_some(columns);
    });
}
