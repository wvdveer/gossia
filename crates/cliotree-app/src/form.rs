//! The form view of a Data Tab (DESIGN.md, "Forms"): the view selector, and
//! the generated Default form, drawn from the tab's schema each time.
//!
//! A form holds no data and no draft of the tab: a field shows the node it is
//! bound to, and a committed change is an ordinary tab edit. The text being
//! typed into a field is kept only until it is committed or dropped, and a
//! value that does not fit stays in the field with its error and is not
//! written.

use std::collections::HashMap;

use cliotree_calc::entry;
use cliotree_core::form::{
    Address, Alignment, CONTROL_HEIGHTS, CONTROL_WIDTHS, ColumnDesign, Control, ControlKind,
    DEFAULT_LABEL_WIDTH, Editor, Format, LabelPosition, RepeaterMode, TrackSize, flow_rows,
    free_place, grid_cells, track_sizes,
};
use cliotree_core::validate::validate_at;
use cliotree_core::{Kind, Node, Path, TypeId, TypeSpec, Value};
use egui::{Align, Key, RichText, Sense, Stroke, StrokeKind};
use egui_extras::{Column as TableColumn, TableBuilder};

use crate::app::ClioApp;
use crate::form_exprs::{Choice, ChoiceList, Gate, choice_list};
use crate::form_free::{FormRects, GridGeom, Placement, keep_rects, last_rects, snap_move};
use crate::form_layout::{
    ColumnEdit, ControlAction, DragItem, DropAt as DragAt, FormDialog, FreeDrop, LayoutState,
    drop_allowed,
};
use crate::icons;
use crate::rows::display;
use crate::style::Typeface;
use crate::table::{self, Column};
use crate::view::{ROW_HEIGHT, computed_fill, error_color};

/// The form every Data Tab has. It cannot be renamed or deleted.
pub const DEFAULT_FORM: &str = cliotree_calc::DEFAULT_FORM;

/// The width of the labels of fields, in points, when nothing sets it.
const LABEL_WIDTH: f32 = DEFAULT_LABEL_WIDTH as f32;

/// The grid sizes snap to when a handle is dragged, in points.
const SNAP: f32 = 4.0;

/// What a tab shows.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum View {
    /// The data tree, which every Data Tab has.
    #[default]
    Tree,
    /// A form of the tab, by name.
    Form(String),
    /// The document as JSON or XML text.
    Raw,
}

impl View {
    pub fn label(&self) -> &str {
        match self {
            View::Tree => "Data tree",
            View::Form(name) => name,
            View::Raw => "Raw",
        }
    }
}

/// The text typed into a field, until it is committed or dropped.
#[derive(Debug, Clone)]
pub struct Draft {
    pub text: String,
    /// Why the text is not a value of the field, if it is not.
    pub error: Option<String>,
}

/// What the form views keep between frames.
#[derive(Debug, Default)]
pub struct FormState {
    /// Text being typed, by tab and node.
    pub drafts: HashMap<(String, Path), Draft>,
    /// The item a stacked repeater shows, by tab and array.
    pub steps: HashMap<(String, Path), usize>,
    /// The key being typed for a new entry of a map, by tab and map.
    pub new_keys: HashMap<(String, Path), String>,
    /// Scroll so that the control bound to this node is in view.
    pub scroll_to: Option<Path>,
    /// Layout mode, while the form being shown is being edited.
    pub layout: Option<LayoutState>,
    /// The dialog of a command on forms, when one is waiting.
    pub dialog: Option<FormDialog>,
    /// The caret of the text box that had the focus in the frame before, for
    /// the arrow keys.
    pub caret: Option<(egui::Id, usize)>,
    /// Frames left to find the control of `scroll_to` (a stacked item has to
    /// be stepped to first).
    pub scroll_tries: u8,
}

/// Something a control asked for while it was drawn. They are applied after
/// the form, because drawing borrows the workbook.
#[derive(Debug)]
pub(crate) enum FormAction {
    /// Enter the text typed into a field.
    Commit(Path, String),
    /// Enter at once, for the controls that have no text: a toggle, a choice.
    Direct(Path, String),
    /// Create an absent object.
    Create(Path),
    Select(Path),
    Add(Path),
    Delete(Path, usize),
    Move(Path, usize, bool),
    Sort(Path, String),
    Step(Path, usize),
    AddEntry(Path, String),
    DeleteEntry(Path, String),
    /// A click on a button: its label, and the procedure it runs.
    Button(String, String),
    /// A command on a control, in layout mode.
    Control(ControlAction),
    /// A command of the tabular view on a table of the form: a filter, hiding
    /// a row or a column, showing them again.
    Table(crate::view::Action),
}

impl ClioApp {
    // ----------------------------------------------------------- the view

    /// What the current tab shows.
    pub fn current_view(&self) -> &View {
        self.views.get(&self.sheet).unwrap_or(&View::Tree)
    }

    pub fn in_tree(&self) -> bool {
        self.current_view() == &View::Tree
    }

    pub fn in_form(&self) -> bool {
        matches!(self.current_view(), View::Form(_))
    }

    /// The names of the views of the current tab: the data tree and the raw
    /// text, which every Data Tab has, then its forms.
    pub fn view_names(&self) -> Vec<View> {
        let mut views = vec![View::Tree, View::Raw];
        views.extend(
            self.calc()
                .forms()
                .all_names(&self.sheet)
                .into_iter()
                .map(View::Form),
        );
        views
    }

    /// The form the tab shows: the one chosen, or Default if it is gone.
    pub fn form_name(&self) -> String {
        match self.current_view() {
            View::Form(name)
                if name == DEFAULT_FORM || self.calc().forms().get(&self.sheet, name).is_some() =>
            {
                name.clone()
            }
            _ => DEFAULT_FORM.to_owned(),
        }
    }

    /// Shows `view` in the current tab. What is being typed is entered first,
    /// as selecting another cell does; a refused entry keeps the user where
    /// they were and says why. Returns whether the view changed.
    pub fn set_view(&mut self, view: View) -> bool {
        if self.current_view() == &view {
            return true;
        }
        if !self.commit_editing() || !self.enter_drafts() {
            return false;
        }
        let selection = self.selection.clone();
        self.form.layout = None;
        if view == View::Tree {
            self.views.remove(&self.sheet);
            // The control that had the focus is the selected node.
            if let Some(path) = selection {
                self.reveal(&path);
            }
        } else {
            self.raw.error = None;
            self.views.insert(self.sheet.clone(), view);
            self.form.scroll_to = selection;
            self.form.scroll_tries = 3;
        }
        self.notice = None;
        true
    }

    /// Types `text` into the field at `path` and leaves it, as a user does in
    /// the form. Returns why it was refused, if it was.
    pub fn enter_in_form(&mut self, path: &Path, text: &str) -> Result<(), String> {
        if self.commit_field(path, text) {
            return Ok(());
        }
        Err(self
            .form
            .drafts
            .get(&(self.sheet.clone(), path.clone()))
            .and_then(|d| d.error.clone())
            .unwrap_or_else(|| format!("{} cannot be entered here", self.name_of(path))))
    }

    /// What a click on a button does (DESIGN.md, "Button procedures"): what is
    /// being typed is entered first, so that the procedure sees what is on the
    /// screen, and a refused entry stops the click. Then the procedure is
    /// asked for; it runs on the evaluation thread when the workbook is up to
    /// date, and its outcome is shown by [`ClioApp::collect_procedures`].
    /// Returns why it did not start.
    pub fn click_button(&mut self, procedure: &str, label: &str) -> Result<(), String> {
        if let Some(call) = self.session.running_procedure() {
            let message = format!("{} is still running", call.button);
            self.notice = Some(message.clone());
            return Err(message);
        }
        if !self.commit_editing() || !self.enter_drafts() {
            return Err(self.notice.clone().unwrap_or_default());
        }
        let call = cliotree_calc::Call {
            procedure: procedure.to_owned(),
            sheet: self.sheet.clone(),
            form: self.form_name(),
            button: label.to_owned(),
            time_limit: None,
        };
        match self.session.run_procedure(call) {
            Ok(()) => {
                self.notice = None;
                Ok(())
            }
            Err(message) => {
                self.notice = Some(message.clone());
                Err(message)
            }
        }
    }

    /// Takes the procedures that have finished: one that failed, or whose
    /// writes were refused, leaves its message where the user sees it, and the
    /// data as it was.
    pub fn collect_procedures(&mut self) {
        for done in self.session.take_finished() {
            self.notice = match done.outcome {
                Ok(()) => None,
                Err(message) => Some(format!("{}: {message}", done.call.button)),
            };
        }
    }

    /// Enters the text typed into the fields of this tab. A text that is not
    /// valid is not written and keeps the user in the form.
    fn enter_drafts(&mut self) -> bool {
        let sheet = self.sheet.clone();
        let mut pending: Vec<(Path, Draft)> = self
            .form
            .drafts
            .iter()
            .filter(|((s, _), _)| s == &sheet)
            .map(|((_, p), d)| (p.clone(), d.clone()))
            .collect();
        pending.sort_by_key(|(p, _)| p.to_string());
        for (path, draft) in pending {
            if self.draft_is_current(&path, &draft) {
                self.form.drafts.remove(&(sheet.clone(), path));
                continue;
            }
            if let Some(error) = &draft.error {
                self.notice = Some(format!("{}: {error}", self.name_of(&path)));
                return false;
            }
            if !self.commit_field(&path, &draft.text) {
                return false;
            }
        }
        true
    }

    fn draft_is_current(&self, path: &Path, draft: &Draft) -> bool {
        self.calc()
            .data(&self.sheet)
            .and_then(|d| d.get(path))
            .map(|n| value_text(n.value()))
            .unwrap_or_default()
            == draft.text
    }

    /// Enters the text of a field. On refusal the text stays in the field with
    /// the reason.
    fn commit_field(&mut self, path: &Path, text: &str) -> bool {
        let sheet = self.sheet.clone();
        let kind = self
            .calc()
            .spec(&sheet)
            .and_then(|s| s.resolve(path).map(|r| s.get(r.ty).kind.clone()));
        let Some(kind) = kind else {
            return false;
        };
        let entered = entry::enter(
            self.session.calc_mut(),
            &sheet,
            path,
            &entry_text(&kind, text),
            false,
        );
        let key = (sheet, path.clone());
        match entered {
            Ok(()) => {
                self.form.drafts.remove(&key);
                self.notice = None;
                self.session.tick();
                true
            }
            Err(e) => {
                self.form.drafts.insert(
                    key,
                    Draft {
                        text: text.to_owned(),
                        error: Some(e.to_string()),
                    },
                );
                false
            }
        }
    }

    // ------------------------------------------------------------ drawing

    /// The form of the tab, filling the central panel: the strip with the
    /// switch to layout mode, then the controls of its design.
    pub(crate) fn form_view(&mut self, ui: &mut egui::Ui) {
        if let Some(notice) = &self.notice {
            let color = error_color(ui);
            ui.colored_label(color, notice);
        }
        let name = self.form_name();
        self.form_bar(ui, &name);
        let design = self.design_of(&name);
        let layout = self.layout_selection(&name);
        let column = self.form.layout.as_ref().and_then(|l| l.column.clone());
        let mut drafts = std::mem::take(&mut self.form.drafts);
        let mut new_keys = std::mem::take(&mut self.form.new_keys);
        let scroll_to = self.form.scroll_to.take();
        let mut actions = Vec::new();
        let mut nav = Vec::new();
        let mut next_scroll = None;
        let tries = self.form.scroll_tries;
        let mut tried = false;
        {
            let calc = self.calc();
            if let (Some(spec), Some(data)) = (calc.spec(&self.sheet), calc.data(&self.sheet)) {
                let last = last_rects(ui.ctx(), &self.sheet, &name);
                let also = self
                    .form
                    .layout
                    .as_ref()
                    .map(|l| l.also.clone())
                    .unwrap_or_default();
                let mut painter = Painter {
                    app: self,
                    form: &name,
                    spec,
                    data,
                    drafts: &mut drafts,
                    new_keys: &mut new_keys,
                    actions: Vec::new(),
                    scroll_to,
                    layout,
                    look: None,
                    font: (None, None),
                    column,
                    column_targets: Vec::new(),
                    at: Vec::new(),
                    drops: Vec::new(),
                    sized: false,
                    label_width: None,
                    also,
                    free_parent: None,
                    grid_parent: None,
                    free_bottom: None,
                    rects: FormRects::default(),
                    last,
                    nav: Vec::new(),
                    seen: Vec::new(),
                    item: None,
                };
                // Both ways: a form whose rows are wider than the window
                // scrolls sideways to its rightmost fields.
                egui::ScrollArea::both()
                    .id_salt("form_scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let title = design
                            .title
                            .clone()
                            .or_else(|| spec.get(spec.root()).title.clone())
                            .unwrap_or_else(|| self.sheet.clone());
                        let response = ui.heading(&title);
                        painter.arrive(&Path::root(), &response);
                        if let Some(help) = spec.get(spec.root()).description.as_deref() {
                            ui.label(RichText::new(help).weak());
                        }
                        ui.add_space(4.0);
                        if design.controls.is_empty() {
                            ui.weak("This form has no controls. Add some from the palette above.");
                        }
                        painter.controls(ui, &design.controls, &[]);
                        if painter.layout.is_some() {
                            let area = ui.clip_rect();
                            if let Some(action) = drop_target(ui, &painter.drops, area) {
                                painter.actions.push(FormAction::Control(action));
                            }
                        }
                    });
                if painter.layout.is_some() {
                    keep_rects(
                        ui.ctx(),
                        &self.sheet,
                        &name,
                        std::mem::take(&mut painter.rects),
                    );
                }
                actions = painter.actions;
                nav = painter.nav;
                // A node with no control of its own: the nearest one above
                // it that has, in the next frame (a stacked repeater may
                // be stepping to its item first).
                if let Some(missed) = painter.scroll_to
                    && tries > 0
                {
                    tried = true;
                    let stepping = actions.iter().any(|a| matches!(a, FormAction::Step(..)));
                    next_scroll = if stepping {
                        Some(missed)
                    } else {
                        std::iter::successors(missed.parent(), Path::parent)
                            .find(|p| painter.seen.contains(p))
                    };
                }
            }
        }
        self.form.scroll_to = next_scroll;
        self.form.scroll_tries = if tried { tries - 1 } else { 0 };
        self.form_arrows(ui.ctx(), &nav);
        self.form.drafts = drafts;
        self.form.new_keys = new_keys;
        for action in actions {
            self.apply_form(action);
        }
    }

    pub(crate) fn apply_form(&mut self, action: FormAction) {
        let sheet = self.sheet.clone();
        match action {
            FormAction::Commit(path, text) => {
                if self.commit_field(&path, &text) {
                    self.pin_form_row(&path);
                }
            }
            FormAction::Direct(path, text) => {
                self.selection = Some(path.clone());
                self.enter_directly(&path, &text);
                self.pin_form_row(&path);
            }
            FormAction::Create(path) => {
                let value = self
                    .calc()
                    .spec(&sheet)
                    .and_then(|s| s.resolve(&path).map(|r| ClioApp::default_value(s, r.ty)));
                if let Some(value) = value {
                    self.set_node(&path, value);
                }
            }
            FormAction::Button(label, procedure) => {
                let _ = self.click_button(&procedure, &label);
            }
            FormAction::Select(path) if self.picking() => self.finish_pick(&path),
            FormAction::Select(path) => self.selection = Some(path),
            FormAction::Add(array) => {
                let before = self.array_len(&array);
                self.add_item_to(&array);
                if self.array_len(&array) > before {
                    self.pin_form_row(&array.index(before));
                }
            }
            FormAction::Delete(array, index) => {
                self.delete_node(&array.index(index));
            }
            FormAction::Move(array, index, up) => {
                let length = self.array_len(&array);
                let other = if up {
                    index.checked_sub(1)
                } else {
                    Some(index + 1).filter(|&i| i < length)
                };
                if let Some(other) = other {
                    let mut order: Vec<usize> = (0..length).collect();
                    order.swap(index, other);
                    self.reorder(&array, &order, if up { "Move up" } else { "Move down" });
                }
            }
            FormAction::Sort(array, column) => self.sort_table(&array, &column, None),
            FormAction::Step(array, index) => {
                self.form.steps.insert((sheet, array), index);
            }
            FormAction::AddEntry(map, key) => self.add_entry(&map, &key),
            FormAction::DeleteEntry(map, key) => {
                self.delete_node(&map.key(key));
            }
            FormAction::Control(action) => self.apply_control(action),
            FormAction::Table(action) => self.apply(action),
        }
    }

    fn array_len(&self, array: &Path) -> usize {
        self.calc()
            .data(&self.sheet)
            .and_then(|d| d.get(array))
            .and_then(|n| n.value().as_array())
            .map_or(0, <[Node]>::len)
    }

    fn set_node(&mut self, path: &Path, value: Value) {
        let sheet = self.sheet.clone();
        match self.session.calc_mut().set_value(&sheet, path, value) {
            Ok(()) => {
                self.notice = None;
                self.session.tick();
            }
            Err(e) => self.notice = Some(e.to_string()),
        }
    }

    /// Adds an item at the end of an array, making the array if it is absent,
    /// and shows the new item.
    fn add_item_to(&mut self, array: &Path) {
        let present = self
            .calc()
            .data(&self.sheet)
            .is_some_and(|d| d.get(array).is_some());
        if present {
            self.add_row(array, None);
        } else {
            let item = self.calc().spec(&self.sheet).and_then(|spec| {
                match &spec.get(spec.resolve(array)?.ty).kind {
                    Kind::Array { items, .. } => Some(ClioApp::default_value(spec, *items)),
                    _ => None,
                }
            });
            if let Some(item) = item {
                let value = Value::Array(vec![Node::new(item)]);
                self.set_node(array, value);
            }
        }
        let last = self.array_len(array).saturating_sub(1);
        self.form
            .steps
            .insert((self.sheet.clone(), array.clone()), last);
    }

    fn add_entry(&mut self, map: &Path, key: &str) {
        let key = key.trim();
        if key.is_empty() {
            self.notice = Some("type the key of the new entry first".to_owned());
            return;
        }
        if let Err(why) = self.add_map_entry(map, key) {
            self.notice = Some(why);
            return;
        }
        self.form
            .new_keys
            .remove(&(self.sheet.clone(), map.clone()));
    }
}

