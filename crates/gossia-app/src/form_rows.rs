//! Adding and deleting the items of a repeater section (DESIGN.md, Later
//! versions, "Form canvases, 0.7.0", "The add/delete button"): the button
//! in each row, which deletes its item, and the blank row after the items,
//! whose fields are typed into a draft that *Add* makes a new item.
//!
//! The blank row is drawn for the item at [`BLANK_ROW`] of the array, an
//! index no item has, so what is typed there, and the text being typed, are
//! kept by paths that never meet an item's, whatever is added or deleted
//! meanwhile.
//!
//! The other way ("Marking rows"): a row mark in each row, and rows buttons
//! on the fixed sections that delete the marked rows, add a row, or mark
//! them all. The marks belong to the form in this window, with the array
//! and the number of items they were made for: when either changes (an
//! edit elsewhere, Undo, a formula, another current row of a master
//! repeater) they no longer fit, and are cleared.

use std::collections::BTreeSet;

use egui::RichText;
use gossia_calc::entry::{self, Entered};
use gossia_core::form::canvas::{RowAllows, RowsAction};
use gossia_core::form::{Control, ControlKind, FormDesign};
use gossia_core::{Kind, Node, Path, Segment, Value};
use gossia_i18n::tr;

use super::{Draft, FormAction, Painter, entry_text};
use crate::app::GossiaApp;
use crate::form_exprs::{fixed, readable_on};

/// The index of the blank row's item: past any item there can be.
pub(super) const BLANK_ROW: usize = usize::MAX;

/// The blank row being drawn: the item it stands for, and what has been
/// typed into it.
#[derive(Debug, Clone)]
pub(super) struct Blank {
    pub item: Path,
    pub node: Node,
}

/// What the add/delete button of the repeater section being drawn may do.
#[derive(Debug, Clone)]
pub(super) struct RowEdit {
    pub array: Path,
    pub allows: RowAllows,
    /// Its items can be added and deleted: the array is not computed. A
    /// section without an add/delete button cannot.
    pub editable: bool,
}

/// The marked rows of a repeater section, with the array and the number of
/// items they were marked in.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Marks {
    pub array: Path,
    pub len: usize,
    pub rows: BTreeSet<usize>,
}

/// A repeater section of the canvas form being drawn, as its row mark and
/// the rows buttons that name it see it.
#[derive(Debug, Clone, Default)]
pub(super) struct Repeater {
    /// Its array, bound with the contexts; None if they do not bind it.
    pub array: Option<Path>,
    pub len: usize,
    /// Its items can be added, deleted and marked: the array is not
    /// computed, and the form is not being designed.
    pub changeable: bool,
    /// It has a row mark.
    pub marking: bool,
    /// The marked rows, if they fit the array.
    pub marked: BTreeSet<usize>,
}

/// A draft with nothing typed into it.
pub(super) fn empty_draft() -> Node {
    Node::new(Value::Object(Default::default()))
}

/// The name of a node as the window shows it, with the blank row's index
/// as `new`.
pub(super) fn blank_name(name: String) -> String {
    name.replace(&format!("[{BLANK_ROW}]"), "[new]")
}

/// The steps of `path` after the item of the blank row of `array`, if it is
/// in it.
fn inside_blank(array: &Path, path: &Path) -> Option<Path> {
    let item = array.index(BLANK_ROW);
    path.segments()
        .starts_with(item.segments())
        .then(|| Path::from(path.segments()[item.len()..].to_vec()))
}

/// Sets the node at `steps` of a draft, making the objects on the way.
fn set_in(node: &mut Node, steps: &[Segment], value: Value) {
    let Some((first, rest)) = steps.split_first() else {
        *node.value_mut() = value;
        return;
    };
    // A row's fields are reached by names; an index in one is not kept.
    let Segment::Key(name) = first else {
        return;
    };
    if !matches!(node.value(), Value::Object(_)) {
        *node.value_mut() = Value::Object(Default::default());
    }
    let Value::Object(fields) = node.value_mut() else {
        return;
    };
    let child = fields
        .entry(name.clone())
        .or_insert_with(|| Node::new(Value::Null));
    set_in(child, rest, value);
}

