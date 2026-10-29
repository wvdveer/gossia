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
    Address, Alignment, ColumnDesign, Control, ControlKind, Editor, Format, RepeaterMode,
};
use cliotree_core::validate::validate_at;
use cliotree_core::{Kind, Node, Path, TypeId, TypeSpec, Value};
use egui::{Align, Key, RichText, Sense, Stroke, StrokeKind};
use egui_extras::{Column as TableColumn, TableBuilder};

use crate::app::ClioApp;
use crate::form_exprs::Gate;
use crate::form_layout::{
    ColumnEdit, ControlAction, DragItem, FormDialog, LayoutState, drop_allowed,
};
use crate::icons;
use crate::rows::display;
use crate::style::Typeface;
use crate::table::{self, Column};
use crate::view::{ROW_HEIGHT, computed_fill, error_color};

/// The form every Data Tab has. It cannot be renamed or deleted.
pub const DEFAULT_FORM: &str = cliotree_calc::DEFAULT_FORM;

/// The width of the labels of fields, in points.
const LABEL_WIDTH: f32 = 170.0;

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
        {
            let calc = self.calc();
            if let (Some(spec), Some(data)) = (calc.spec(&self.sheet), calc.data(&self.sheet)) {
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
                actions = painter.actions;
            }
        }
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
                self.commit_field(&path, &text);
            }
            FormAction::Direct(path, text) => {
                self.selection = Some(path.clone());
                self.enter_directly(&path, &text);
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
            FormAction::Select(path) => self.selection = Some(path),
            FormAction::Add(array) => self.add_item_to(&array),
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
        let exists = self
            .calc()
            .data(&self.sheet)
            .and_then(|d| d.get(&map.key(key)))
            .is_some();
        if exists {
            self.notice = Some(format!("{} already has an entry {key}", self.name_of(map)));
            return;
        }
        let value = self.calc().spec(&self.sheet).and_then(|spec| {
            match &spec.get(spec.resolve(map)?.ty).kind {
                Kind::Map { value } => Some(ClioApp::default_value(spec, *value)),
                _ => None,
            }
        });
        let Some(value) = value else {
            return;
        };
        let map_present = self
            .calc()
            .data(&self.sheet)
            .is_some_and(|d| d.get(map).is_some());
        if map_present {
            self.set_node(&map.key(key), value);
        } else {
            let mut fields = std::collections::BTreeMap::new();
            fields.insert(key.to_owned(), Node::new(value));
            self.set_node(map, Value::Object(fields.into_iter().collect()));
        }
        self.form
            .new_keys
            .remove(&(self.sheet.clone(), map.clone()));
    }
}

// ------------------------------------------------------------------ text

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
    column_targets: Vec<(String, egui::Rect)>,
    /// Where the control being drawn is in the design, for the expressions
    /// of the columns of its table.
    at: Address,
    /// In layout mode: where each control was drawn, for dropping a dragged
    /// one.
    drops: Vec<DropZone>,
}