// ------------------------------------------------------------------ text

/// What the editor of a field with choices is given.
struct ChoiceField<'a> {
    path: &'a Path,
    key: &'a (String, Path),
    /// The accessible name of the editor.
    name: &'a str,
    /// The text of the value stored, empty for none.
    current: &'a str,
    items: &'a [Choice],
    required: bool,
    read_only: bool,
    allow_other: bool,
    /// The width of the editor; None in a cell of a table.
    width: Option<f32>,
}

/// Whether an optional expression has text.
fn has_text(text: &Option<String>) -> bool {
    text.as_deref().is_some_and(|t| !t.trim().is_empty())
}

/// What a dropdown shows for an item: its code and its text (`AMS — Amsterdam`),
/// or the text alone when it is the code (owner, 2026-11-01). Trailing
/// spaces of a code, as COBOL text is padded, are not shown; a code of only
/// spaces is shown as `(spaces)`.
fn choice_label(item: &Choice) -> String {
    if item.text == item.code {
        return item.text.clone();
    }
    let code = item.code.trim_end();
    if code.is_empty() && !item.code.is_empty() {
        format!("(spaces) \u{2014} {}", item.text)
    } else {
        format!("{code} \u{2014} {}", item.text)
    }
}

/// The item whose code is the value stored.
fn position_of(items: &[Choice], current: &str) -> Option<usize> {
    if current.is_empty() {
        return None;
    }
    items.iter().position(|item| same_code(&item.code, current))
}

/// Whether a code is the value stored, as text: the same text, or the same
/// number (the code `2` is the stored `2.0`), or the same truth.
fn same_code(code: &str, current: &str) -> bool {
    code == current
        || matches!((code.parse::<f64>(), current.parse::<f64>()), (Ok(a), Ok(b)) if a == b)
        || (matches!(code, "True" | "False" | "true" | "false")
            && code.eq_ignore_ascii_case(current))
}

/// The text of a value as a field shows it.
fn value_text(value: &Value) -> String {
    match value {
        Value::Array(_) | Value::Object(_) | Value::Error(_) => String::new(),
        other => display(other),
    }
}

fn is_text(kind: &Kind) -> bool {
    matches!(kind, Kind::String(_) | Kind::Enum { .. })
}

/// What to give `entry::enter` for the text of a field. A form has no
/// formulas, so text is always a literal: text that starts with `=` is
/// entered with the apostrophe that says so.
fn entry_text(kind: &Kind, text: &str) -> String {
    if text.trim().is_empty() {
        String::new()
    } else if is_text(kind) {
        format!("'{text}")
    } else {
        text.to_owned()
    }
}

/// Why the text is not a value of the field at `path`, if it is not.
fn check_text(spec: &TypeSpec, path: &Path, text: &str) -> Option<String> {
    let kind = &spec.get(spec.resolve(path)?.ty).kind;
    if !is_text(kind) && text.trim_start().starts_with('=') {
        return Some(
            "a form has no formulas; enter a literal here, or a formula in the data tree"
                .to_owned(),
        );
    }
    entry::classify(spec, path, &entry_text(kind, text))
        .err()
        .map(|e| e.to_string())
}

fn hint(kind: &Kind) -> &'static str {
    match kind {
        Kind::Integer(_) => "whole number",
        Kind::Number(_) => "number",
        Kind::Date => "yyyy-mm-dd",
        Kind::DateTime => "yyyy-mm-ddThh:mm:ss",
        _ => "",
    }
}

// --------------------------------------------------------------- painter

struct Painter<'a> {
    app: &'a ClioApp,
    /// The name of the form, for the results of its expressions.
    form: &'a str,
    spec: &'a TypeSpec,
    data: &'a Node,
    drafts: &'a mut HashMap<(String, Path), Draft>,
    new_keys: &'a mut HashMap<(String, Path), String>,
    actions: Vec<FormAction>,
    scroll_to: Option<Path>,
    /// In layout mode: the control that is selected, if one is.
    layout: Option<Option<Address>>,
    /// What the control being drawn says about how to draw its node.
    look: Option<(Path, Look)>,
    /// The font of the group being drawn, which its controls inherit.
    font: (Option<Typeface>, Option<u32>),
    /// In layout mode: the column selected in the table of the selected
    /// control.
    column: Option<String>,
    /// In layout mode: the titles of the columns of the table being drawn,
    /// which select the column when clicked.
    column_targets: Vec<(String, egui::Rect, egui::Rect)>,
    /// Where the control being drawn is in the design, for the expressions
    /// of the columns of its table.
    at: Address,
    /// In layout mode: where each control was drawn, for dropping a dragged
    /// one.
    drops: Vec<DropZone>,
    /// The control being drawn has a width of its own, so its editor fills it
    /// rather than stopping at the usual width.
    sized: bool,
    /// The width of the labels of the fields of the group being drawn.
    label_width: Option<u32>,
    /// In layout mode: the other selected controls, which the align tools
    /// move with the selected one.
    also: Vec<Address>,
    /// The Free group the control about to be drawn is in, taken by it.
    free_parent: Option<FreeParent>,
    /// The Grid group the control about to be drawn is in, taken by it.
    grid_parent: Option<GridParent>,
    /// For a Free or Grid group with a height: where the bottom of its
    /// inside is.
    free_bottom: Option<f32>,
    /// In layout mode: where the controls are drawn in this frame, and where
    /// they were in the frame before.
    rects: FormRects,
    last: FormRects,
    /// The controls the arrow keys can go to.
    nav: Vec<crate::form_nav::NavStop>,
    /// The nodes whose controls were drawn, for finding the nearest one
    /// above a node that has none.
    seen: Vec<Path>,
    /// In the design of a stacked repeater's items: the item drawn, and its
    /// index. The paths of the controls there are relative to it.
    item: Option<(Path, usize)>,
}

/// The Grid group a control is drawn in: where its rows and columns are.
#[derive(Debug, Clone)]
struct GridParent {
    geom: GridGeom,
}

/// The Free group a control is drawn in: its address, the top left of its
/// inside, and the size of its inside.
#[derive(Debug, Clone)]
struct FreeParent {
    at: Address,
    origin: egui::Pos2,
    size: (f32, f32),
}

/// Where a control was drawn in layout mode.
#[derive(Debug, Clone)]
pub(crate) struct DropZone {
    pub at: Address,
    /// For a control other than a group, the whole width of its list, so
    /// that the space beside a narrow one counts as it.
    pub rect: egui::Rect,
    pub group: bool,
    /// For a Free group: its inside, and how many controls it has.
    pub free: Option<(egui::Rect, usize)>,
    /// For a Grid group: its rows and columns, and how many controls it has.
    pub grid: Option<(GridGeom, usize)>,
}

/// Where a dragged control would be put down: in the list of `parent` at
/// `index` (counted as the list is before the move), with the line that
/// shows it, from `x.0` to `x.1` at `y`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DropPlace {
    pub parent: Address,
    pub index: usize,
    pub y: f32,
    pub x: (f32, f32),
    /// In a Free group, the point it goes to; in a Grid group, the cell.
    pub free: Option<DragAt>,
}

/// How close to the top or bottom of a group the pointer puts a control
/// before or after it rather than inside.
const GROUP_EDGE: f32 = 6.0;

/// Where a control dropped at `pos` goes. Over a control, before it or after
/// it by which half the pointer is in; over a group, before or after it near
/// its edges and among its controls inside; elsewhere in `area`, among the
/// controls of the top level.
pub(crate) fn drop_place(
    drops: &[DropZone],
    pos: egui::Pos2,
    area: egui::Rect,
) -> Option<DropPlace> {
    let over = drops
        .iter()
        .filter(|z| z.rect.contains(pos))
        .min_by(|a, b| a.rect.area().total_cmp(&b.rect.area()));
    let beside = |z: &DropZone, after: bool| {
        let (&last, parent) = z.at.split_last()?;
        Some(DropPlace {
            parent: parent.to_vec(),
            index: last + usize::from(after),
            y: if after {
                z.rect.bottom() + 2.0
            } else {
                z.rect.top() - 2.0
            },
            x: (z.rect.left(), z.rect.right()),
            free: None,
        })
    };
    let (parent, frame) = match over {
        Some(z) if !z.group => return beside(z, pos.y > z.rect.center().y),
        Some(z) if pos.y < z.rect.top() + GROUP_EDGE => return beside(z, false),
        Some(z) if pos.y > z.rect.bottom() - GROUP_EDGE => return beside(z, true),
        // In a Free group, at the end of its list, where the pointer is.
        Some(DropZone {
            at,
            free: Some((region, count)),
            ..
        }) => {
            return Some(DropPlace {
                parent: at.clone(),
                index: *count,
                y: pos.y,
                x: (region.left(), region.right()),
                free: Some(DragAt::Free(FreeDrop {
                    point: (
                        (pos.x - region.left()).max(0.0),
                        (pos.y - region.top()).max(0.0),
                    ),
                    size: (region.width(), region.height()),
                })),
            });
        }
        // In a Grid group, at the end of its list, in the cell under the
        // pointer.
        Some(DropZone {
            at,
            grid: Some((geom, count)),
            ..
        }) if geom.region.expand(40.0).contains(pos) => {
            let (row, column) = geom.cell_at(pos);
            let cell = geom.cell_rect(row, column);
            return Some(DropPlace {
                parent: at.clone(),
                index: *count,
                y: cell.top(),
                x: (cell.left(), cell.right()),
                free: Some(DragAt::Cell(row, column)),
            });
        }
        Some(z) => (z.at.clone(), z.rect.shrink(GROUP_EDGE)),
        None if area.contains(pos) => (Vec::new(), area),
        None => return None,
    };
    let children: Vec<&DropZone> = drops
        .iter()
        .filter(|z| {
            z.at.split_last()
                .is_some_and(|(_, p)| p == parent.as_slice())
        })
        .collect();
    let index = children
        .iter()
        .filter(|z| z.rect.center().y < pos.y)
        .count();
    let child = |i: usize| children.iter().find(|z| z.at.last() == Some(&i));
    let (y, x) = match (child(index), children.iter().max_by_key(|z| z.at.last())) {
        (Some(z), _) => (z.rect.top() - 2.0, (z.rect.left(), z.rect.right())),
        (None, Some(z)) => (z.rect.bottom() + 2.0, (z.rect.left(), z.rect.right())),
        (None, None) => (frame.center().y, (frame.left(), frame.right())),
    };
    Some(DropPlace {
        parent,
        index,
        y,
        x,
        free: None,
    })
}

/// While a control is dragged over the form: the line where it would go and
/// its name beside the pointer. When it is let go there, the move.
fn drop_target(ui: &egui::Ui, drops: &[DropZone], area: egui::Rect) -> Option<ControlAction> {
    let ctx = ui.ctx();
    let (item, name) = egui::DragAndDrop::payload::<(DragItem, String)>(ctx)?
        .as_ref()
        .clone();
    let pos = ui.input(|i| i.pointer.latest_pos())?;
    let place = drop_place(drops, pos, area).filter(|p| drop_allowed(&item, &p.parent, p.index));
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::unique("form_drop"),
    ));
    let accent = ui.visuals().selection.stroke.color;
    if let Some(place) = place
        .as_ref()
        .filter(|p| matches!(p.free, Some(DragAt::Cell(..))))
    {
        // The cell it goes to.
        let painter = painter.with_clip_rect(area);
        let cell = egui::Rect::from_min_max(
            egui::pos2(place.x.0, place.y),
            egui::pos2(place.x.1, place.y + 24.0),
        );
        painter.rect_stroke(cell, 2.0, Stroke::new(2.0, accent), StrokeKind::Inside);
    } else if let Some(place) = place.as_ref().filter(|p| p.free.is_some()) {
        // A mark where it goes in the Free group.
        let painter = painter.with_clip_rect(area);
        let at = egui::pos2(place.x.0, 0.0) + egui::vec2(0.0, place.y);
        let point = egui::pos2(pos.x, at.y);
        painter.circle_stroke(point, 5.0, Stroke::new(2.0, accent));
    } else if let Some(place) = &place {
        let painter = painter.with_clip_rect(area);
        let (a, b) = (
            egui::pos2(place.x.0, place.y),
            egui::pos2(place.x.1, place.y),
        );
        painter.line_segment([a, b], Stroke::new(2.0, accent));
        painter.circle_filled(a, 3.0, accent);
        painter.circle_filled(b, 3.0, accent);
    }
    painter.text(
        pos + egui::vec2(14.0, 4.0),
        egui::Align2::LEFT_TOP,
        name,
        egui::TextStyle::Body.resolve(ui.style()),
        ui.visuals().strong_text_color(),
    );
    let released = ui.input(|i| i.pointer.any_released());
    let place = place.filter(|_| released)?;
    Some(ControlAction::Drop(
        item,
        place.parent,
        place.index,
        place.free,
    ))
}

/// How the control of a node wants it drawn, beyond what its type calls for.
#[derive(Debug, Clone, Default)]
struct Look {
    help: Option<String>,
    editor: Option<Editor>,
    read_only: bool,
    /// A string's text box shows dots.
    masked: bool,
    /// Absent: numbers on the right, the rest on the left.
    align: Option<Alignment>,
    format: Format,
    /// Where the label of a field is, and how wide.
    label_position: Option<LabelPosition>,
    label_width: Option<f32>,
    /// The height of a several-lines text box.
    height: Option<f32>,
    /// The list of choices of a dropdown or typeahead, when it has one.
    choices: Option<ChoiceList>,
    /// A typeahead enters text that matches no choice as it is.
    allow_other: bool,
}

/// Where a control is among the others of its list, for its menu.
struct Slot {
    at: Address,
    siblings: usize,
    above_is_group: bool,
}

fn slots(list: &[Control], parent: &[usize]) -> Vec<Slot> {
    // The control above takes others: a group, or a stacked repeater whose
    // items are designed, at the top level (one in an item takes none).
    let takes =
        |c: &Control| c.kind == ControlKind::Group || (c.is_stacked() && !c.children.is_empty());
    (0..list.len())
        .map(|i| {
            let mut at = parent.to_vec();
            at.push(i);
            Slot {
                at,
                siblings: list.len(),
                above_is_group: i > 0 && takes(&list[i - 1]),
            }
        })
        .collect()
}