/// Takes the node at `steps` out of a draft.
fn remove_in(node: &mut Node, steps: &[Segment]) {
    let Some((Segment::Key(name), rest)) = steps.split_first() else {
        return;
    };
    let Value::Object(fields) = node.value_mut() else {
        return;
    };
    if rest.is_empty() {
        fields.shift_remove(name);
    } else if let Some(child) = fields.get_mut(name) {
        remove_in(child, rest);
    }
}

/// Puts what a draft holds over a new item.
fn merge(into: &mut Node, draft: &Node) {
    match draft.value() {
        Value::Object(fields) => {
            if !matches!(into.value(), Value::Object(_)) {
                *into.value_mut() = Value::Object(Default::default());
            }
            let Value::Object(target) = into.value_mut() else {
                return;
            };
            for (name, child) in fields {
                let slot = target
                    .entry(name.clone())
                    .or_insert_with(|| Node::new(Value::Null));
                merge(slot, child);
            }
        }
        value => *into.value_mut() = value.clone(),
    }
}

/// The first add/delete button on a repeater section's row.
fn row_button_in(control: &Control) -> Option<&Control> {
    control.children.iter().find_map(|c| {
        if c.kind == ControlKind::RowButton {
            Some(c)
        } else {
            row_button_in(c)
        }
    })
}