/// Where a control was drawn in layout mode.
#[derive(Debug, Clone)]
pub(crate) struct DropZone {
    pub at: Address,
    /// For a control other than a group, the whole width of its list, so
    /// that the space beside a narrow one counts as it.
    pub rect: egui::Rect,
    pub group: bool,
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
        })
    };
    let (parent, frame) = match over {
        Some(z) if !z.group => return beside(z, pos.y > z.rect.center().y),
        Some(z) if pos.y < z.rect.top() + GROUP_EDGE => return beside(z, false),
        Some(z) if pos.y > z.rect.bottom() - GROUP_EDGE => return beside(z, true),
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
    if let Some(place) = &place {
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
    Some(ControlAction::Drop(item, place.parent, place.index))
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
}

/// Where a control is among the others of its list, for its menu.
struct Slot {
    at: Address,
    siblings: usize,
    above_is_group: bool,
}

fn slots(list: &[Control], parent: &[usize]) -> Vec<Slot> {
    (0..list.len())
        .map(|i| {
            let mut at = parent.to_vec();
            at.push(i);
            Slot {
                at,
                siblings: list.len(),
                above_is_group: i > 0 && list[i - 1].kind == ControlKind::Group,
            }
        })
        .collect()
}

impl Painter<'_> {
    fn key(&self, path: &Path) -> (String, Path) {
        (self.app.sheet.clone(), path.clone())
    }

    fn arrive(&mut self, path: &Path, response: &egui::Response) {
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
        if self.layout.is_some() {
            self.control_in_style(ui, c, slot);
            return;
        }
        let gate = crate::form_exprs::gate(self.app.calc(), &self.app.sheet, self.form, &slot.at);
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
        if c.kind == ControlKind::Group {
            self.group(ui, c, slot);
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
            target.dnd_set_drag_payload((DragItem::Control(slot.at.clone()), control_name(c)));
            self.drops.push(DropZone {
                at: slot.at.clone(),
                rect: egui::Rect::from_x_y_ranges(ui.max_rect().x_range(), rect.y_range()),
                group: false,
            });
            self.layout_target(ui, &target, rect, c, slot);
            for (name, title) in columns {
                self.column_target(ui, c, slot, &name, title, rect);
            }
        }
    }

    /// The title of a column of a table in layout mode: a click selects the
    /// column, and its menu moves or hides it.
    fn column_target(
        &mut self,
        ui: &mut egui::Ui,
        c: &Control,
        slot: &Slot,
        name: &str,
        title: egui::Rect,
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
        let selected = self.layout.as_ref().and_then(Option::as_ref) == Some(&slot.at)
            && self.column.is_none();
        let name = control_name(c);
        target.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, name.clone()));
        if target.clicked() {
            self.actions
                .push(FormAction::Control(ControlAction::Select(slot.at.clone())));
        }
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
    }

    /// A group: its controls in columns, in a box when it has a title or is
    /// bound to an object.
    fn group(&mut self, ui: &mut egui::Ui, c: &Control, slot: &Slot) {
        let layout = self.layout.is_some();
        let problem = design_problem(self.spec, c);
        let path = c.bound_path().filter(|_| problem.is_none());
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
                label_cell(ui, label.as_deref().unwrap_or(""), false, help);
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
            let columns = c.columns.unwrap_or(1).clamp(1, 6) as usize;
            if columns == 1 {
                this.controls(ui, &c.children, &slot.at);
            } else {
                let list = slots(&c.children, &slot.at);
                ui.columns(columns, |cols| {
                    for (i, (child, child_slot)) in c.children.iter().zip(&list).enumerate() {
                        this.control(&mut cols[i % columns], child, child_slot);
                    }
                });
            }
        };
        if !framed {
            draw(self, ui);
            return;
        }
        let response = ui.group(|ui| {
            ui.set_width(ui.available_width());
            let title = label.clone().unwrap_or_default();
            if layout {
                let header = ui.add(
                    egui::Label::new(RichText::new(&title).strong()).sense(Sense::click_and_drag()),
                );
                let name = control_name(c);
                header.dnd_set_drag_payload((DragItem::Control(slot.at.clone()), name.clone()));
                header.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::Role::Button, true, name.clone())
                });
                if header.clicked() {
                    self.actions
                        .push(FormAction::Control(ControlAction::Select(slot.at.clone())));
                }
                header.context_menu(|ui| self.control_menu(ui, slot));
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
            });
        }
        if self.layout.as_ref().and_then(Option::as_ref) == Some(&slot.at) {
            ui.painter().rect_stroke(
                response.response.rect.expand(2.0),
                3.0,
                Stroke::new(1.5, ui.visuals().selection.stroke.color),
                StrokeKind::Outside,
            );
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
                let response = ui.add_enabled(can, egui::Button::new(&label));
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
        if let Some(problem) = design_problem(self.spec, c) {
            self.placeholder(ui, c, &problem);
            return;
        }
        let Some(path) = c.bound_path() else {
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
            },
        ));
        match (&self.spec.get(ty).kind, c.kind) {
            (Kind::Array { .. }, _) => {
                self.repeater_of(ui, &path, ty, &label, Some(c));
            }
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
                label_cell(ui, label, required, self.help(path, ty));
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
        let problem = ui
            .horizontal(|ui| {
                label_cell(ui, label, required, self.help(path, ty));
                self.editor(ui, path, ty, required, false)
            })
            .inner;
        if let Some(problem) = problem {
            ui.horizontal(|ui| {
                ui.add_space(LABEL_WIDTH + ui.spacing().item_spacing.x);
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
        let computed = formula.is_some() || computed_by.is_some();
        let read_only = computed || !app.can_edit(path);
        let node = self.data.get(path);
        let value = node.map(Node::value);
        let kind = &self.spec.get(ty).kind;
        let name = app.name_of(path);
        let key = self.key(path);
        let look = self
            .look
            .as_ref()
            .filter(|(p, _)| p == path)
            .map(|(_, look)| look.clone())
            .unwrap_or_default();
        let read_only = read_only || look.read_only;
        let as_text = matches!(look.editor, Some(Editor::Text | Editor::Multiline));

        let mut problem = None;
        let mut response = None;
        match (value, kind) {
            (Some(Value::Error(error)), _) => {
                let r = ui.label(
                    RichText::new(error.kind.code())
                        .color(error_color(ui))
                        .strong(),
                );
                r.on_hover_text(format!("{} {}", error.kind.code(), error.message));
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
                let width = if compact {
                    f32::INFINITY
                } else {
                    ui.available_width().min(360.0)
                };
                let edit = if long {
                    egui::TextEdit::multiline(&mut text).desired_rows(3)
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
        }
        if let Some(text) = formula {
            ui.label(RichText::new("fx").italics().weak())
                .on_hover_text(format!("={text}"));
        } else if let Some(by) = &computed_by {
            ui.label(RichText::new("fx").italics().weak())
                .on_hover_text(format!("computed by {}", app.name_of(by)));
        }
        if pending {
            crate::style::pending_marker(ui).on_hover_text("Waiting to be calculated");
        }
        problem
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
        ui.group(|ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let title_font =
                    control.and_then(|c| own_font(c.title_font, c.title_font_size, self.font));
                let response = in_font(ui, title_font, |ui| ui.strong(label));
                self.arrive(path, &response);
                ui.label(RichText::new(count_text(items.len())).weak());
                if ui.add_enabled(can_add, egui::Button::new("New")).clicked() {
                    self.actions.push(FormAction::Add(path.clone()));
                }
            });
            if let Some(help) = self.help(path, ty) {
                ui.label(RichText::new(help).weak().small());
            }
            let columns = match control.and_then(|c| c.mode) {
                Some(RepeaterMode::Stacked) => None,
                _ => item_columns(self.spec, item_ty).map(|columns| arranged(control, columns)),
            };
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
                            self.table(ui, path, items.len(), &columns, at, header_font);
                        });
                }
                Some(_) => {}
                None => self.stacked(ui, path, item_ty, items.len()),
            }
        });
    }

    /// A table of the items of an array: its columns that are shown, in their
    /// order, each with its own properties. `at` is the address of the
    /// control the table is of: its columns can be selected in layout mode,
    /// and out of it their expressions hide a column or colour a cell.
    fn table(
        &mut self,
        ui: &mut egui::Ui,
        array: &Path,
        len: usize,
        columns: &[(ColumnDesign, Option<Column>)],
        at: Option<Address>,
        header_font: Option<ColumnFont>,
    ) {
        let designed = at.is_some();
        // The expressions of the columns, out of layout mode: a column whose
        // *visible when* is false is left out; what the others say about the
        // column as a whole goes to each of its cells.
        let gated = at.filter(|_| self.layout.is_none());
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
            if !head.hidden {
                shown.push((design.clone(), column.clone()));
                heads.push(head);
            }
        }
        let columns = &shown[..];
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
        let mut table = TableBuilder::new(ui)
            .id_salt(("form_table", &self.app.sheet, array.to_string()))
            .striped(true)
            .vscroll(false)
            .sense(Sense::click())
            .cell_layout(egui::Layout::left_to_right(Align::Center));
        for (design, column) in columns {
            table = table.column(match (design.width, column) {
                (Some(width), _) => TableColumn::exact(width as f32).clip(true),
                (None, None) => TableColumn::auto().at_least(32.0),
                (None, Some(_)) => TableColumn::initial(140.0).at_least(60.0).clip(true),
            });
        }
        let mut sort = None;
        let mut titles = Vec::new();
        table
            .header(header_height, |mut header| {
                for (design, column) in columns {
                    let mut max = egui::Rect::NOTHING;
                    let (used, _) = header.col(|ui| {
                        max = ui.max_rect();
                        in_font(ui, header_font, |ui| {
                            aligned(ui, design.title_align, |ui| {
                                let title =
                                    design.label.clone().unwrap_or_else(|| design.name.clone());
                                let text = RichText::new(title).strong();
                                let sorts = sortable && column.is_some();
                                let mut response = if sorts {
                                    ui.add(egui::Label::new(text).sense(Sense::CLICK))
                                } else {
                                    ui.label(text)
                                };
                                let hover = match (&design.help, sorts) {
                                    (Some(help), true) => {
                                        Some(format!("{help}\nClick to sort by this column"))
                                    }
                                    (Some(help), false) => Some(help.clone()),
                                    (None, true) => Some("Sort by this column".to_owned()),
                                    (None, false) => None,
                                };
                                if let Some(hover) = hover {
                                    response = response.on_hover_text(hover);
                                }
                                if sorts && response.clicked() {
                                    sort = Some(design.name.clone());
                                }
                            });
                        });
                    });
                    // The column, or its title where that is wider. (The response
                    // of the cell reaches past the column, by the spacing.)
                    titles.push((design.name.clone(), max.union(used)));
                }
            })
            .body(|body| {
                body.rows(row_height, len, |mut row| {
                    let index = row.index();
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
                                        let look = Look {
                                            help: None,
                                            editor: design.editor,
                                            read_only: design.read_only,
                                            masked: false,
                                            align: design.align,
                                            format: design.format(),
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
        if designed && self.layout.is_some() {
            self.column_targets = titles;
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
    }

    /// Items that are not flat, one at a time with a stepper.
    fn stacked(&mut self, ui: &mut egui::Ui, array: &Path, item_ty: TypeId, len: usize) {
        if len == 0 {
            return;
        }
        let at = self
            .app
            .form
            .steps
            .get(&self.key(array))
            .copied()
            .unwrap_or(0)
            .min(len - 1);
        let editable = self.app.can_reorder(array);
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
        self.node(ui, &item, item_ty, &format!("Item {}", at + 1), true);
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

fn count_text(n: usize) -> String {
    if n == 1 {
        "1 item".to_owned()
    } else {
        format!("{n} items")
    }
}

/// The label of a field: fixed width so that the editors line up, a mark for
/// a required field, and the help text of the schema on hover.
fn label_cell(ui: &mut egui::Ui, label: &str, required: bool, help: Option<&str>) {
    let response = ui
        .allocate_ui_with_layout(
            egui::vec2(LABEL_WIDTH, ROW_HEIGHT),
            egui::Layout::left_to_right(Align::Center),
            |ui| {
                ui.set_min_size(egui::vec2(LABEL_WIDTH, ROW_HEIGHT));
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

/// The columns the table of a repeater control can have, if it is a table.
pub(crate) fn columns_of(spec: &TypeSpec, control: &Control) -> Option<Vec<Column>> {
    let path = control.bound_path()?;
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