impl Painter<'_> {
    fn key(&self, path: &Path) -> (String, Path) {
        (self.app.sheet.clone(), path.clone())
    }

    fn arrive(&mut self, path: &Path, response: &egui::Response) {
        if self.scroll_to.is_some() {
            self.seen.push(path.clone());
        }
        if self.scroll_to.as_ref() == Some(path) {
            response.scroll_to_me(Some(Align::Center));
            self.scroll_to = None;
        }
    }

    // ------------------------------------------------------------ controls

    fn controls(&mut self, ui: &mut egui::Ui, list: &[Control], parent: &[usize]) {
        for (control, slot) in list.iter().zip(slots(list, parent)) {
            self.control(ui, control, &slot);
        }
    }

    /// A control as its expressions say: hidden, disabled, in its colours,
    /// with the pending marker while one is waiting to run again. In layout
    /// mode every control is drawn as designed.
    fn control(&mut self, ui: &mut egui::Ui, c: &Control, slot: &Slot) {
        if c.width.is_none() && !c.fill && c.height.is_none() && c.align.is_none() {
            self.control_gated(ui, c, slot);
        } else {
            self.sized(ui, c, slot);
        }
    }

    /// A control with a size or an alignment of its own: drawn in a box of
    /// its width (or the whole width, for `fill`), put where its alignment
    /// says. A control without a width is placed by the width it had in the
    /// frame before, as egui knows a size only once it has drawn it.
    fn sized(&mut self, ui: &mut egui::Ui, c: &Control, slot: &Slot) {
        let available = ui.available_width();
        let width = if c.fill {
            Some(available)
        } else {
            c.width.map(|w| w as f32)
        };
        let id = ui.scope_id().with(("control_width", slot.at.clone()));
        let known = width.or_else(|| ui.data(|d| d.get_temp::<f32>(id)));
        let offset = match (c.align, known) {
            (Some(Alignment::Center), Some(w)) => ((available - w) / 2.0).max(0.0),
            (Some(Alignment::Right), Some(w)) => (available - w).max(0.0),
            _ => 0.0,
        };
        let outer = self.sized;
        self.sized = width.is_some();
        // A group sets its own least height, inside its box; a repeater
        // scrolls in its height.
        let min_height = c
            .height
            .filter(|_| !matches!(c.kind, ControlKind::Group | ControlKind::Repeater));
        let rect = ui
            .horizontal_top(|ui| {
                ui.add_space(offset);
                ui.allocate_ui_with_layout(
                    egui::vec2(width.unwrap_or(available - offset), 0.0),
                    egui::Layout::top_down(Align::Min),
                    |ui| {
                        if let Some(w) = width {
                            ui.set_width(w);
                        }
                        if let Some(h) = min_height {
                            ui.set_min_height(h as f32);
                        }
                        self.control_gated(ui, c, slot);
                    },
                )
                .response
                .rect
            })
            .inner;
        self.sized = outer;
        if width.is_none() && known != Some(rect.width()) {
            ui.data_mut(|d| d.insert_temp(id, rect.width()));
            ui.ctx().request_repaint();
        }
    }

    fn control_gated(&mut self, ui: &mut egui::Ui, c: &Control, slot: &Slot) {
        if self.layout.is_some() {
            self.control_in_style(ui, c, slot);
            return;
        }
        let item = self.item.as_ref().map(|(_, i)| *i);
        let gate =
            crate::form_exprs::gate(self.app.calc(), &self.app.sheet, self.form, &slot.at, item);
        if gate == Gate::default() {
            self.control_in_style(ui, c, slot);
            return;
        }
        if gate.hidden {
            return;
        }
        self.styled(ui, &gate, false, |this, ui| {
            this.control_in_style(ui, c, slot)
        });
    }

    /// Draws `body` as a gate says: disabled, in its colours, with the
    /// pending marker in the top-right corner and the reasons on hover. The
    /// fill of a control is a frame around it; the fill of a cell of a table
    /// is the whole cell.
    fn styled(
        &mut self,
        ui: &mut egui::Ui,
        gate: &Gate,
        cell: bool,
        body: impl FnOnce(&mut Self, &mut egui::Ui),
    ) {
        let response = ui
            .scope(|ui| {
                // On a fill with no text colour of its own, text is black or
                // white, whichever reads on it.
                let text = gate
                    .text
                    .or_else(|| gate.fill.map(crate::form_exprs::readable_on));
                if let Some(text) = text {
                    let visuals = ui.visuals_mut();
                    visuals.override_text_color = Some(text);
                    // Titles are drawn in the strong colour, and the tick of a
                    // check box and the arrow of a drop-down in the colour of
                    // a widget at rest.
                    visuals.widgets.active.fg_stroke.color = text;
                    visuals.widgets.inactive.fg_stroke.color = text;
                }
                let inner = |this: &mut Self, ui: &mut egui::Ui| {
                    ui.add_enabled_ui(!gate.disabled, |ui| body(this, ui));
                };
                match gate.fill {
                    Some(fill) => {
                        // A text box draws its own background.
                        ui.visuals_mut().extreme_bg_color = fill;
                        ui.visuals_mut().text_edit_bg_color = Some(fill);
                        // So do buttons, check boxes and drop-downs at rest.
                        let widgets = &mut ui.visuals_mut().widgets;
                        let outline = widgets.noninteractive.bg_stroke.color;
                        widgets.inactive.bg_fill = fill;
                        widgets.inactive.weak_bg_fill = fill;
                        if widgets.inactive.bg_stroke.width == 0.0 {
                            widgets.inactive.bg_stroke = Stroke::new(1.0, outline);
                        }
                        if cell {
                            ui.painter().rect_filled(ui.max_rect(), 0.0, fill);
                            inner(self, ui);
                        } else {
                            egui::Frame::new()
                                .fill(fill)
                                .corner_radius(3.0)
                                .inner_margin(2.0)
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    inner(self, ui);
                                });
                        }
                    }
                    None => inner(self, ui),
                }
            })
            .response;
        if gate.pending {
            // In the top-right corner, over what is there.
            let side = if cell { 7.0 } else { 10.0 };
            let rect = egui::Rect::from_min_size(
                egui::pos2(response.rect.right() - side, response.rect.top()),
                egui::vec2(side, side),
            );
            ui.put(rect, |ui: &mut egui::Ui| crate::style::pending_marker(ui))
                .on_hover_text("Waiting to be calculated");
        }
        if !gate.why.is_empty() {
            let why = gate.why.join("\n");
            response.clone().on_hover_text(&why);
            response.on_disabled_hover_text(why);
        }
    }

    /// A control, in its font: its own, or else the one of the group it is
    /// in, for the typeface and the size separately.
    fn control_in_style(&mut self, ui: &mut egui::Ui, c: &Control, slot: &Slot) {
        if c.font.is_none() && c.font_size.is_none() {
            self.control_in_font(ui, c, slot);
            return;
        }
        let outer = self.font;
        self.font = (c.font.or(outer.0), c.font_size.or(outer.1));
        let (typeface, size) = self.font;
        ui.scope(|ui| {
            crate::style::use_font(ui.style_mut(), typeface, size);
            self.control_in_font(ui, c, slot);
        });
        self.font = outer;
    }

    fn control_in_font(&mut self, ui: &mut egui::Ui, c: &Control, slot: &Slot) {
        self.at = slot.at.clone();
        let free = self.free_parent.take();
        let grid = self.grid_parent.take();
        if c.kind == ControlKind::Group || (self.layout.is_some() && self.designs_items(c)) {
            self.group(ui, c, slot, free, grid);
        } else if self.layout.is_none() {
            self.leaf(ui, c);
        } else {
            // In layout mode a control is drawn as it will look, but cannot be
            // used: a click selects it, and a click on the title of a column
            // of its table selects the column.
            self.column_targets.clear();
            let rect = ui
                .scope(|ui| {
                    ui.add_enabled_ui(false, |ui| self.leaf(ui, c));
                })
                .response
                .rect;
            let columns = std::mem::take(&mut self.column_targets);
            let id = ui.scope_id().with(("control", slot.at.clone()));
            let target = ui.interact(rect, id, Sense::click_and_drag());
            self.rects.controls.insert(slot.at.clone(), rect);
            match (&free, &grid) {
                // In a Free group a drag moves it to a point, in a Grid group
                // to a cell.
                (Some(free), _) => self.free_drag(ui, &target, rect, slot, free),
                (_, Some(grid)) => self.grid_drag(ui, &target, slot, grid),
                _ => {
                    target.dnd_set_drag_payload((
                        DragItem::Control(slot.at.clone()),
                        control_name(c),
                    ));
                    self.drops.push(DropZone {
                        at: slot.at.clone(),
                        rect: egui::Rect::from_x_y_ranges(ui.max_rect().x_range(), rect.y_range()),
                        group: false,
                        free: None,
                        grid: None,
                    });
                }
            }
            self.layout_target(ui, &target, rect, c, slot);
            for (name, title, column) in columns {
                self.column_target(ui, c, slot, &name, (title, column), rect);
                self.column_edge(ui, slot, &name, column);
            }
            self.handles(ui, rect, c, slot, free.as_ref());
        }
    }

    /// Whether a control is selected in layout mode, as the one the others
    /// follow or as one of the others.
    fn is_selected(&self, at: &Address) -> bool {
        self.layout.as_ref().and_then(Option::as_ref) == Some(at) || self.also.contains(at)
    }

    /// Selects a control on a click, or with Ctrl adds it to the selection
    /// or takes it out.
    fn select_on(&mut self, ui: &egui::Ui, response: &egui::Response, at: &Address) {
        if response.clicked() {
            // Ctrl as the input says, or as the click itself said.
            let ctrl = ui.input(|i| {
                i.modifiers.command
                    || i.events.iter().any(|e| {
                        matches!(e, egui::Event::PointerButton { modifiers, .. } if modifiers.command)
                    })
            });
            let action = if ctrl {
                ControlAction::Toggle(at.clone())
            } else {
                ControlAction::Select(at.clone())
            };
            self.actions.push(FormAction::Control(action));
        }
    }

    /// A control of a Free group dragged in layout mode: it follows the
    /// pointer from where it was, snapped to the grid and to the edges and
    /// centres of the others and of the group, with a guide line for what it
    /// snapped to (Alt drags without snapping). Each drag is one change.
    fn free_drag(
        &mut self,
        ui: &egui::Ui,
        target: &egui::Response,
        rect: egui::Rect,
        slot: &Slot,
        free: &FreeParent,
    ) {
        let id = egui::Id::unique(("form_move", &self.app.sheet, self.form));
        if target.drag_started() {
            self.actions
                .push(FormAction::Control(ControlAction::ResizeStart));
            ui.data_mut(|d| d.insert_temp(id, rect.min - free.origin));
            if !self.is_selected(&slot.at) {
                self.actions
                    .push(FormAction::Control(ControlAction::Select(slot.at.clone())));
            }
        }
        if !target.dragged() {
            return;
        }
        let start = ui.data(|d| d.get_temp::<egui::Vec2>(id));
        let (from, pos) = ui.input(|i| (i.pointer.press_origin(), i.pointer.latest_pos()));
        let (Some(start), Some(from), Some(pos)) = (start, from, pos) else {
            return;
        };
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        let moved = egui::Rect::from_min_size((start + (pos - from)).to_pos2(), rect.size());
        let others: Vec<egui::Rect> = self
            .last
            .controls
            .iter()
            .filter(|(a, _)| {
                **a != slot.at && a.split_last().is_some_and(|(_, p)| p == free.at.as_slice())
            })
            .map(|(_, r)| r.translate(-free.origin.to_vec2()))
            .collect();
        let snap = !ui.input(|i| i.modifiers.alt);
        let area = egui::vec2(free.size.0, free.size.1);
        let (to, guides) = snap_move(moved, &others, area, snap);
        let painter = ui.ctx().layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::unique("form_guides"),
        ));
        let stroke = Stroke::new(1.0, ui.visuals().selection.stroke.color);
        let o = free.origin;
        for guide in guides {
            let (a, b) = if guide.vertical {
                (
                    egui::pos2(o.x + guide.at, o.y),
                    egui::pos2(o.x + guide.at, o.y + area.y),
                )
            } else {
                (
                    egui::pos2(o.x, o.y + guide.at),
                    egui::pos2(o.x + area.x, o.y + guide.at),
                )
            };
            painter.line_segment([a, b], stroke);
        }
        self.actions.push(FormAction::Control(ControlAction::Place(
            vec![Placement {
                at: slot.at.clone(),
                x: Some(to.x),
                y: Some(to.y),
                size: Some(free.size),
                ..Placement::default()
            }],
            Some("move".to_owned()),
        )));
    }

    /// A control of a Grid group dragged in layout mode: the cell under the
    /// pointer is outlined (one past the last row or column adds one), and
    /// letting go puts the control in it. Each drag is one change.
    fn grid_drag(
        &mut self,
        ui: &egui::Ui,
        target: &egui::Response,
        slot: &Slot,
        grid: &GridParent,
    ) {
        if target.drag_started() && !self.is_selected(&slot.at) {
            self.actions
                .push(FormAction::Control(ControlAction::Select(slot.at.clone())));
        }
        if !(target.dragged() || target.drag_stopped()) {
            return;
        }
        let Some(pos) = ui.input(|i| i.pointer.latest_pos()) else {
            return;
        };
        let (row, column) = grid.geom.cell_at(pos);
        if target.drag_stopped() {
            self.actions.push(FormAction::Control(ControlAction::Place(
                vec![Placement {
                    at: slot.at.clone(),
                    cell: Some((row, column)),
                    ..Placement::default()
                }],
                None,
            )));
            return;
        }
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        let painter = ui.ctx().layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::unique("form_guides"),
        ));
        painter.rect_stroke(
            grid.geom.cell_rect(row, column),
            2.0,
            Stroke::new(2.0, ui.visuals().selection.stroke.color),
            StrokeKind::Inside,
        );
    }

    /// The controls of a Grid group, each in its cell. A column or row is as
    /// big as it says, or its biggest control (known from the frame before),
    /// or its share of what the others leave; a control in a column of a set
    /// size fills its cell unless it has a width, which its alignment places.
    fn grid_children(&mut self, ui: &mut egui::Ui, c: &Control, slot: &Slot) {
        let layout = self.layout.is_some();
        let (cells, nrows, ncols) = grid_cells(c);
        let gap = ui.spacing().item_spacing;
        let width = ui.available_width();
        let key = egui::Id::unique(("form_grid_sizes", &self.app.sheet, self.form, &slot.at));
        let measured: Vec<egui::Vec2> = ui.data(|d| d.get_temp(key)).unwrap_or_default();
        let size_of = |i: usize| measured.get(i).copied().unwrap_or(egui::Vec2::ZERO);
        let mut column_content = vec![0.0f32; ncols];
        let mut row_content = vec![0.0f32; nrows];
        for cell in &cells {
            let m = size_of(cell.index);
            if cell.columns == 1 {
                column_content[cell.column] = column_content[cell.column].max(m.x);
            }
            if cell.rows == 1 {
                row_content[cell.row] = row_content[cell.row].max(m.y);
            }
        }
        // In layout mode an empty row or column still has room to drop into.
        if layout {
            column_content.iter_mut().for_each(|v| *v = v.max(40.0));
            row_content.iter_mut().for_each(|v| *v = v.max(24.0));
        }
        let bottom_of_box = self.free_bottom.take();
        let fixed = c.height.map(|h| {
            bottom_of_box
                .map_or(h as f32, |b| b - ui.cursor().top())
                .max(16.0)
        });
        let columns = track_sizes(&c.grid_columns, &column_content, Some(width), gap.x);
        let rows = track_sizes(&c.grid_rows, &row_content, fixed, gap.y);
        let ranges = |sizes: &[f32], start: f32, gap: f32| {
            let mut at = start;
            sizes
                .iter()
                .map(|s| {
                    let range = (at, at + s);
                    at += s + gap;
                    range
                })
                .collect::<Vec<(f32, f32)>>()
        };
        let total = |sizes: &[f32], gap: f32| {
            sizes.iter().sum::<f32>() + gap * sizes.len().saturating_sub(1) as f32
        };
        let height = total(&rows, gap.y).max(fixed.unwrap_or(0.0));
        let (region, _) = ui.allocate_exact_size(
            egui::vec2(total(&columns, gap.x).max(width), height),
            Sense::hover(),
        );
        let geom = GridGeom {
            region,
            columns: ranges(&columns, region.left(), gap.x),
            rows: ranges(&rows, region.top(), gap.y),
        };
        let list = slots(&c.children, &slot.at);
        let mut sizes = vec![egui::Vec2::ZERO; c.children.len()];
        for cell in &cells {
            let child = &c.children[cell.index];
            let x0 = geom.columns[cell.column].0;
            let x1 = geom.columns[cell.column + cell.columns - 1].1;
            let y0 = geom.rows[cell.row].0;
            let cell_width = x1 - x0;
            let automatic = cell.columns == 1
                && c.grid_columns
                    .get(cell.column)
                    .is_none_or(|t| *t == TrackSize::Auto);
            let w = match child.width {
                Some(w) if !child.fill => Some(w as f32),
                _ if automatic && !child.fill => None,
                _ => Some(cell_width),
            };
            let offset = match (child.align, w) {
                (Some(Alignment::Center), Some(w)) => ((cell_width - w) / 2.0).max(0.0),
                (Some(Alignment::Right), Some(w)) => (cell_width - w).max(0.0),
                _ => 0.0,
            };
            let max_width = w.unwrap_or((region.right() - x0).max(24.0));
            let max = egui::Rect::from_min_size(
                egui::pos2(x0 + offset, y0),
                egui::vec2(max_width, 100_000.0),
            );
            let mut child_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(max)
                    .layout(egui::Layout::top_down(Align::Min)),
            );
            if let Some(w) = w {
                child_ui.set_width(w);
            }
            if let Some(h) = child
                .height
                .filter(|_| !matches!(child.kind, ControlKind::Group | ControlKind::Repeater))
            {
                child_ui.set_min_height(h as f32);
            }
            let outer = self.sized;
            self.sized = w.is_some();
            self.grid_parent = Some(GridParent { geom: geom.clone() });
            self.control_gated(&mut child_ui, child, &list[cell.index]);
            self.grid_parent = None;
            self.sized = outer;
            sizes[cell.index] = child_ui.min_rect().size();
        }
        let changed = sizes.len() != measured.len()
            || sizes
                .iter()
                .zip(&measured)
                .any(|(a, b)| (*a - *b).length() > 0.5);
        if changed {
            ui.data_mut(|d| d.insert_temp(key, sizes));
            ui.ctx().request_repaint();
        }
        if layout {
            // The lines between the rows and the columns.
            let stroke = Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color);
            let painter = ui.painter();
            painter.rect_stroke(region, 0.0, stroke, StrokeKind::Inside);
            for (_, end) in geom
                .columns
                .iter()
                .take(geom.columns.len().saturating_sub(1))
            {
                let x = end + gap.x / 2.0;
                painter.line_segment(
                    [egui::pos2(x, region.top()), egui::pos2(x, region.bottom())],
                    stroke,
                );
            }
            for (_, end) in geom.rows.iter().take(geom.rows.len().saturating_sub(1)) {
                let y = end + gap.y / 2.0;
                painter.line_segment(
                    [egui::pos2(region.left(), y), egui::pos2(region.right(), y)],
                    stroke,
                );
            }
            self.rects.grids.insert(slot.at.clone(), geom);
        }
    }

    /// The controls of a Free group, each at its place, the later ones over
    /// the earlier. The group is as tall as its lowest control (known from
    /// the frame before), or its height.
    fn free_children(&mut self, ui: &mut egui::Ui, c: &Control, slot: &Slot) {
        let layout = self.layout.is_some();
        let width = ui.available_width();
        let key = egui::Id::unique(("form_free_height", &self.app.sheet, self.form, &slot.at));
        let bottom_of_box = self.free_bottom.take();
        let fixed = c.height.map(|h| {
            bottom_of_box
                .map_or(h as f32, |b| b - ui.cursor().top())
                .max(16.0)
        });
        let last = ui.data(|d| d.get_temp::<f32>(key)).unwrap_or(0.0);
        let least = if layout { 48.0 } else { 0.0 };
        let height = fixed.unwrap_or(last).max(least);
        let (region, _) = ui.allocate_exact_size(egui::vec2(width, height), Sense::hover());
        let size = (width, height);
        let mut bottom: f32 = 0.0;
        for (child, s) in c.children.iter().zip(slots(&c.children, &slot.at)) {
            let place = free_place(c, child, size);
            let min = region.min + egui::vec2(place.left, place.top);
            let w = place.width.unwrap_or((region.right() - min.x).max(24.0));
            let max = egui::Rect::from_min_size(min, egui::vec2(w, 100_000.0));
            let mut child_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(max)
                    .layout(egui::Layout::top_down(Align::Min)),
            );
            if let Some(w) = place.width {
                child_ui.set_width(w);
            }
            if let Some(h) = place
                .height
                .filter(|_| !matches!(child.kind, ControlKind::Group | ControlKind::Repeater))
            {
                child_ui.set_min_height(h);
            }
            let outer = self.sized;
            self.sized = place.width.is_some();
            self.free_parent = Some(FreeParent {
                at: slot.at.clone(),
                origin: region.min,
                size,
            });
            self.control_gated(&mut child_ui, child, &s);
            self.free_parent = None;
            self.sized = outer;
            bottom = bottom.max(child_ui.min_rect().bottom() - region.top());
        }
        if fixed.is_none() && (bottom - last).abs() > 0.5 {
            ui.data_mut(|d| d.insert_temp(key, bottom));
            ui.ctx().request_repaint();
        }
        if layout {
            self.rects.free.insert(slot.at.clone(), region);
            ui.painter().rect_stroke(
                region,
                0.0,
                Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color),
                StrokeKind::Inside,
            );
        }
    }

    /// The handles of the selected control in layout mode: its right edge
    /// sets the width, its bottom edge the height, and the corner both, in
    /// points snapped to a grid (Alt drags without snapping). A double click
    /// on a handle puts its size back to automatic.
    fn handles(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        c: &Control,
        slot: &Slot,
        free: Option<&FreeParent>,
    ) {
        let selected = self.layout.as_ref().and_then(Option::as_ref) == Some(&slot.at)
            && self.column.is_none();
        if !selected {
            return;
        }
        let rect = rect.expand(2.0);
        let handles = [
            (true, false, rect.right_center(), "Width handle"),
            (false, true, rect.center_bottom(), "Height handle"),
            (true, true, rect.right_bottom(), "Size handle"),
        ];
        for (sets_width, sets_height, centre, name) in handles {
            if sets_height && c.kind == ControlKind::Separator {
                continue;
            }
            let area = egui::Rect::from_center_size(centre, egui::vec2(8.0, 8.0));
            // Not from the Ui: a control that gets a size is drawn in a box of its
            // own, and the drag must survive that.
            let id = egui::Id::unique(("form_resize", &self.app.sheet, self.form, &slot.at, name));
            let response = ui.interact(area, id, Sense::click_and_drag());
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, name));
            let colour = ui.visuals().selection.stroke.color;
            ui.painter().rect_filled(area.shrink(1.0), 1.0, colour);
            ui.painter().rect_stroke(
                area.shrink(1.0),
                1.0,
                Stroke::new(1.0, ui.visuals().extreme_bg_color),
                StrokeKind::Outside,
            );
            if response.hovered() || response.dragged() {
                ui.ctx().set_cursor_icon(match (sets_width, sets_height) {
                    (true, true) => egui::CursorIcon::ResizeNwSe,
                    (true, false) => egui::CursorIcon::ResizeHorizontal,
                    _ => egui::CursorIcon::ResizeVertical,
                });
            }
            if response.double_clicked() {
                self.actions.push(FormAction::Control(ControlAction::Resize(
                    slot.at.clone(),
                    sets_width.then_some(None),
                    sets_height.then_some(None),
                )));
                continue;
            }
            if response.drag_started() {
                self.actions
                    .push(FormAction::Control(ControlAction::ResizeStart));
            }
            if response.dragged()
                && let Some(pointer) = response.interact_pointer_pos()
            {
                let snap = !ui.input(|i| i.modifiers.alt);
                let width = snapped(pointer.x - rect.left(), snap, &CONTROL_WIDTHS);
                let height = snapped(pointer.y - rect.top(), snap, &CONTROL_HEIGHTS);
                // In a Free group, as seen, so that a stretched control keeps
                // what it shows.
                let action = match free {
                    Some(free) => ControlAction::Place(
                        vec![Placement {
                            at: slot.at.clone(),
                            width: sets_width.then_some(width as f32),
                            height: sets_height.then_some(height as f32),
                            size: Some(free.size),
                            ..Placement::default()
                        }],
                        Some("resize".to_owned()),
                    ),
                    None => ControlAction::Resize(
                        slot.at.clone(),
                        sets_width.then_some(Some(width)),
                        sets_height.then_some(Some(height)),
                    ),
                };
                self.actions.push(FormAction::Control(action));
            }
        }
    }

    /// The title of a column of a table in layout mode: a click selects the
    /// column, and its menu moves or hides it.
    /// In layout mode: the right edge of a column's title, which is dragged
    /// to give the column a width of its own. A line shows where the edge
    /// goes; letting go sets the width, one step of undo. (The table is drawn
    /// disabled in layout mode, so its own handles do not work.)
    fn column_edge(&mut self, ui: &mut egui::Ui, slot: &Slot, name: &str, column: egui::Rect) {
        let gap = ui.spacing().item_spacing.x;
        let edge = egui::Rect::from_x_y_ranges(
            (column.right() - 3.0)..=(column.right() + gap + 3.0),
            column.y_range(),
        );
        let id = ui.scope_id().with(("column_edge", slot.at.clone(), name));
        let handle = ui.interact(edge, id, Sense::drag());
        let label = format!("Width of column {name}");
        handle.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Splitter, true, &label));
        if handle.hovered() || handle.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeColumn);
        }
        let pointer = ui.input(|i| i.pointer.interact_pos());
        if handle.drag_started()
            && let Some(p) = pointer
        {
            ui.data_mut(|d| d.insert_temp(id, p.x));
        }
        let origin: Option<f32> = ui.data(|d| d.get_temp(id));
        let (Some(origin), Some(p)) = (origin, pointer) else {
            return;
        };
        let width = (column.width() + p.x - origin).max(24.0);
        if handle.dragged() {
            ui.painter().vline(
                column.left() + width,
                column.y_range(),
                Stroke::new(2.0, ui.visuals().selection.stroke.color),
            );
        }
        if handle.drag_stopped() {
            ui.data_mut(|d| d.remove::<f32>(id));
            if (width - column.width()).abs() >= 1.0 {
                self.actions.push(FormAction::Control(ControlAction::Column(
                    slot.at.clone(),
                    name.to_owned(),
                    ColumnEdit::Width(width.round() as u32),
                )));
            }
        }
    }

    fn column_target(
        &mut self,
        ui: &mut egui::Ui,
        c: &Control,
        slot: &Slot,
        name: &str,
        (title, _column): (egui::Rect, egui::Rect),
        table: egui::Rect,
    ) {
        let id = ui.scope_id().with(("column", slot.at.clone(), name));
        let target = ui.interact(title, id, Sense::click());
        let label = column_label(name, c);
        target.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label.clone()));
        if target.clicked() {
            self.actions
                .push(FormAction::Control(ControlAction::SelectColumn(
                    slot.at.clone(),
                    name.to_owned(),
                )));
        }
        let selected = self.layout.as_ref().and_then(Option::as_ref) == Some(&slot.at)
            && self.column.as_deref() == Some(name);
        if selected {
            // The whole column, from its title down to the end of the table.
            let column = egui::Rect::from_min_max(
                title.min,
                egui::pos2(title.max.x, (table.max.y - 6.0).max(title.max.y)),
            );
            ui.painter().rect_stroke(
                column,
                3.0,
                Stroke::new(1.5, ui.visuals().selection.stroke.color),
                StrokeKind::Inside,
            );
        } else if target.hovered() {
            ui.painter().rect_stroke(
                title,
                3.0,
                Stroke::new(1.0, ui.visuals().widgets.hovered.bg_stroke.color),
                StrokeKind::Inside,
            );
        }
        let at = slot.at.clone();
        target.context_menu(|ui| {
            let mut command = |ui: &mut egui::Ui, label: &str, edit: ColumnEdit| {
                if ui.button(label).clicked() {
                    self.actions.push(FormAction::Control(ControlAction::Column(
                        at.clone(),
                        name.to_owned(),
                        edit,
                    )));
                    ui.close();
                }
            };
            command(ui, "Move column left", ColumnEdit::Move(true));
            command(ui, "Move column right", ColumnEdit::Move(false));
            command(ui, "Hide column", ColumnEdit::Show(false));
        });
    }

    /// What selecting a control in layout mode looks like and does.
    fn layout_target(
        &mut self,
        ui: &mut egui::Ui,
        target: &egui::Response,
        rect: egui::Rect,
        c: &Control,
        slot: &Slot,
    ) {
        // When a column of its table is selected, the column is outlined, not
        // the control.
        let selected = self.is_selected(&slot.at) && self.column.is_none();
        let name = control_name(c);
        target.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, name.clone()));
        self.select_on(ui, target, &slot.at);
        if selected {
            ui.painter().rect_stroke(
                rect.expand(2.0),
                3.0,
                Stroke::new(1.5, ui.visuals().selection.stroke.color),
                StrokeKind::Outside,
            );
        } else if target.hovered() {
            ui.painter().rect_stroke(
                rect.expand(2.0),
                3.0,
                Stroke::new(1.0, ui.visuals().widgets.hovered.bg_stroke.color),
                StrokeKind::Outside,
            );
        }
        target.context_menu(|ui| {
            self.control_menu(ui, slot);
        });
    }

    /// The commands on a control, in its context menu.
    fn control_menu(&mut self, ui: &mut egui::Ui, slot: &Slot) {
        let index = slot.at.last().copied().unwrap_or(0);
        let nested = slot.at.len() > 1;
        let mut commands = vec![
            (
                "Move up",
                index > 0,
                ControlAction::Shift(slot.at.clone(), true),
            ),
            (
                "Move down",
                index + 1 < slot.siblings,
                ControlAction::Shift(slot.at.clone(), false),
            ),
            (
                "Move into the group above",
                slot.above_is_group,
                ControlAction::Nest(slot.at.clone(), true),
            ),
            (
                "Move out of the group",
                nested,
                ControlAction::Nest(slot.at.clone(), false),
            ),
        ];
        commands.push((
            "Delete control",
            true,
            ControlAction::Delete(slot.at.clone()),
        ));
        for (label, enabled, action) in commands {
            if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                self.actions.push(FormAction::Control(action));
                ui.close();
            }
        }
        // With several selected, this one among them: the align tools.
        let selection: Vec<Address> = self
            .layout
            .clone()
            .flatten()
            .into_iter()
            .chain(self.also.iter().cloned())
            .collect();
        if selection.len() > 1 && selection.contains(&slot.at) {
            ui.menu_button("Align", |ui| {
                for how in crate::form_free::Arrange::ALL {
                    let placements = crate::form_free::arranged(how, &selection, &self.last);
                    if ui
                        .add_enabled(placements.is_some(), egui::Button::new(how.label()))
                        .clicked()
                    {
                        self.actions.push(FormAction::Control(ControlAction::Place(
                            placements.unwrap_or_default(),
                            None,
                        )));
                        ui.close();
                    }
                }
            });
        }
    }

    /// A group: its controls in columns, in a box when it has a title or is
    /// bound to an object.
    fn group(
        &mut self,
        ui: &mut egui::Ui,
        c: &Control,
        slot: &Slot,
        free: Option<FreeParent>,
        grid: Option<GridParent>,
    ) {
        let layout = self.layout.is_some();
        let problem = self.problem_of(c);
        let path = self.path_of(c).filter(|_| problem.is_none());
        // A stacked repeater in layout mode: its controls are the design of
        // its items, drawn for the item its stepper is at.
        let item = match (&path, c.kind) {
            (Some(array), ControlKind::Repeater) => {
                let len = self
                    .data
                    .get(array)
                    .and_then(|n| n.value().as_array())
                    .map_or(0, <[Node]>::len);
                let at = self.step_of(array, len);
                Some((array.index(at), at))
            }
            _ => None,
        };
        let label = c.label.clone().or_else(|| {
            path.as_ref()
                .and_then(last_name)
                .or_else(|| layout.then(|| "(group)".to_owned()))
        });
        // An object that is not there is made on request; in layout mode its
        // controls are shown all the same.
        if let Some(path) = &path
            && !layout
            && !path.is_root()
            && self.data.get(path).is_none()
        {
            let ty = self.spec.resolve(path).map(|r| r.ty);
            ui.horizontal(|ui| {
                let help = ty.and_then(|ty| self.help(path, ty));
                let width = self.label_width.map_or(LABEL_WIDTH, |w| w as f32);
                label_cell(ui, label.as_deref().unwrap_or(""), false, help, width);
                let can = self.app.can_edit(path);
                if ui.add_enabled(can, egui::Button::new("Add")).clicked() {
                    self.actions.push(FormAction::Create(path.clone()));
                }
            });
            return;
        }
        let framed = label.is_some() || path.is_some() || layout;
        let draw = |this: &mut Self, ui: &mut egui::Ui| {
            if let Some(problem) = &problem {
                this.placeholder(ui, c, problem);
                return;
            }
            // The controls inside set their own sizes, and take the width of
            // the labels of the group.
            let outer = (this.sized, this.label_width);
            this.sized = false;
            this.label_width = c.label_width.or(this.label_width);
            let outer_item = this.item.clone();
            if item.is_some() {
                this.item = item.clone();
            }
            ui.scope(|ui| {
                if let Some(gap) = c.spacing {
                    ui.spacing_mut().item_spacing = egui::vec2(gap as f32, gap as f32);
                }
                if c.is_free() {
                    this.free_children(ui, c, slot);
                    return;
                }
                if c.is_grid() {
                    this.grid_children(ui, c, slot);
                    return;
                }
                let columns = c.columns.unwrap_or(1).clamp(1, 6) as usize;
                if columns == 1 {
                    this.controls(ui, &c.children, &slot.at);
                    return;
                }
                // Rows of cells of equal width, as tall as their tallest
                // control, so that the controls of a row line up.
                let list = slots(&c.children, &slot.at);
                let gap = ui.spacing().item_spacing.x;
                let cell = ((ui.available_width() - gap * (columns - 1) as f32) / columns as f32)
                    .max(24.0);
                for row in flow_rows(&c.children, columns) {
                    ui.horizontal_top(|ui| {
                        for &(i, _, span) in &row.cells {
                            let width = cell * span as f32 + gap * (span - 1) as f32;
                            ui.allocate_ui_with_layout(
                                egui::vec2(width, 0.0),
                                egui::Layout::top_down(Align::Min),
                                |ui| {
                                    ui.set_width(width);
                                    this.control(ui, &c.children[i], &list[i]);
                                },
                            );
                        }
                    });
                }
            });
            this.item = outer_item;
            (this.sized, this.label_width) = outer;
        };
        if !framed {
            draw(self, ui);
            return;
        }
        let mut frame = egui::Frame::group(ui.style());
        if let Some(padding) = c.padding {
            frame = frame.inner_margin(padding as f32);
        }
        let mut header_response = None;
        let response = frame.show(ui, |ui| {
            ui.set_width(ui.available_width());
            if let Some(height) = c.height {
                let margin = frame.inner_margin.sum().y + frame.stroke.width * 2.0;
                ui.set_min_height((height as f32 - margin).max(0.0));
                if c.is_free() || c.is_grid() {
                    self.free_bottom = Some(ui.cursor().top() + (height as f32 - margin).max(0.0));
                }
            }
            let title = label.clone().unwrap_or_default();
            if layout {
                let header = ui.add(
                    egui::Label::new(RichText::new(&title).strong()).sense(Sense::click_and_drag()),
                );
                let name = control_name(c);
                // In a Free or Grid group, dragging the title moves the group.
                if free.is_none() && grid.is_none() {
                    header.dnd_set_drag_payload((DragItem::Control(slot.at.clone()), name.clone()));
                }
                header.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::Role::Button, true, name.clone())
                });
                self.select_on(ui, &header, &slot.at);
                header.context_menu(|ui| self.control_menu(ui, slot));
                header_response = Some(header);
            } else {
                let response = ui.strong(&title);
                if let Some(path) = &path {
                    self.arrive(path, &response);
                }
            }
            let help = c.help.as_deref().or_else(|| {
                let path = path.as_ref()?;
                self.help(path, self.spec.resolve(path)?.ty)
            });
            if let Some(help) = help {
                ui.label(RichText::new(help).weak().small());
            }
            draw(self, ui);
        });
        if layout {
            self.drops.push(DropZone {
                at: slot.at.clone(),
                rect: response.response.rect,
                group: true,
                free: self
                    .rects
                    .free
                    .get(&slot.at)
                    .map(|r| (*r, c.children.len())),
                grid: self
                    .rects
                    .grids
                    .get(&slot.at)
                    .map(|g| (g.clone(), c.children.len())),
            });
            self.rects
                .controls
                .insert(slot.at.clone(), response.response.rect);
            if let (Some(free), Some(header)) = (&free, &header_response) {
                self.free_drag(ui, header, response.response.rect, slot, free);
            }
            if let (Some(grid), Some(header)) = (&grid, &header_response) {
                self.grid_drag(ui, header, slot, grid);
            }
        }
        if self.is_selected(&slot.at) {
            ui.painter().rect_stroke(
                response.response.rect.expand(2.0),
                3.0,
                Stroke::new(1.5, ui.visuals().selection.stroke.color),
                StrokeKind::Outside,
            );
        }
        if self.layout.as_ref().and_then(Option::as_ref) == Some(&slot.at) {
            self.handles(ui, response.response.rect, c, slot, free.as_ref());
        }
    }

    /// A control that is not a group.
    fn leaf(&mut self, ui: &mut egui::Ui, c: &Control) {
        match c.kind {
            ControlKind::Label => {
                ui.label(c.text.clone().unwrap_or_default());
            }
            ControlKind::Separator => {
                ui.separator();
            }
            ControlKind::Button => {
                let label = c.label.clone().unwrap_or_else(|| "Button".to_owned());
                let running = self
                    .app
                    .session
                    .running_procedure()
                    .map(|c| c.button.clone());
                let (can, hint) = match &c.procedure {
                    None => (false, "No procedure has been chosen.".to_owned()),
                    Some(_) if !self.app.calc().is_trusted() => (
                        false,
                        "Buttons run Python, so this workbook has to be trusted first.".to_owned(),
                    ),
                    Some(_) if running.is_some() => (
                        false,
                        format!(
                            "{} is running; wait for it to finish.",
                            running.unwrap_or_default()
                        ),
                    ),
                    Some(p) => (true, format!("Runs {p}")),
                };
                let width = if self.sized {
                    ui.available_width()
                } else {
                    0.0
                };
                let height = c.height.map_or(0.0, |h| h as f32);
                let button = egui::Button::new(&label).min_size(egui::vec2(width, height));
                let response = ui.add_enabled(can, button);
                if can {
                    self.nav.push(crate::form_nav::NavStop {
                        id: response.id,
                        rect: response.rect,
                        text: false,
                        vertical: true,
                    });
                }
                if can {
                    if response.on_hover_text(hint).clicked()
                        && let Some(procedure) = &c.procedure
                    {
                        self.actions
                            .push(FormAction::Button(label.clone(), procedure.clone()));
                    }
                } else {
                    response.on_disabled_hover_text(hint);
                }
            }
            ControlKind::Field | ControlKind::Repeater => self.bound(ui, c),
            ControlKind::Group => {}
        }
    }

    /// A field or a repeater: the control of the node it is bound to.
    fn bound(&mut self, ui: &mut egui::Ui, c: &Control) {
        if let Some(problem) = self.problem_of(c) {
            self.placeholder(ui, c, &problem);
            return;
        }
        let Some(path) = self.path_of(c) else {
            return;
        };
        let Some(resolved) = self.spec.resolve(&path) else {
            return;
        };
        let ty = resolved.ty;
        let label = c
            .label
            .clone()
            .or_else(|| last_name(&path))
            .unwrap_or_else(|| self.app.sheet.clone());
        self.look = Some((
            path.clone(),
            Look {
                help: c.help.clone(),
                editor: c.editor,
                read_only: c.read_only,
                masked: c.masked,
                align: None,
                format: c.format(),
                label_position: c.label_position,
                label_width: c.label_width.or(self.label_width).map(|w| w as f32),
                height: c.height.map(|h| h as f32),
                choices: has_text(&c.choices)
                    .then(|| {
                        let app = self.app;
                        let item = self.item.as_ref().map(|(_, i)| *i);
                        choice_list(app.calc(), &app.sheet, self.form, &self.at, None, item)
                    })
                    .flatten(),
                allow_other: c.allow_other,
            },
        ));
        match (&self.spec.get(ty).kind, c.kind) {
            (Kind::Array { .. }, _) => match c.height {
                // In its height, scrolling.
                Some(height) => {
                    egui::ScrollArea::vertical()
                        .id_salt(("form_repeater_height", path.to_string()))
                        .max_height(height as f32)
                        .min_scrolled_height(height as f32)
                        .auto_shrink([false, true])
                        .show(ui, |ui| self.repeater_of(ui, &path, ty, &label, Some(c)));
                }
                None => self.repeater_of(ui, &path, ty, &label, Some(c)),
            },
            (Kind::Map { .. }, _) => self.map(ui, &path, ty, &label),
            (Kind::Object { .. }, _) => self.object(ui, &path, ty, &label, resolved.required),
            _ => self.field(ui, &path, ty, &label, resolved.required),
        }
        self.look = None;
    }

    /// Stands in for a control whose path the schema does not have.
    fn placeholder(&mut self, ui: &mut egui::Ui, c: &Control, problem: &str) {
        let what = c
            .path
            .clone()
            .filter(|p| !p.is_empty())
            .unwrap_or_else(|| c.kind.label().to_lowercase());
        let color = error_color(ui);
        ui.label(RichText::new(format!("{what}: {problem}")).color(color));
    }

    /// The control for the node at `path` of type `ty`: a group, a repeater,
    /// a list of entries, or a field.
    fn node(&mut self, ui: &mut egui::Ui, path: &Path, ty: TypeId, label: &str, required: bool) {
        match &self.spec.get(ty).kind {
            Kind::Object { .. } => self.object(ui, path, ty, label, required),
            Kind::Array { .. } => self.repeater(ui, path, ty, label),
            Kind::Map { .. } => self.map(ui, path, ty, label),
            _ => self.field(ui, path, ty, label, required),
        }
    }

    /// The help text of a node: what its control says, or else the schema.
    fn help(&self, path: &Path, ty: TypeId) -> Option<&str> {
        self.look
            .as_ref()
            .filter(|(p, _)| p == path)
            .and_then(|(_, look)| look.help.as_deref())
            .or(self.spec.get(ty).description.as_deref())
    }

    fn object(&mut self, ui: &mut egui::Ui, path: &Path, ty: TypeId, label: &str, required: bool) {
        let Kind::Object { fields } = &self.spec.get(ty).kind else {
            return;
        };
        let root = path.is_root();
        if self.data.get(path).is_none() {
            // Absent: an empty control, and the object is made on request.
            ui.horizontal(|ui| {
                let width = self.label_width.map_or(LABEL_WIDTH, |w| w as f32);
                label_cell(ui, label, required, self.help(path, ty), width);
                let can = self.app.can_edit(path);
                if ui.add_enabled(can, egui::Button::new("Add")).clicked() {
                    self.actions.push(FormAction::Create(path.clone()));
                }
            });
            return;
        }
        let draw = |this: &mut Self, ui: &mut egui::Ui| {
            for field in fields {
                let child = path.key(field.name.as_str());
                this.node(ui, &child, field.ty, &field.name, field.required);
            }
        };
        if root {
            let response = ui.heading(label);
            self.arrive(path, &response);
            if let Some(help) = self.help(path, ty) {
                ui.label(RichText::new(help).weak());
            }
            ui.add_space(4.0);
            draw(self, ui);
        } else {
            ui.group(|ui| {
                ui.set_width(ui.available_width());
                let response = ui.strong(label);
                self.arrive(path, &response);
                if let Some(help) = self.help(path, ty) {
                    ui.label(RichText::new(help).weak().small());
                }
                draw(self, ui);
            });
        }
    }

    // ------------------------------------------------------------- fields

    fn field(&mut self, ui: &mut egui::Ui, path: &Path, ty: TypeId, label: &str, required: bool) {
        let (position, width) = match self.look.as_ref().filter(|(p, _)| p == path) {
            Some((_, look)) => (
                look.label_position.unwrap_or(LabelPosition::Left),
                look.label_width.unwrap_or(LABEL_WIDTH),
            ),
            None => (
                LabelPosition::Left,
                self.label_width.map_or(LABEL_WIDTH, |w| w as f32),
            ),
        };
        let help = self.help(path, ty).map(str::to_owned);
        let indent = match position {
            LabelPosition::Left => width + ui.spacing().item_spacing.x,
            _ => 0.0,
        };
        if position == LabelPosition::Top {
            label_cell(ui, label, required, help.as_deref(), ui.available_width());
        }
        let problem = ui
            .horizontal(|ui| {
                if position == LabelPosition::Left {
                    label_cell(ui, label, required, help.as_deref(), width);
                }
                self.editor(ui, path, ty, required, false)
            })
            .inner;
        if let Some(problem) = problem {
            ui.horizontal(|ui| {
                ui.add_space(indent);
                ui.label(RichText::new(problem).small().color(error_color(ui)));
            });
        }
    }

    /// The editor of a scalar, with the marks a cell has: the tint and the
    /// marker of a computed value, the outline of data that does not fit, and
    /// the marker of a calculation that is waiting. Returns what is wrong.
    fn editor(
        &mut self,
        ui: &mut egui::Ui,
        path: &Path,
        ty: TypeId,
        required: bool,
        compact: bool,
    ) -> Option<String> {
        let app = self.app;
        let calc = app.calc();
        let cell = calc.cell(&app.sheet, path);
        let formula = cell.as_ref().and_then(|c| c.formula);
        let computed_by = cell.as_ref().and_then(|c| c.computed_by.clone());
        let pending = cell.as_ref().is_some_and(|c| c.pending);
        let untrusted = cell.as_ref().is_some_and(|c| c.untrusted);
        let stale = cell.as_ref().is_some_and(|c| c.stale);
        let computed = formula.is_some() || computed_by.is_some();
        let read_only = computed || !app.can_edit(path);
        let node = self.data.get(path);
        let value = node.map(Node::value);
        let kind = &self.spec.get(ty).kind;
        let name = app.name_of(path);
        let key = self.key(path);
        let mut look = self
            .look
            .as_ref()
            .filter(|(p, _)| p == path)
            .map(|(_, look)| look.clone())
            .unwrap_or_default();
        // Without a list of its own, a field whose type names values (the
        // condition names of a COBOL item) offers those. Whether it takes
        // other codes is the control's *Allow other values*, which the
        // generated form turns on for it (owner, 2026-11-01).
        let named = &self.spec.get(ty).choices;
        if look.choices.is_none() && !named.is_empty() {
            look.choices = Some(ChoiceList {
                items: Ok(named
                    .iter()
                    .map(|(value, name)| Choice {
                        code: crate::rows::display(value),
                        text: name.clone(),
                    })
                    .collect()),
                pending: false,
            });
        }
        let read_only = read_only || look.read_only;
        let as_text = matches!(look.editor, Some(Editor::Text | Editor::Multiline));
        // A list of choices, unless the field is to be text, or the list
        // cannot be had: then the field is a text box of the code, and says
        // why on hover. On an enumeration its values stay the rule.
        let mut choice_why = None;
        let choices = match &look.choices {
            Some(list) if !as_text => match &list.items {
                Ok(items) => {
                    let mut items = items.clone();
                    if let Kind::Enum { values, .. } = kind {
                        items.retain(|item| values.contains(&item.code));
                    }
                    Some((items, look.editor == Some(Editor::Typeahead)))
                }
                Err(why) => {
                    choice_why = Some(why.clone());
                    None
                }
            },
            _ => None,
        };
        let pending = pending || look.choices.as_ref().is_some_and(|l| l.pending);
        let mut not_listed = false;

        let mut problem = None;
        let mut response = None;
        // How the arrow keys leave the editor: (a text box, Up and Down).
        let mut nav: Option<(bool, bool)> = None;
        // A value that is out of date is grey, as in the grid.
        let text_colour = ui.visuals().override_text_color;
        if stale {
            ui.visuals_mut().override_text_color = Some(crate::style::Palette::of_ui(ui).stale);
        }
        match (value, kind) {
            (Some(Value::Error(error)), _) => {
                let r = ui.label(
                    RichText::new(error.kind.code())
                        .color(error_color(ui))
                        .strong(),
                );
                r.on_hover_text(format!("{} {}", error.kind.code(), error.message));
            }
            (value, _)
                if choices.is_some()
                    && !matches!(value, Some(Value::Array(_) | Value::Object(_))) =>
            {
                let (items, typeahead) = choices.as_ref().expect("matched");
                let current = value.map(value_text).unwrap_or_default();
                let field = ChoiceField {
                    path,
                    key: &key,
                    name: &name,
                    current: &current,
                    items,
                    required,
                    read_only,
                    allow_other: look.allow_other,
                    width: if compact {
                        None
                    } else if self.sized {
                        Some(ui.available_width() - if computed || pending { 24.0 } else { 0.0 })
                    } else {
                        Some(ui.available_width().min(360.0))
                    },
                };
                let (r, listed) = if *typeahead {
                    nav = Some((true, false));
                    self.typeahead(ui, &field)
                } else {
                    nav = Some((false, true));
                    self.dropdown(ui, &field)
                };
                not_listed = !listed;
                response = Some(r);
            }
            (_, Kind::Bool) if !as_text => {
                let mut checked = matches!(value, Some(Value::Bool(true)));
                let r = ui.add_enabled(!read_only, egui::Checkbox::new(&mut checked, ""));
                r.widget_info(|| {
                    egui::WidgetInfo::selected(
                        egui::Role::CheckBox,
                        !read_only,
                        checked,
                        name.clone(),
                    )
                });
                if r.changed() {
                    self.actions
                        .push(FormAction::Direct(path.clone(), checked.to_string()));
                }
                nav = Some((false, true));
                response = Some(r);
            }
            (_, Kind::Enum { values, .. }) if !as_text => {
                let current = match value {
                    Some(Value::Str(s)) => s.clone(),
                    _ => String::new(),
                };
                let mut chosen = current.clone();
                let r = ui
                    .add_enabled_ui(!read_only, |ui| {
                        egui::ComboBox::from_id_salt(("form_enum", &app.sheet, path.to_string()))
                            .selected_text(&current)
                            .show_ui(ui, |ui| {
                                if !required {
                                    ui.selectable_value(&mut chosen, String::new(), "(none)");
                                }
                                for option in values {
                                    ui.selectable_value(&mut chosen, option.clone(), option);
                                }
                            })
                            .response
                    })
                    .inner;
                r.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::Role::ComboBox, !read_only, name.clone())
                });
                if chosen != current {
                    self.actions.push(FormAction::Direct(path.clone(), chosen));
                }
                nav = Some((false, true));
                response = Some(r);
            }
            (Some(Value::Array(_) | Value::Object(_)), _) => {
                ui.label(
                    RichText::new(display(value.expect("matched")))
                        .weak()
                        .italics(),
                );
            }
            _ => {
                let current = value.map(value_text).unwrap_or_default();
                // A number or date format shows while the field does not have
                // the focus; typing is into the value as it is stored.
                let id = ui.make_persistent_id(("form_text", path.to_string()));
                let focused = ui.memory(|m| m.has_focus(id));
                let mut text = match self.drafts.get(&key) {
                    Some(draft) => draft.text.clone(),
                    None if !focused => value
                        .and_then(|v| look.format.apply(v))
                        .unwrap_or_else(|| current.clone()),
                    None => current.clone(),
                };
                // A masked string is one line of dots, whatever the editor.
                let masked = look.masked && matches!(kind, Kind::String(_));
                let long = !masked
                    && match look.editor {
                        Some(Editor::Multiline) => {
                            matches!(kind, Kind::String(_) | Kind::Enum { .. })
                        }
                        Some(Editor::Text) => false,
                        _ => {
                            matches!(kind, Kind::String(c) if c.max_length.is_some_and(|m| m > 200))
                        }
                    };
                // A date gets a button that opens a calendar.
                let date = !read_only && !long && !masked && date_kind(kind).is_some();
                // In a control of a set width, what is left after the markers
                // that follow it.
                let markers = if computed || pending { 24.0 } else { 0.0 }
                    + if date { 28.0 } else { 0.0 }
                    + if untrusted && formula.is_some() {
                        80.0
                    } else {
                        0.0
                    };
                let width = if compact {
                    f32::INFINITY
                } else if self.sized {
                    ui.available_width() - markers
                } else {
                    ui.available_width().min(360.0)
                };
                let edit = if long {
                    let edit = egui::TextEdit::multiline(&mut text).desired_rows(3);
                    match look.height {
                        Some(height) => edit.min_size(egui::vec2(0.0, height)),
                        None => edit,
                    }
                } else {
                    egui::TextEdit::singleline(&mut text).password(masked)
                };
                // Numbers line up on the right, unless the column says.
                let align = match look.align {
                    Some(Alignment::Left) => egui::Align2::LEFT_CENTER,
                    Some(Alignment::Center) => egui::Align2::CENTER_CENTER,
                    Some(Alignment::Right) => egui::Align2::RIGHT_CENTER,
                    None if matches!(kind, Kind::Integer(_) | Kind::Number(_)) => {
                        egui::Align2::RIGHT_CENTER
                    }
                    None => egui::Align2::LEFT_CENTER,
                };
                let r = ui.add(
                    edit.id(id)
                        .desired_width(width)
                        .align(align)
                        .hint_text(hint(kind))
                        .interactive(!read_only),
                );
                let label = name.clone();
                r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, &label));
                if r.changed() {
                    let error = check_text(self.spec, path, &text);
                    self.drafts.insert(key.clone(), Draft { text, error });
                }
                if r.gained_focus() {
                    self.actions.push(FormAction::Select(path.clone()));
                }
                if r.has_focus() && ui.input(|i| i.key_pressed(Key::Escape)) {
                    self.drafts.remove(&key);
                    r.surrender_focus();
                }
                if r.lost_focus()
                    && let Some(draft) = self.drafts.get(&key)
                {
                    if draft.text == current {
                        self.drafts.remove(&key);
                    } else if draft.error.is_none() {
                        self.actions
                            .push(FormAction::Commit(path.clone(), draft.text.clone()));
                    }
                }
                if date {
                    let typed = self
                        .drafts
                        .get(&key)
                        .map_or_else(|| current.clone(), |d| d.text.clone());
                    self.date_button(ui, path, &name, &typed, kind);
                }
                nav = Some((true, !long));
                response = Some(r);
            }
        }

        if let Some(draft) = self.drafts.get(&key) {
            problem = draft.error.clone();
        } else if let (Some(node), false) = (node, matches!(value, Some(Value::Error(_)))) {
            problem = validate_at(self.spec, ty, node, path)
                .into_iter()
                .next()
                .map(|issue| issue.message);
        } else if node.is_none() && required && !computed {
            problem = Some("required".to_owned());
        }

        ui.visuals_mut().override_text_color = text_colour;
        // A read-only text box is drawn enabled but cannot take the focus, so
        // the arrow keys pass it by, as Tab does.
        if let (Some(r), Some((text, vertical))) = (&response, nav)
            && r.enabled()
            && !read_only
        {
            self.nav.push(crate::form_nav::NavStop {
                id: r.id,
                rect: r.rect,
                text,
                vertical,
            });
        }
        if let Some(r) = &response {
            self.arrive(path, r);
            if computed {
                ui.painter().rect_filled(r.rect, 2.0, computed_fill(ui));
            }
            if problem.is_some() {
                ui.painter().rect_stroke(
                    r.rect.expand(1.0),
                    2.0,
                    Stroke::new(1.0, error_color(ui)),
                    StrokeKind::Outside,
                );
            } else if app.selection.as_ref() == Some(path) {
                ui.painter().rect_stroke(
                    r.rect.expand(1.0),
                    2.0,
                    Stroke::new(1.0, ui.visuals().selection.stroke.color),
                    StrokeKind::Outside,
                );
            }
            if let Some(message) = &problem {
                r.clone().on_hover_text(message);
            }
            if let Some(why) = &choice_why {
                r.clone().on_hover_text(why);
            }
            if not_listed {
                r.clone()
                    .on_hover_text("This value is not in the list of choices.");
            }
            // The node's comments, as on its cell: the triangle and the
            // popup, and outside layout mode the comment commands in the
            // field's menu.
            app.comment_marks(ui, r.rect, path);
            if self.layout.is_none() && calc.format_has_comments(&app.sheet) {
                let mut commands = Vec::new();
                // A menu of its own id: egui's default is the one a dropdown's
                // list uses, and a click, which closes the menu, would close
                // the list as soon as it opens.
                egui::Popup::context_menu(r)
                    .id(r.id.with("comment_menu"))
                    .show(|ui| {
                        app.comment_menu_items(ui, path, r.rect.right_top(), &mut commands, false);
                    });
                self.actions
                    .extend(commands.into_iter().map(FormAction::Table));
            }
        }
        if not_listed && !compact {
            ui.label(RichText::new("not in the list").small().weak().italics());
        }
        if let Some(text) = formula {
            ui.label(RichText::new("fx").italics().weak())
                .on_hover_text(format!("={text}"));
        } else if let Some(by) = &computed_by {
            ui.label(RichText::new("fx").italics().weak())
                .on_hover_text(format!("computed by {}", app.name_of(by)));
        }
        if untrusted && formula.is_some() {
            crate::view::untrusted_mark(ui, stale);
        }
        if pending {
            crate::style::pending_marker(ui).on_hover_text("Waiting to be calculated");
        }
        problem
    }

    /// The button beside a date that opens a calendar. A day picked there is
    /// entered at once; a date and time keeps its time.
    fn date_button(&mut self, ui: &mut egui::Ui, path: &Path, name: &str, text: &str, kind: &Kind) {
        let Some(with_time) = date_kind(kind) else {
            return;
        };
        let typed = text.trim();
        let shown = typed
            .get(..10)
            .and_then(|d| d.parse::<jiff::civil::Date>().ok());
        let mut day = shown.unwrap_or_else(today);
        let before = day;
        let salt = format!("form_date {}", path);
        let r = ui.add(
            egui_extras::DatePickerButton::new(&mut day)
                .id_salt(&salt)
                // The button shows a calendar from the icon font, which
                // follows every text font; its own icon is an emoji the
                // bundled fonts lack.
                .format(CALENDAR)
                .show_icon(false)
                .combo_boxes(false)
                .calendar_week(false)
                .highlight_weekends(false),
        );
        let label = format!("Pick a date for {name}");
        r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
        let r = r.on_hover_text("Pick a date");
        if day != before || (shown.is_none() && r.changed()) {
            let date = day.strftime("%Y-%m-%d").to_string();
            let entered = if with_time {
                let rest = typed.get(10..).filter(|t| t.starts_with('T'));
                format!("{date}{}", rest.unwrap_or("T00:00:00"))
            } else {
                date
            };
            self.drafts.remove(&self.key(path));
            self.actions.push(FormAction::Direct(path.clone(), entered));
        }
    }

    /// A dropdown of the choices of a field: it shows the text of the item
    /// whose code is stored, and enters the code of the item chosen. Returns
    /// its response, and whether what is stored is in the list (nothing
    /// stored is).
    fn dropdown(&mut self, ui: &mut egui::Ui, f: &ChoiceField) -> (egui::Response, bool) {
        let selected = position_of(f.items, f.current);
        let listed = selected.is_some() || f.current.is_empty();
        // With *allow other values*, Other... turns the field into a text
        // box for one code, until it loses the focus.
        let write_in =
            ui.make_persistent_id(("form_write_in", &self.app.sheet, f.path.to_string()));
        if f.allow_other
            && !f.read_only
            && ui.data(|d| d.get_temp::<bool>(write_in)).unwrap_or(false)
        {
            return (self.write_in(ui, f, write_in), listed);
        }
        let shown = match selected {
            Some(i) => choice_label(&f.items[i]),
            None if f.current.is_empty() => String::new(),
            None => format!("{} (not in the list)", f.current),
        };
        let mut picked = None;
        let mut other = false;
        let r = ui
            .add_enabled_ui(!f.read_only, |ui| {
                let combo = egui::ComboBox::from_id_salt((
                    "form_choice",
                    &self.app.sheet,
                    f.path.to_string(),
                ))
                .selected_text(shown);
                let combo = match f.width {
                    Some(width) if self.sized => combo.width(width),
                    _ => combo,
                };
                combo
                    .show_ui(ui, |ui| {
                        if !f.required
                            && ui
                                .selectable_label(f.current.is_empty(), "(none)")
                                .clicked()
                        {
                            picked = Some(String::new());
                        }
                        for (i, item) in f.items.iter().enumerate() {
                            if ui
                                .selectable_label(selected == Some(i), choice_label(item))
                                .clicked()
                            {
                                picked = Some(item.code.clone());
                            }
                        }
                        if f.allow_other
                            && ui
                                .selectable_label(!listed, "Other...")
                                .on_hover_text("Type a code that is not in the list")
                                .clicked()
                        {
                            other = true;
                        }
                    })
                    .response
            })
            .inner;
        r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, !f.read_only, f.name));
        if other {
            ui.data_mut(|d| d.insert_temp(write_in, true));
            ui.ctx().request_repaint();
        }
        if let Some(code) = picked
            && !same_code(&code, f.current)
        {
            self.actions.push(FormAction::Commit(f.path.clone(), code));
        }
        (r, listed)
    }

    /// The text box a dropdown becomes after Other...: it starts on the code
    /// stored, and leaving it enters what was typed (read as the field's
    /// type, as typing in a cell is), unless Escape gives it up.
    fn write_in(&mut self, ui: &mut egui::Ui, f: &ChoiceField, flag: egui::Id) -> egui::Response {
        let text_id = flag.with("text");
        let started = ui.data(|d| d.get_temp::<String>(text_id));
        let mut text = started.clone().unwrap_or_else(|| f.current.to_owned());
        let r = ui.add(
            egui::TextEdit::singleline(&mut text)
                .id(text_id)
                .desired_width(f.width.unwrap_or(f32::INFINITY))
                .hint_text("type a code"),
        );
        r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, f.name));
        if started.is_none() {
            r.request_focus();
        }
        ui.data_mut(|d| d.insert_temp(text_id, text.clone()));
        let escape = ui.input(|i| i.key_pressed(Key::Escape));
        if r.lost_focus() || (started.is_some() && !r.has_focus()) {
            ui.data_mut(|d| {
                d.remove::<bool>(flag);
                d.remove::<String>(text_id);
            });
            if !escape && !same_code(&text, f.current) {
                self.actions
                    .push(FormAction::Commit(f.path.clone(), text.trim().to_owned()));
            }
        }
        r
    }

    /// A typeahead of the choices of a field: a text box that shows the text
    /// of the item whose code is stored and, while it has the focus, lists
    /// the items whose text contains what is typed, best first. The arrows
    /// move in the list, Enter or a click takes an item; leaving the box
    /// takes the item whose text (or code) is what was typed, or with *allow
    /// other values* the text itself, and otherwise refuses it with the
    /// reason. Returns its response, and whether what is stored is listed.
    fn typeahead(&mut self, ui: &mut egui::Ui, f: &ChoiceField) -> (egui::Response, bool) {
        let selected = position_of(f.items, f.current);
        let listed = selected.is_some() || f.current.is_empty();
        let shown = selected.map_or_else(|| f.current.to_owned(), |i| f.items[i].text.clone());
        let id = ui.make_persistent_id(("form_typeahead", f.path.to_string()));
        let highlight_id = id.with("highlight");
        let mut text = match self.drafts.get(f.key) {
            Some(draft) => draft.text.clone(),
            None => shown.clone(),
        };
        let r = ui.add(
            egui::TextEdit::singleline(&mut text)
                .id(id)
                .desired_width(f.width.unwrap_or(f32::INFINITY))
                .hint_text("type to search")
                .interactive(!f.read_only),
        );
        r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, f.name));
        if r.changed() {
            self.drafts.insert(
                f.key.clone(),
                Draft {
                    text: text.clone(),
                    error: None,
                },
            );
            ui.data_mut(|d| d.insert_temp(highlight_id, 0usize));
        }
        if r.gained_focus() {
            self.actions.push(FormAction::Select(f.path.clone()));
        }
        let typed = self.drafts.get(f.key).map(|d| d.text.clone());
        let matches = match &typed {
            Some(typed) => crate::form_exprs::matching(f.items, typed),
            None => (0..f.items.len()).collect(),
        };
        let mut highlight = ui
            .data(|d| d.get_temp::<usize>(highlight_id))
            .unwrap_or(0)
            .min(matches.len().saturating_sub(1));
        let mut picked: Option<String> = None;
        if r.has_focus() {
            let (down, up, escape) = ui.input(|i| {
                (
                    i.key_pressed(Key::ArrowDown),
                    i.key_pressed(Key::ArrowUp),
                    i.key_pressed(Key::Escape),
                )
            });
            if down {
                highlight = (highlight + 1).min(matches.len().saturating_sub(1));
            }
            if up {
                highlight = highlight.saturating_sub(1);
            }
            ui.data_mut(|d| d.insert_temp(highlight_id, highlight));
            if escape {
                self.drafts.remove(f.key);
                r.surrender_focus();
            } else if !matches.is_empty() {
                egui::Area::new(id.with("list"))
                    .order(egui::Order::Foreground)
                    .fixed_pos(r.rect.left_bottom() + egui::vec2(0.0, 2.0))
                    .show(ui.ctx(), |ui| {
                        egui::Frame::popup(ui.style()).show(ui, |ui| {
                            ui.set_min_width(r.rect.width() - 12.0);
                            egui::ScrollArea::vertical()
                                .max_height(240.0)
                                .show(ui, |ui| {
                                    for (n, &i) in matches.iter().enumerate() {
                                        let item =
                                            ui.selectable_label(n == highlight, &f.items[i].text);
                                        if n == highlight && (down || up) {
                                            item.scroll_to_me(None);
                                        }
                                        // Taken on the press: the box loses the
                                        // focus when the click ends.
                                        if item.is_pointer_button_down_on() {
                                            picked = Some(f.items[i].code.clone());
                                        }
                                    }
                                });
                        });
                    });
            }
        }
        if r.lost_focus()
            && picked.is_none()
            && let Some(typed) = typed
        {
            let enter = ui.input(|i| i.key_pressed(Key::Enter));
            if enter && let Some(&i) = matches.get(highlight) {
                picked = Some(f.items[i].code.clone());
            } else if typed.trim().is_empty() {
                picked = Some(String::new());
            } else if let Some(i) = crate::form_exprs::exact(f.items, &typed) {
                picked = Some(f.items[i].code.clone());
            } else if f.allow_other {
                picked = Some(typed.trim().to_owned());
            } else {
                let error = format!("\u{201c}{}\u{201d} is not one of the choices", typed.trim());
                self.drafts.insert(
                    f.key.clone(),
                    Draft {
                        text: typed,
                        error: Some(error),
                    },
                );
            }
        }
        if let Some(code) = picked {
            self.drafts.remove(f.key);
            ui.data_mut(|d| d.remove::<usize>(highlight_id));
            if !same_code(&code, f.current) {
                self.actions.push(FormAction::Commit(f.path.clone(), code));
            }
            r.surrender_focus();
        }
        (r, listed)
    }

    // ----------------------------------------------------------- repeaters

    fn repeater(&mut self, ui: &mut egui::Ui, path: &Path, ty: TypeId, label: &str) {
        self.repeater_of(ui, path, ty, label, None);
    }

    /// An array as a table or one item at a time, as its control says; with no
    /// say, a table when the items are flat.
    fn repeater_of(
        &mut self,
        ui: &mut egui::Ui,
        path: &Path,
        ty: TypeId,
        label: &str,
        control: Option<&Control>,
    ) {
        let Kind::Array { items: item_ty, .. } = &self.spec.get(ty).kind else {
            return;
        };
        let item_ty = *item_ty;
        let node = self.data.get(path);
        let items: &[Node] = node.and_then(|n| n.value().as_array()).unwrap_or(&[]);
        let can_add = self.app.can_reorder(path);
        let columns = match control.and_then(|c| c.mode) {
            Some(RepeaterMode::Stacked) => None,
            _ => item_columns(self.spec, item_ty).map(|columns| arranged(control, columns)),
        };
        let shown = columns.as_ref().map(|_| self.table_shown(path, items));
        ui.group(|ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                let title_font =
                    control.and_then(|c| own_font(c.title_font, c.title_font_size, self.font));
                let response = in_font(ui, title_font, |ui| ui.strong(label));
                self.arrive(path, &response);
                ui.label(RichText::new(count_text(items.len())).weak());
                if ui.add_enabled(can_add, egui::Button::new("New")).clicked() {
                    self.actions.push(FormAction::Add(path.clone()));
                }
                if let Some(shown) = &shown {
                    self.table_strip(ui, path, shown);
                }
            });
            if let Some(help) = self.help(path, ty) {
                ui.label(RichText::new(help).weak().small());
            }
            match columns {
                // In layout mode an empty table still shows its columns.
                Some(columns) if !items.is_empty() || self.layout.is_some() => {
                    let header_font = control
                        .and_then(|c| own_font(c.header_font, c.header_font_size, self.font));
                    // A table wider than the form scrolls sideways on its own,
                    // as the tabular pane does.
                    egui::ScrollArea::horizontal()
                        .id_salt(("form_table_scroll", path.to_string()))
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            let at = control.map(|_| self.at.clone());
                            let shown = shown.unwrap_or_default();
                            let table = FormTable {
                                array: path,
                                len: items.len(),
                                columns: &columns,
                                shown: &shown,
                                at,
                                header_font,
                            };
                            self.table(ui, table);
                        });
                }
                Some(_) => {}
                None => self.stacked(ui, path, item_ty, items.len(), control),
            }
        });
    }

    /// A table of the items of an array: its columns that are shown, in their
    /// order, each with its own properties. `at` is the address of the
    /// control the table is of: its columns can be selected in layout mode,
    /// and out of it their expressions hide a column or colour a cell.
    fn table(&mut self, ui: &mut egui::Ui, table: FormTable<'_>) {
        let FormTable {
            array,
            len,
            columns,
            shown: view,
            at,
            header_font,
        } = table;
        let designed = at.is_some();
        // The expressions of the columns, out of layout mode: a column whose
        // *visible when* is false is left out; what the others say about the
        // column as a whole goes to each of its cells.
        let gated = at.clone().filter(|_| self.layout.is_none());
        let mut shown = Vec::new();
        let mut heads = Vec::new();
        for (design, column) in columns {
            let head = match (&gated, column) {
                (Some(at), Some(_)) => crate::form_exprs::column_gate(
                    self.app.calc(),
                    &self.app.sheet,
                    self.form,
                    at,
                    &design.name,
                    None,
                ),
                _ => Gate::default(),
            };
            let by_hand = column.is_some() && view.hidden_columns.contains(&design.name);
            if !head.hidden && !by_hand {
                shown.push((design.clone(), column.clone()));
                heads.push(head);
            }
        }
        let columns = &shown[..];
        // Out of layout mode a column's title has the commands of the
        // tabular view: filter, hide.
        let menus = self.layout.is_none();
        let filterable: Vec<String> = self
            .app
            .columns_of(array)
            .into_iter()
            .filter(|c| !c.summary)
            .map(|c| c.name)
            .collect();
        let filtered: Vec<bool> = columns
            .iter()
            .map(|(d, _)| self.app.filter_of(array, &d.name).is_some())
            .collect();
        let mut command: Option<crate::view::Action> = None;
        let rows = &view.visible.rows;
        let editable = self.app.can_reorder(array);
        let sortable = editable
            && columns
                .iter()
                .any(|(_, c)| c.as_ref().is_some_and(|c| !c.fields.is_empty()));
        let inherited = self.font;
        let fonts: Vec<Option<ColumnFont>> = columns
            .iter()
            .map(|(d, _)| column_font(d, inherited))
            .collect();
        let mut row_height = crate::style::row_height(ui);
        for (typeface, size) in fonts.iter().flatten() {
            row_height = row_height.max(crate::style::row_height_in(ui, *typeface, *size));
        }
        let header_height = match header_font {
            Some((typeface, size)) => crate::style::row_height_in(ui, typeface, size),
            None => crate::style::row_height(ui),
        };
        let header_height = (header_height - 6.0).max(22.0);
        // Every column can be made wider or narrower by dragging the edge of
        // its title. The widths dragged to are kept with the table's state,
        // which starts again when the design's widths change.
        let design_widths: Vec<Option<u32>> = columns.iter().map(|(d, _)| d.width).collect();
        let mut table = TableBuilder::new(ui)
            .id_salt((
                "form_table",
                &self.app.sheet,
                array.to_string(),
                design_widths,
            ))
            .striped(true)
            .vscroll(false)
            .sense(Sense::click())
            .cell_layout(egui::Layout::left_to_right(Align::Center));
        for (design, column) in columns {
            let shape = match (design.width, column) {
                (Some(width), _) => TableColumn::initial(width as f32).at_least(24.0).clip(true),
                (None, None) => TableColumn::auto().at_least(32.0),
                (None, Some(_)) => TableColumn::initial(140.0).at_least(60.0).clip(true),
            };
            table = table.column(shape.resizable(true));
        }
        // Each column's own place in the header row, without a wider title.
        let mut drawn: Vec<(String, egui::Rect)> = Vec::new();
        let mut sort = None;
        let mut titles = Vec::new();
        table
            .header(header_height, |mut header| {
                for ((design, column), &filtered) in columns.iter().zip(&filtered) {
                    let mut max = egui::Rect::NOTHING;
                    let (used, _) = header.col(|ui| {
                        max = ui.max_rect();
                        in_font(ui, header_font, |ui| {
                            aligned(ui, design.title_align, |ui| {
                                let title =
                                    design.label.clone().unwrap_or_else(|| design.name.clone());
                                let text = RichText::new(title).strong();
                                let sorts = sortable && column.is_some();
                                let menu = menus && column.is_some();
                                let mut response = if sorts || menu {
                                    ui.add(egui::Label::new(text).sense(Sense::CLICK))
                                } else {
                                    ui.label(text)
                                };
                                if filtered {
                                    crate::table_view::filter_mark(ui);
                                }
                                let mut hover: Vec<String> = design.help.iter().cloned().collect();
                                if sorts {
                                    hover.push("Click to sort by this column".to_owned());
                                }
                                if menu {
                                    hover.push("Right-click to filter or hide it".to_owned());
                                }
                                if !hover.is_empty() {
                                    response = response.on_hover_text(hover.join("\n"));
                                }
                                if sorts && response.clicked() {
                                    sort = Some(design.name.clone());
                                }
                                if menu {
                                    let can_filter = filterable.contains(&design.name);
                                    response.context_menu(|ui| {
                                        command = column_menu(
                                            ui,
                                            array,
                                            &design.name,
                                            (sorts, can_filter, filtered),
                                        )
                                        .or(command.take());
                                    });
                                }
                            });
                        });
                    });
                    // The column, or its title where that is wider. (The response
                    // of the cell reaches past the column, by the spacing.)
                    titles.push((design.name.clone(), max.union(used)));
                    drawn.push((design.name.clone(), max));
                }
            })
            .body(|body| {
                body.rows(row_height, rows.len(), |mut row| {
                    let index = rows[row.index()];
                    for (((design, column), font), head) in columns.iter().zip(&fonts).zip(&heads) {
                        row.col(|ui| {
                            in_font(ui, *font, |ui| {
                                aligned(ui, design.align, |ui| match column {
                                    None => {
                                        let label = ui.add(
                                            egui::Label::new(
                                                RichText::new(design.row_label(index)).weak(),
                                            )
                                            .sense(Sense::click()),
                                        );
                                        label.context_menu(|ui| {
                                            self.row_menu(ui, array, index, len, editable);
                                        });
                                    }
                                    Some(column) => {
                                        let cell = column.cell_path(array, index);
                                        let choices = at
                                            .as_ref()
                                            .filter(|_| has_text(&design.choices))
                                            .and_then(|at| {
                                                choice_list(
                                                    self.app.calc(),
                                                    &self.app.sheet,
                                                    self.form,
                                                    at,
                                                    Some((&design.name, index)),
                                                    None,
                                                )
                                            });
                                        let look = Look {
                                            help: None,
                                            editor: design.editor,
                                            read_only: design.read_only,
                                            masked: false,
                                            align: design.align,
                                            format: design.format(),
                                            choices,
                                            allow_other: design.allow_other,
                                            ..Look::default()
                                        };
                                        let outer = self.look.replace((cell.clone(), look));
                                        let gate = match &gated {
                                            Some(at) => cell_gate(
                                                self.app.calc(),
                                                &self.app.sheet,
                                                self.form,
                                                at,
                                                &design.name,
                                                index,
                                                head,
                                            ),
                                            None => Gate::default(),
                                        };
                                        if gate == Gate::default() {
                                            self.editor(
                                                ui,
                                                &cell,
                                                column.ty,
                                                column.required,
                                                true,
                                            );
                                        } else {
                                            self.styled(ui, &gate, true, |this, ui| {
                                                this.editor(
                                                    ui,
                                                    &cell,
                                                    column.ty,
                                                    column.required,
                                                    true,
                                                );
                                            });
                                        }
                                        self.look = outer;
                                    }
                                });
                            });
                        });
                    }
                    // A right click on the row opens its menu.
                    row.response().context_menu(|ui| {
                        self.row_menu(ui, array, index, len, editable);
                    });
                });
            });
        if let Some(name) = sort {
            self.actions.push(FormAction::Sort(array.clone(), name));
        }
        if let Some(command) = command {
            self.actions.push(FormAction::Table(command));
        }
        if designed && self.layout.is_some() {
            self.column_targets = titles
                .into_iter()
                .zip(drawn)
                .map(|((name, title), (_, column))| (name, title, column))
                .collect();
        }
    }

    /// The commands of a table row, in its context menu.
    fn row_menu(
        &mut self,
        ui: &mut egui::Ui,
        array: &Path,
        index: usize,
        len: usize,
        editable: bool,
    ) {
        if ui
            .add_enabled(editable && index > 0, egui::Button::new("Move up"))
            .clicked()
        {
            self.actions
                .push(FormAction::Move(array.clone(), index, true));
            ui.close();
        }
        if ui
            .add_enabled(editable && index + 1 < len, egui::Button::new("Move down"))
            .clicked()
        {
            self.actions
                .push(FormAction::Move(array.clone(), index, false));
            ui.close();
        }
        if ui
            .add_enabled(editable, egui::Button::new("Delete row"))
            .clicked()
        {
            self.actions.push(FormAction::Delete(array.clone(), index));
            ui.close();
        }
        if self.layout.is_none() {
            ui.separator();
            if ui.button("Hide row").clicked() {
                self.actions
                    .push(FormAction::Table(crate::view::Action::HideRow(
                        array.clone(),
                        index,
                    )));
                ui.close();
            }
        }
    }

    /// Which rows and columns of a table the user has left shown: the state
    /// of the tabular view of the same array. In layout mode, all of them.
    fn table_shown(&self, array: &Path, items: &[Node]) -> TableShown {
        let all = TableShown {
            visible: table::Visible {
                rows: (0..items.len()).collect(),
                total: items.len(),
                ..table::Visible::default()
            },
            ..TableShown::default()
        };
        if self.layout.is_some() {
            return all;
        }
        let Some(view) = self.app.view_of(array) else {
            return all;
        };
        let columns = self.app.columns_of(array);
        TableShown {
            visible: table::visible(self.spec, items, &columns, view),
            hidden_columns: view.hidden_columns.iter().cloned().collect(),
            hidden_rows: self.app.hidden_rows(array),
            filtered: !view.filters.is_empty(),
        }
    }

    /// Beside the title of a table: how many rows are hidden or filtered,
    /// and the commands that show them again.
    fn table_strip(&mut self, ui: &mut egui::Ui, array: &Path, shown: &TableShown) {
        use crate::view::Action;
        let visible = &shown.visible;
        if visible.hidden > 0 || visible.filtered > 0 {
            ui.label(
                RichText::new(format!(
                    "{} shown ({} hidden, {} filtered)",
                    visible.rows.len(),
                    visible.hidden,
                    visible.filtered
                ))
                .weak(),
            );
        }
        let mut command = None;
        if shown.filtered {
            if ui.button("Clear all filters").clicked() {
                command = Some(Action::ClearAllFilters(array.clone()));
            }
            if ui
                .button("Reapply filter")
                .on_hover_text("Hide the rows edited since that no longer pass the filters")
                .clicked()
            {
                command = Some(Action::ReapplyFilters(array.clone()));
            }
        }
        let hidden = shown.hidden_columns.len() + shown.hidden_rows.len();
        if hidden > 0 {
            ui.menu_button(format!("Hidden ({hidden})"), |ui| {
                for column in &shown.hidden_columns {
                    if ui.button(format!("Show column {column}")).clicked() {
                        command = Some(Action::ShowColumn(array.clone(), column.clone()));
                        ui.close();
                    }
                }
                for (index, id) in &shown.hidden_rows {
                    if ui.button(format!("Show row [{index}]")).clicked() {
                        command = Some(Action::ShowRow(array.clone(), *id));
                        ui.close();
                    }
                }
                ui.separator();
                if ui.button("Show all").clicked() {
                    command = Some(Action::ShowAllHidden(array.clone()));
                    ui.close();
                }
            });
        }
        if let Some(command) = command {
            self.actions.push(FormAction::Table(command));
        }
    }

    /// Items that are not flat, one at a time with a stepper.
    /// Items that are not flat, or a repeater set to stacked: one item at a
    /// time with a stepper. The item is drawn as the control's design of its
    /// items says, or as generated.
    fn stacked(
        &mut self,
        ui: &mut egui::Ui,
        array: &Path,
        item_ty: TypeId,
        len: usize,
        control: Option<&Control>,
    ) {
        if len == 0 {
            return;
        }
        let at = self.step_of(array, len);
        let editable = self.app.can_reorder(array);
        // Going to a node inside another item: step to that item.
        if let Some(wanted) = &self.scroll_to
            && wanted.len() > array.len()
            && wanted.segments().starts_with(array.segments())
            && let cliotree_core::Segment::Index(i) = wanted.segments()[array.len()]
            && i != at
            && i < len
        {
            self.actions.push(FormAction::Step(array.clone(), i));
        }
        ui.horizontal(|ui| {
            if icons::button(ui, at > 0, icons::CARET_LEFT, "<", "Previous item").clicked() {
                self.actions.push(FormAction::Step(array.clone(), at - 1));
            }
            ui.label(format!("{} of {len}", at + 1));
            if icons::button(ui, at + 1 < len, icons::CARET_RIGHT, ">", "Next item").clicked() {
                self.actions.push(FormAction::Step(array.clone(), at + 1));
            }
            if ui
                .add_enabled(editable, egui::Button::new("Delete"))
                .clicked()
            {
                self.actions.push(FormAction::Delete(array.clone(), at));
            }
        });
        let item = array.index(at);
        match control.filter(|c| self.designs_items(c)) {
            Some(c) => {
                let address = self.at.clone();
                let outer = self.item.replace((item, at));
                self.controls(ui, &c.children, &address);
                self.item = outer;
                self.at = address;
            }
            None => self.node(ui, &item, item_ty, &format!("Item {}", at + 1), true),
        }
    }

    /// The item a stacked repeater of `len` items shows.
    fn step_of(&self, array: &Path, len: usize) -> usize {
        self.app
            .form
            .steps
            .get(&self.key(array))
            .copied()
            .unwrap_or(0)
            .min(len.saturating_sub(1))
    }

    /// Whether a control is a stacked repeater drawn by the design of its
    /// items. One inside the items of another shows its items as generated.
    fn designs_items(&self, c: &Control) -> bool {
        c.is_stacked() && !c.children.is_empty() && self.item.is_none()
    }

    /// The node a control shows: its path, or in the design of a stacked
    /// repeater's items, its path in the item drawn (the item itself for a
    /// control that is not bound).
    fn path_of(&self, c: &Control) -> Option<Path> {
        match &self.item {
            None => c.bound_path(),
            Some((item, _)) => match c.bound_path() {
                Some(rest) => Some(item.join(&rest)),
                None => Some(item.clone()),
            },
        }
    }

    /// Why a control cannot be drawn, its path read from the item drawn when
    /// it is in the design of a stacked repeater's items.
    fn problem_of(&self, c: &Control) -> Option<String> {
        match &self.item {
            None => design_problem(self.spec, c),
            Some((item, _)) => cliotree_core::form::control_problem_at(self.spec, c, item),
        }
    }

    // ---------------------------------------------------------------- maps

    fn map(&mut self, ui: &mut egui::Ui, path: &Path, ty: TypeId, label: &str) {
        let Kind::Map { value: value_ty } = &self.spec.get(ty).kind else {
            return;
        };
        let value_ty = *value_ty;
        let entries: Vec<String> = self
            .data
            .get(path)
            .and_then(|n| n.value().as_object())
            .map(|fields| fields.keys().cloned().collect())
            .unwrap_or_default();
        let editable = self.app.can_edit(path);
        ui.group(|ui| {
            ui.set_width(ui.available_width());
            let response = ui.strong(label);
            self.arrive(path, &response);
            if let Some(help) = self.help(path, ty) {
                ui.label(RichText::new(help).weak().small());
            }
            for key in &entries {
                ui.horizontal(|ui| {
                    ui.allocate_ui_with_layout(
                        egui::vec2(LABEL_WIDTH, ROW_HEIGHT),
                        egui::Layout::left_to_right(Align::Center),
                        |ui| {
                            ui.set_min_size(egui::vec2(LABEL_WIDTH, ROW_HEIGHT));
                            ui.add(egui::Label::new(RichText::new(key).monospace()).truncate());
                        },
                    );
                    self.editor(ui, &path.key(key.as_str()), value_ty, true, false);
                    if icons::button(ui, editable, icons::X, "x", "Delete entry").clicked() {
                        self.actions
                            .push(FormAction::DeleteEntry(path.clone(), key.clone()));
                    }
                });
            }
            let map_key = self.key(path);
            let mut new_key = self.new_keys.get(&map_key).cloned().unwrap_or_default();
            ui.horizontal(|ui| {
                let r = ui.add(
                    egui::TextEdit::singleline(&mut new_key)
                        .hint_text("new key")
                        .desired_width(LABEL_WIDTH),
                );
                let name = format!("New key of {}", self.app.name_of(path));
                r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, &name));
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                let add = ui
                    .add_enabled(editable, egui::Button::new("Add entry"))
                    .clicked();
                if add || enter {
                    self.actions
                        .push(FormAction::AddEntry(path.clone(), new_key.clone()));
                }
            });
            if new_key.is_empty() {
                self.new_keys.remove(&map_key);
            } else {
                self.new_keys.insert(map_key, new_key);
            }
        });
    }
}