impl Painter<'_> {
    /// The node at `path`: in the blank row, from its draft.
    pub(super) fn node_at(&self, path: &Path) -> Option<&Node> {
        if let Some(blank) = &self.blank
            && path.segments().starts_with(blank.item.segments())
        {
            return blank
                .node
                .get(&Path::from(path.segments()[blank.item.len()..].to_vec()));
        }
        self.data.get(path)
    }

    /// Whether `path` is in the blank row being drawn, where every field
    /// can be typed into although its node is not there.
    pub(super) fn in_blank(&self, path: &Path) -> bool {
        self.blank
            .as_ref()
            .is_some_and(|b| path.segments().starts_with(b.item.segments()))
    }

    /// Whether `path`, in the blank row, can be typed into: not a column
    /// the array has a formula for, which the new item gets.
    pub(super) fn blank_typed(&self, path: &Path) -> bool {
        let Some(blank) = &self.blank else {
            return false;
        };
        if !path.segments().starts_with(blank.item.segments()) {
            return false;
        }
        let Some(array) = blank.item.parent() else {
            return false;
        };
        match path.segments().get(blank.item.len()) {
            Some(Segment::Key(field)) => self
                .app
                .calc()
                .column_formula(&self.app.sheet, &array, field)
                .is_none(),
            _ => true,
        }
    }

    /// What the add/delete button of the repeater section `section`, over
    /// `array`, may do. Not in layout mode, where a section is designed.
    pub(super) fn row_edit_of(&self, section: &Control, array: &Path) -> RowEdit {
        let button = row_button_in(section);
        let editable = button.is_some()
            && self.layout.is_none()
            && (self.data.get(array).is_none() || self.app.can_reorder(array));
        RowEdit {
            array: array.clone(),
            allows: button.map(Control::row_allows).unwrap_or_default(),
            editable,
        }
    }

    /// Finds the repeater sections of a canvas form, for row marks and rows
    /// buttons; marks that no longer fit their array are cleared.
    pub(super) fn prepare_repeaters(&mut self, design: &FormDesign) {
        self.repeaters.clear();
        for index in design.repeater_sections() {
            let (Some(name), Some(c)) = (design.row_name_of(index), design.controls.get(index))
            else {
                continue;
            };
            let array = self.own_path(c);
            let len = array
                .as_ref()
                .and_then(|a| self.data.get(a))
                .and_then(|n| n.value().as_array().map(<[Node]>::len))
                .unwrap_or(0);
            let changeable = self.layout.is_none()
                && array
                    .as_ref()
                    .is_some_and(|a| self.data.get(a).is_none() || self.app.can_reorder(a));
            let key = (self.app.sheet.clone(), self.form.to_owned(), name.clone());
            let marked = match self.app.form.marks.get(&key) {
                Some(m) if Some(&m.array) == array.as_ref() && m.len == len => m.rows.clone(),
                Some(_) => {
                    self.actions.push(FormAction::ClearMarks(name.clone()));
                    BTreeSet::new()
                }
                None => BTreeSet::new(),
            };
            let marking = design.row_mark_of(index).is_some();
            self.repeaters.insert(
                name,
                Repeater {
                    array,
                    len,
                    changeable,
                    marking,
                    marked,
                },
            );
        }
    }

    /// Gives the focus to the first of the controls the arrow keys can go
    /// to that were drawn since `stops`, and scrolls `row` into view.
    pub(super) fn focus_first(&mut self, ui: &mut egui::Ui, stops: usize, row: egui::Rect) {
        let first = self.nav[stops..]
            .iter()
            .min_by(|a, b| {
                (a.rect.top(), a.rect.left())
                    .partial_cmp(&(b.rect.top(), b.rect.left()))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|s| s.id);
        if let Some(id) = first {
            ui.memory_mut(|m| m.request_focus(id));
        }
        if !ui.clip_rect().contains_rect(row) {
            ui.scroll_to_rect(row, None);
        }
    }

    /// Whether the first field of the row of `item` is to be given the
    /// focus.
    pub(super) fn wants_row_focus(&self, item: &Path) -> bool {
        self.app
            .form
            .focus_row
            .as_ref()
            .is_some_and(|(sheet, i)| sheet == &self.app.sheet && i == item)
    }

    /// The row mark: a checkbox in each row of a repeater section, ticked
    /// when the row is marked. Not drawn in the blank row, nor where the
    /// rows cannot be deleted (a computed array).
    pub(super) fn row_mark(&mut self, ui: &mut egui::Ui, c: &Control) {
        let size = c.height.map_or(0.0, |h| h as f32);
        if self.layout.is_some() {
            ui.add_enabled(false, egui::Checkbox::without_text(&mut false))
                .on_disabled_hover_text(tr!("Marks the row, for a rows button to act on."));
            let _ = size;
            return;
        }
        if self.blank.is_some() {
            return;
        }
        let Some((name, index)) = self.row.clone() else {
            return;
        };
        let Some(repeater) = self.repeaters.get(&name) else {
            return;
        };
        let (Some(array), true) = (repeater.array.clone(), repeater.changeable) else {
            return;
        };
        let len = repeater.len;
        let was = repeater.marked.contains(&index);
        let mut on = was;
        let response = ui.add(egui::Checkbox::without_text(&mut on));
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::Role::CheckBox,
                true,
                was,
                tr!("Mark row {row}", row = index + 1),
            )
        });
        self.nav.push(crate::form_nav::NavStop {
            id: response.id,
            rect: response.rect,
            text: false,
            vertical: true,
        });
        if on != was {
            self.actions.push(FormAction::Mark {
                row: name,
                array,
                len,
                index,
                on,
            });
        }
    }

    /// A rows button: *Delete marked* with the count of marked rows, *Add
    /// row*, or *Toggle all*, acting on the repeater it names by row name.
    pub(super) fn rows_button(&mut self, ui: &mut egui::Ui, c: &Control) {
        let action = c.rows_action();
        let repeater = c
            .repeater
            .as_deref()
            .and_then(|n| self.repeaters.get(n))
            .cloned();
        let count = repeater.as_ref().map_or(0, |r| r.marked.len());
        let mut label = c.rows_button_label();
        if action == RowsAction::DeleteMarked {
            label = format!("{label} ({count})");
        }
        let (can, hint) = match &repeater {
            _ if self.layout.is_some() => (false, String::new()),
            None => (
                false,
                tr!("Choose the repeater this button acts on.").to_owned(),
            ),
            Some(r) if r.array.is_none() => (
                false,
                tr!("The repeater's path does not lead to a list.").to_owned(),
            ),
            Some(r) if !r.changeable => (
                false,
                tr!("The rows of this repeater are worked out by a formula.").to_owned(),
            ),
            Some(r) if action.needs_marks() && !r.marking => {
                (false, tr!("The repeater has no row mark.").to_owned())
            }
            Some(_) if action == RowsAction::DeleteMarked && count == 0 => {
                (false, tr!("No row is marked.").to_owned())
            }
            Some(r) if action == RowsAction::ToggleAll && r.len == 0 => {
                (false, tr!("There are no rows.").to_owned())
            }
            Some(_) => (true, action.label().to_owned()),
        };
        let width = if self.sized {
            ui.available_width()
        } else {
            0.0
        };
        let height = c.height.map_or(0.0, |h| h as f32);
        let button = egui::Button::new(&label)
            .wrap_mode(egui::TextWrapMode::Truncate)
            .min_size(egui::vec2(width, height));
        let response = ui.add_enabled(can, button);
        if !can {
            if !hint.is_empty() {
                response.on_disabled_hover_text(hint);
            }
            return;
        }
        self.nav.push(crate::form_nav::NavStop {
            id: response.id,
            rect: response.rect,
            text: false,
            vertical: true,
        });
        if !response.on_hover_text(hint).clicked() {
            return;
        }
        let (Some(r), Some(name)) = (repeater, c.repeater.clone()) else {
            return;
        };
        let Some(array) = r.array else {
            return;
        };
        self.actions.push(match action {
            RowsAction::DeleteMarked => FormAction::DeleteMarked(name, array),
            RowsAction::AddRow => FormAction::AppendRow(name, array),
            RowsAction::ToggleAll => FormAction::ToggleMarks(name, array, r.len),
        });
    }

    /// Whether the first field of the blank row of `array` is to be given
    /// the focus.
    pub(super) fn wants_blank_focus(&self, array: &Path) -> bool {
        self.app
            .form
            .focus_blank
            .as_ref()
            .is_some_and(|(sheet, a)| sheet == &self.app.sheet && a == array)
    }

    /// The add/delete button: *Add* in the blank row, *Delete* in a row of
    /// an item, each in its own label and colours. Not drawn where it may
    /// not do that (*Add only*, *Delete only*, or a computed array). In
    /// layout mode, its delete configuration, as it is in a row.
    pub(super) fn row_button(&mut self, ui: &mut egui::Ui, c: &Control) {
        let add = self.blank.is_some();
        let edit = self.row_edit.clone();
        if let Some(edit) = &edit {
            let allowed = if add {
                edit.allows.adds()
            } else {
                edit.allows.deletes()
            };
            if !edit.editable || !allowed {
                return;
            }
        }
        let (fill, text) = if add {
            (&c.add_fill_colour, &c.add_text_colour)
        } else {
            (&c.delete_fill_colour, &c.delete_text_colour)
        };
        let fill = fill.as_deref().and_then(fixed);
        let text = text.as_deref().and_then(fixed).or(fill.map(readable_on));
        let mut label = RichText::new(c.row_button_label(add));
        if let Some(text) = text {
            label = label.color(text);
        }
        let width = if self.sized {
            ui.available_width()
        } else {
            0.0
        };
        let height = c.height.map_or(0.0, |h| h as f32);
        let mut button = egui::Button::new(label)
            .wrap_mode(egui::TextWrapMode::Truncate)
            .min_size(egui::vec2(width, height));
        if let Some(fill) = fill {
            button = button.fill(fill);
        }
        let response = ui.add(button);
        let Some(edit) = edit else {
            return;
        };
        if response.clicked() {
            if add {
                self.actions.push(FormAction::AddRow(edit.array));
            } else if let Some((_, i)) = &self.row {
                self.actions.push(FormAction::Delete(edit.array, *i));
            }
        }
    }
}

