//! Adding and deleting the items of a repeater section (DESIGN.md, Later
//! versions, "Form canvases, 0.7.0", "The add/delete button"): the button
//! in each row, which deletes its item, and the blank row after the items,
//! whose fields are typed into a draft that *Add* makes a new item.
//!
//! The blank row is drawn for the item at [`BLANK_ROW`] of the array, an
//! index no item has, so what is typed there, and the text being typed, are
//! kept by paths that never meet an item's, whatever is added or deleted
//! meanwhile.

use egui::RichText;
use gossia_calc::entry::{self, Entered};
use gossia_core::form::canvas::RowAllows;
use gossia_core::form::{Control, ControlKind};
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