/// How a cell of a column is drawn: its row's expressions, with what the
/// column's *visible when* said when it could not hide the column (an error,
/// or not run yet).
fn cell_gate(
    calc: &cliotree_calc::Calc,
    tab: &str,
    form: &str,
    at: &[usize],
    column: &str,
    row: usize,
    head: &Gate,
) -> Gate {
    let mut gate = crate::form_exprs::column_gate(calc, tab, form, at, column, Some(row));
    gate.disabled |= head.disabled;
    gate.pending |= head.pending;
    gate.why.extend(head.why.iter().cloned());
    gate
}

/// Phosphor's calendar, on the button that opens one.
const CALENDAR: &str = "\u{E108}";

/// Whether a type is a date (`Some(false)`) or a date and time
/// (`Some(true)`), as a type or as a string format.
fn date_kind(kind: &Kind) -> Option<bool> {
    use cliotree_core::StringFormat;
    match kind {
        Kind::Date => Some(false),
        Kind::DateTime => Some(true),
        Kind::String(c) => match c.format {
            Some(StringFormat::Date) => Some(false),
            Some(StringFormat::DateTime) => Some(true),
            None => None,
        },
        _ => None,
    }
}

/// Today's date where the user is, for a calendar opened on an empty date.
fn today() -> jiff::civil::Date {
    jiff::Zoned::now().date()
}