impl GossiaApp {
    fn marks_key(&self, row: &str) -> (String, String, String) {
        (self.sheet.clone(), self.form_name(), row.to_owned())
    }

    /// The marks of the repeater `row` of the form shown, made afresh when
    /// they were made for another array or number of items.
    fn marks_mut(&mut self, row: &str, array: &Path, len: usize) -> &mut Marks {
        let key = self.marks_key(row);
        let marks = self.form.marks.entry(key).or_default();
        if &marks.array != array || marks.len != len {
            *marks = Marks {
                array: array.clone(),
                len,
                rows: BTreeSet::new(),
            };
        }
        marks
    }

    /// Marks or unmarks a row of the repeater `row`.
    pub(super) fn set_mark(&mut self, row: &str, array: &Path, len: usize, index: usize, on: bool) {
        if index >= len {
            return;
        }
        let marks = self.marks_mut(row, array, len);
        if on {
            marks.rows.insert(index);
        } else {
            marks.rows.remove(&index);
        }
    }

    /// *Toggle all*: marks every row of the repeater `row` when any is
    /// unmarked, else clears every mark.
    pub(super) fn toggle_marks(&mut self, row: &str, array: &Path, len: usize) {
        let marks = self.marks_mut(row, array, len);
        marks.rows = if marks.rows.len() < len {
            (0..len).collect()
        } else {
            BTreeSet::new()
        };
    }