fn count_text(n: usize) -> String {
    if n == 1 {
        "1 item".to_owned()
    } else {
        format!("{n} items")
    }
}

/// The label of a field: fixed width so that the editors line up, a mark for
/// a required field, and the help text of the schema on hover.
fn label_cell(ui: &mut egui::Ui, label: &str, required: bool, help: Option<&str>, width: f32) {
    if width <= 0.0 {
        return;
    }
    let response = ui
        .allocate_ui_with_layout(
            egui::vec2(width, ROW_HEIGHT),
            egui::Layout::left_to_right(Align::Center),
            |ui| {
                ui.set_min_size(egui::vec2(width, ROW_HEIGHT));
                let response = ui.add(egui::Label::new(label).truncate());
                if required {
                    ui.label(RichText::new("*").color(error_color(ui)));
                }
                response
            },
        )
        .inner;
    if let Some(help) = help {
        response.on_hover_text(help);
    }
}

/// A typeface and a size, each inherited when None.
type ColumnFont = (Option<Typeface>, Option<u32>);

/// The font of a column that has one of its own, over the one it inherits.
fn column_font(design: &ColumnDesign, inherited: ColumnFont) -> Option<ColumnFont> {
    (design.font.is_some() || design.font_size.is_some()).then(|| {
        (
            design.font.or(inherited.0),
            design.font_size.or(inherited.1),
        )
    })
}

/// Draws in a font, if one is given.
fn in_font<R>(
    ui: &mut egui::Ui,
    font: Option<ColumnFont>,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    match font {
        None => add(ui),
        Some((typeface, size)) => {
            ui.scope(|ui| {
                crate::style::use_font(ui.style_mut(), typeface, size);
                add(ui)
            })
            .inner
        }
    }
}

/// Draws a cell's contents to its left, centre or right.
fn aligned<R>(
    ui: &mut egui::Ui,
    align: Option<Alignment>,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let layout = match align {
        None | Some(Alignment::Left) => return add(ui),
        Some(Alignment::Center) => {
            egui::Layout::centered_and_justified(egui::Direction::LeftToRight)
        }
        Some(Alignment::Right) => egui::Layout::right_to_left(Align::Center),
    };
    ui.with_layout(layout, add).inner
}

/// The columns of a table of items of `item_ty`: its fields, or `value` for
/// scalars. None for items that are not flat, which are stacked.
/// A table of the items of an array, to draw.
struct FormTable<'a> {
    array: &'a Path,
    /// The number of items.
    len: usize,
    columns: &'a [(ColumnDesign, Option<Column>)],
    /// The rows and columns left shown.
    shown: &'a TableShown,
    /// The address of the control the table is of: its columns can be
    /// selected in layout mode, and out of it their expressions hide a
    /// column or colour a cell.
    at: Option<Address>,
    header_font: Option<ColumnFont>,
}