    /// *Delete marked*: deletes the marked rows of the repeater `row`, as
    /// one edit to undo, without asking. Undo brings them back, unmarked.
    pub(super) fn delete_marked(&mut self, row: &str, array: &Path) {
        let key = self.marks_key(row);
        let Some(marks) = self.form.marks.remove(&key) else {
            return;
        };
        if &marks.array != array || marks.rows.is_empty() {
            return;
        }
        let sheet = self.sheet.clone();
        let count = marks.rows.len();
        let calc = self.session.calc_mut();
        calc.begin_entry(&tr!("Delete {count} rows", count = count));
        let mut refused = None;
        // From the last, so the numbers of the others stay as marked.
        for &i in marks.rows.iter().rev() {
            if let Err(e) = calc.remove(&sheet, &array.index(i)) {
                refused = Some(e.to_string());
                break;
            }
        }
        calc.end_entry();
        self.notice = refused;
        self.session.tick();
    }

    /// *Add row* on a rows button: appends an item as *New* makes one to the
    /// array of the repeater `row`, as one edit; it becomes the current row
    /// and its first field gets the focus.
    pub(super) fn append_row(&mut self, row: &str, array: &Path) {
        let before = self.array_len(array);
        self.apply_form(FormAction::Add(array.clone()));
        if self.array_len(array) <= before {
            return;
        }
        let form = self.form_name();
        let sheet = self.sheet.clone();
        self.session
            .calc_mut()
            .set_form_context(&sheet, &form, row, Value::Int(before as i64));
        self.form.focus_row = Some((sheet, array.index(before)));
    }

    /// The marked rows of each repeater of the form `form`, by row name, for
    /// a procedure's `ctx.marked`: an empty list for one with none.
    pub(super) fn marked_rows(&self, form: &str) -> Vec<(String, Vec<usize>)> {
        let design = self.design_of(form);
        if !design.canvas {
            return Vec::new();
        }
        design
            .row_names()
            .into_iter()
            .map(|name| {
                let key = (self.sheet.clone(), form.to_owned(), name.clone());
                let rows = self
                    .form
                    .marks
                    .get(&key)
                    .map(|m| m.rows.iter().copied().collect())
                    .unwrap_or_default();
                (name, rows)
            })
            .collect()
    }