/// What of a table on a form is shown, by the filters and the hiding of the
/// tabular view.
#[derive(Default)]
struct TableShown {
    visible: table::Visible,
    hidden_columns: Vec<String>,
    hidden_rows: Vec<(usize, cliotree_core::NodeId)>,
    filtered: bool,
}

/// The menu of a column's title on a form: sorting, the filter, hiding.
fn column_menu(
    ui: &mut egui::Ui,
    array: &Path,
    name: &str,
    (sorts, can_filter, filtered): (bool, bool, bool),
) -> Option<crate::view::Action> {
    use crate::view::Action;
    let mut chosen = None;
    let mut item = |ui: &mut egui::Ui, enabled: bool, label: &str, action: Action| {
        if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
            chosen = Some(action);
            ui.close();
        }
    };
    let (array, name) = (array.clone(), name.to_owned());
    item(
        ui,
        sorts,
        "Sort ascending",
        Action::SortBy(array.clone(), name.clone(), Some(false)),
    );
    item(
        ui,
        sorts,
        "Sort descending",
        Action::SortBy(array.clone(), name.clone(), Some(true)),
    );
    ui.separator();
    item(
        ui,
        can_filter,
        "Filter...",
        Action::FilterDialog(array.clone(), name.clone()),
    );
    item(
        ui,
        filtered,
        "Clear filter",
        Action::ClearFilter(array.clone(), name.clone()),
    );
    ui.separator();
    item(ui, true, "Hide column", Action::HideColumn(array, name));
    chosen
}

fn item_columns(spec: &TypeSpec, item_ty: TypeId) -> Option<Vec<Column>> {
    match &spec.get(item_ty).kind {
        Kind::Object { .. } => {
            let columns = table::columns(spec, item_ty);
            (!columns.is_empty() && columns.iter().all(|c| !c.summary)).then_some(columns)
        }
        Kind::Array { .. } | Kind::Map { .. } => None,
        _ => Some(vec![Column {
            name: "value".to_owned(),
            fields: Vec::new(),
            ty: item_ty,
            required: true,
            summary: false,
        }]),
    }
}

/// The columns the table of the repeater at `at` can have, if it is a
/// table. In the design of a stacked repeater's items its path is read from
/// the first item.
pub(crate) fn columns_of(
    spec: &TypeSpec,
    design: &cliotree_core::form::FormDesign,
    at: &[usize],
) -> Option<Vec<Column>> {
    let path = design.path_in_item(at, design.get(at)?, 0)?;
    let resolved = spec.resolve(&path)?;
    match &spec.get(resolved.ty).kind {
        Kind::Array { items, .. } => item_columns(spec, *items),
        _ => None,
    }
}

/// The columns of a table that are shown, in their order, each with its
/// design and the column of the items it is (None for the row numbers).
fn arranged(
    control: Option<&Control>,
    columns: Vec<Column>,
) -> Vec<(ColumnDesign, Option<Column>)> {
    let names: Vec<String> = columns.iter().map(|c| c.name.clone()).collect();
    let designs = match control {
        Some(control) => control.arrange_columns(&names),
        None => Control::new(ControlKind::Repeater).arrange_columns(&names),
    };
    designs
        .into_iter()
        .filter(|d| !d.hidden)
        .map(|d| {
            let column = if d.is_row_number() {
                None
            } else {
                columns.iter().find(|c| c.name == d.name).cloned()
            };
            (d, column)
        })
        .collect()
}

/// What a column is called to a screen reader and a test.
pub(crate) fn column_label(name: &str, control: &Control) -> String {
    format!("Column {name} of {}", control_name(control))
}

/// The name of the last step of a path, which labels a control by default.
pub(crate) fn last_name(path: &Path) -> Option<String> {
    match path.segments().last()? {
        cliotree_core::Segment::Key(key) => Some(key.clone()),
        cliotree_core::Segment::Index(i) => Some(format!("[{i}]")),
    }
}

/// Why the control cannot be drawn against the schema, if it cannot.
pub(crate) fn design_problem(spec: &TypeSpec, control: &Control) -> Option<String> {
    cliotree_core::form::control_problem(spec, control)
}

/// What a control is called to a screen reader, a test, and the list of
/// broken controls.
pub(crate) fn control_name(control: &Control) -> String {
    let detail = match control.kind {
        ControlKind::Label => control.text.clone().unwrap_or_default(),
        ControlKind::Button => control
            .label
            .clone()
            .or_else(|| control.procedure.clone())
            .unwrap_or_default(),
        ControlKind::Separator => String::new(),
        _ => control
            .path
            .clone()
            .or_else(|| control.label.clone())
            .unwrap_or_default(),
    };
    if detail.is_empty() {
        control.kind.label().to_owned()
    } else {
        format!("{} {detail}", control.kind.label())
    }
}

/// A font that a control sets for a part of itself, over the one it inherits;
/// None if it sets neither the typeface nor the size.
fn own_font(
    typeface: Option<Typeface>,
    size: Option<u32>,
    inherited: ColumnFont,
) -> Option<ColumnFont> {
    (typeface.is_some() || size.is_some()).then(|| (typeface.or(inherited.0), size.or(inherited.1)))
}

/// A size dragged to `points`, snapped to the grid unless `snap` is off, and
/// kept within `range`.
pub(crate) fn snapped(points: f32, snap: bool, range: &std::ops::RangeInclusive<u32>) -> u32 {
    let points = if snap {
        (points / SNAP).round() * SNAP
    } else {
        points.round()
    };
    (points.max(0.0) as u32).clamp(*range.start(), *range.end())
}