    /// Enters an edit made in the blank row of the repeater section over
    /// `array` into its draft, read and checked as typing into the node
    /// would be. Refused text stays in the field with the reason.
    pub(super) fn edit_row_draft(&mut self, array: &Path, action: FormAction) {
        let sheet = self.sheet.clone();
        let Some(spec) = self.calc().spec(&sheet).cloned() else {
            return;
        };
        let (path, text, typed) = match action {
            FormAction::Commit(path, text) => (path, text, true),
            FormAction::Direct(path, text) => (path, text, false),
            FormAction::Create(path) => {
                let Some(steps) = inside_blank(array, &path) else {
                    return;
                };
                if let Some(r) = spec.resolve(&path) {
                    let value = GossiaApp::default_value(&spec, r.ty);
                    let draft = self.row_draft_mut(array);
                    set_in(draft, steps.segments(), value);
                }
                return;
            }
            _ => return,
        };
        let Some(steps) = inside_blank(array, &path) else {
            return;
        };
        let Some(kind) = spec.resolve(&path).map(|r| spec.get(r.ty).kind.clone()) else {
            return;
        };
        let entered = if typed {
            entry_text(&kind, &text)
        } else {
            text.clone()
        };
        let key = (sheet, path.clone());
        let refused = match entry::classify(&spec, &path, &entered) {
            Ok(Entered::Literal(value)) => {
                set_in(self.row_draft_mut(array), steps.segments(), value);
                None
            }
            Ok(Entered::Clear) => {
                remove_in(self.row_draft_mut(array), steps.segments());
                None
            }
            Ok(Entered::Formula(_)) => Some(
                tr!("a form has no formulas; enter a literal here, or a formula in the data tree")
                    .to_owned(),
            ),
            Err(e) => Some(e.to_string()),
        };
        match refused {
            None => {
                self.form.drafts.remove(&key);
                self.notice = None;
            }
            Some(error) if typed && !matches!(kind, Kind::Bool) => {
                self.form.drafts.insert(
                    key,
                    Draft {
                        text,
                        error: Some(error),
                    },
                );
            }
            Some(error) => self.notice = Some(error),
        }
    }

    fn row_draft_mut(&mut self, array: &Path) -> &mut Node {
        self.form
            .row_drafts
            .entry((self.sheet.clone(), array.clone()))
            .or_insert_with(empty_draft)
    }

    /// *Add* in the blank row of the repeater section over `array`: a new
    /// item as *New* makes one, with the draft's values over it, appended as
    /// one edit. Text still being typed there counts; text that is refused
    /// keeps it from adding, and says why. The draft is then emptied, and
    /// the first field of the new blank row gets the focus.
    pub(super) fn add_from_draft(&mut self, array: &Path) {
        let sheet = self.sheet.clone();
        let mut typing: Vec<(Path, Draft)> = self
            .form
            .drafts
            .iter()
            .filter(|((s, p), _)| s == &sheet && inside_blank(array, p).is_some())
            .map(|((_, p), d)| (p.clone(), d.clone()))
            .collect();
        typing.sort_by_key(|(p, _)| p.to_string());
        for (path, draft) in typing {
            if draft.error.is_none() {
                self.edit_row_draft(array, FormAction::Commit(path.clone(), draft.text));
            }
            let refused = self
                .form
                .drafts
                .get(&(sheet.clone(), path.clone()))
                .and_then(|d| d.error.clone());
            if let Some(error) = refused {
                self.notice = Some(tr!(
                    "The row was not added: {field}: {error}",
                    field = blank_name(self.name_of(&path)),
                    error = error
                ));
                return;
            }
        }
        let calc = self.session.calc_mut();
        let Some(spec) = calc.spec(&sheet) else {
            return;
        };
        let item_type = match spec.resolve(array).map(|r| (r.ty, &spec.get(r.ty).kind)) {
            Some((_, Kind::Array { items, .. })) => *items,
            Some((ty, Kind::Any)) => ty,
            _ => return,
        };
        let mut item = Node::new(GossiaApp::default_value(spec, item_type));
        let key = (sheet.clone(), array.clone());
        if let Some(draft) = self.form.row_drafts.get(&key) {
            merge(&mut item, draft);
        }
        let items = calc
            .data(&sheet)
            .and_then(|d| d.get(array))
            .map(|n| n.value().as_array().map_or(0, <[Node]>::len));
        let added = match items {
            Some(len) => calc
                .insert_item(&sheet, array, len, item.value().clone())
                .map(|()| len),
            // No array yet: one holding the new item.
            None => calc
                .set_value(&sheet, array, Value::Array(vec![item]))
                .map(|()| 0),
        };
        match added {
            Ok(at) => {
                self.form.row_drafts.remove(&key);
                self.form
                    .drafts
                    .retain(|(s, p), _| s != &sheet || inside_blank(array, p).is_none());
                self.form.focus_blank = Some(key);
                self.notice = None;
                self.session.tick();
                self.pin_form_row(&array.index(at));
            }
            Err(e) => self.notice = Some(e.to_string()),
        }
    }
}
