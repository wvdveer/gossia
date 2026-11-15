//! Canvas forms in the form view (DESIGN.md, Later versions, "Form
//! canvases, 0.7.0"): the stack of section canvases, each as wide as the
//! form, the rows of each repeater section, the current row, and context
//! fields.
//!
//! A repeater section draws one row for each item, its canvas at the row's
//! height, one under another; only the rows on the screen are drawn, the
//! others only take their room, so a long array scrolls as a short one. A
//! press in a row, or the focus going into one, makes it the current row.
//! In layout mode a repeater section is drawn as its first row, which is
//! what is designed.

use egui::{Align, RichText};
use gossia_core::form::canvas::{ContextType, Section};
use gossia_core::form::{Control, FormDesign, LabelPosition, truthy};
use gossia_core::{Node, Path, Segment, Value};
use gossia_i18n::tr;

use super::{
    Draft, FormAction, LABEL_WIDTH, Painter, Slot, error_color, label_cell, slots, value_text,
};

/// The width of the mark on the left of the current row.
const CURRENT_MARK: f32 = 3.0;

/// The key of the draft of a context field, which no node of a tab can have.
fn draft_path(name: &str) -> Path {
    Path::root().key("\u{1}context").key(name)
}

impl Painter<'_> {
    /// The sections of a canvas form, one under another, each as wide as
    /// the form. A fixed section with a height of 0 is hidden, except in
    /// layout mode.
    pub(super) fn canvas(&mut self, ui: &mut egui::Ui, design: &FormDesign) {
        let width = design.form_width() as f32;
        let all = slots(&design.controls, &[]);
        for (index, (c, slot)) in design.controls.iter().zip(all).enumerate() {
            match design.section(index) {
                Some(Section::Repeater(_)) => {
                    let name = design.row_name_of(index).unwrap_or_default();
                    self.repeater_section(ui, c, &slot, &name, width);
                }
                _ => {
                    if c.height == Some(0) && self.layout.is_none() {
                        continue;
                    }
                    self.in_width(ui, width, |this, ui| this.control_gated(ui, c, &slot));
                }
            }
        }
    }

    /// Draws `body` in a box `width` points wide.
    fn in_width(
        &mut self,
        ui: &mut egui::Ui,
        width: f32,
        body: impl FnOnce(&mut Self, &mut egui::Ui),
    ) {
        ui.allocate_ui_with_layout(
            egui::vec2(width, 0.0),
            egui::Layout::top_down(Align::Min),
            |ui| {
                ui.set_width(width);
                body(self, ui);
            },
        );
    }

    /// A repeater section: a row for each item of its array, the current one
    /// marked on its left.
    fn repeater_section(
        &mut self,
        ui: &mut egui::Ui,
        c: &Control,
        slot: &Slot,
        row_name: &str,
        width: f32,
    ) {
        if let Some(problem) = self.problem_of(c) {
            self.in_width(ui, width, |this, ui| this.placeholder(ui, c, &problem));
            return;
        }
        let array = self.own_path(c);
        let len = array
            .as_ref()
            .and_then(|a| self.data.get(a))
            .and_then(|n| n.value().as_array().map(<[Node]>::len))
            .unwrap_or(0);
        let height = c.height.unwrap_or(24).max(16) as f32;
        if self.layout.is_some() {
            // The first row, which is what is designed.
            let item = array.map(|a| a.index(0));
            self.in_width(ui, width, |this, ui| {
                this.in_row(ui, item, 0, row_name, |this, ui| {
                    this.section_row(ui, c, slot)
                });
            });
            return;
        }
        let Some(array) = array else {
            return;
        };
        if len == 0 {
            self.in_width(ui, width, |_, ui| {
                ui.add_space(4.0);
                ui.weak(tr!("No items"));
            });
            return;
        }
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(width, height * len as f32), egui::Sense::hover());
        let clip = ui.clip_rect();
        let first = (((clip.top() - rect.top()) / height).floor().max(0.0)) as usize;
        let last = ((((clip.bottom() - rect.top()) / height).ceil()).max(0.0) as usize).min(len);
        let current = match self.contexts.as_ref().and_then(|c| c.get(row_name)) {
            Some(Value::Int(i)) => Some(*i as usize),
            _ => None,
        };
        let pressed = ui.input(|i| {
            i.pointer
                .press_origin()
                .filter(|_| i.pointer.primary_pressed())
        });
        let mut drawn = Vec::new();
        for i in first..last.max(first) {
            let row = egui::Rect::from_min_size(
                egui::pos2(rect.left(), rect.top() + height * i as f32),
                egui::vec2(width, height),
            );
            let mut row_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(row)
                    .layout(egui::Layout::top_down(Align::Min)),
            );
            self.in_row(
                &mut row_ui,
                Some(array.index(i)),
                i,
                row_name,
                |this, ui| {
                    this.section_row(ui, c, slot);
                },
            );
            if current == Some(i) {
                let mark = egui::Rect::from_min_size(
                    row.min + egui::vec2(0.0, 2.0),
                    egui::vec2(CURRENT_MARK, (height - 4.0).max(1.0)),
                );
                ui.painter()
                    .rect_filled(mark, 1.0, ui.visuals().selection.bg_fill);
            }
            if pressed.is_some_and(|p| row.contains(p) && clip.contains(p)) && current != Some(i) {
                self.actions.push(FormAction::SetContext(
                    row_name.to_owned(),
                    Value::Int(i as i64),
                ));
            }
            drawn.push((i, row));
        }
        // The focus gone into another row (Tab, the arrow keys).
        let focused = ui
            .memory(|m| m.focused())
            .and_then(|id| ui.ctx().read_response(id))
            .map(|r| r.rect.center());
        if let Some(at) = focused
            && let Some((i, _)) = drawn.iter().find(|(_, r)| r.contains(at))
            && current != Some(*i)
            && pressed.is_none()
        {
            self.actions.push(FormAction::SetContext(
                row_name.to_owned(),
                Value::Int(*i as i64),
            ));
        }
    }

    /// Draws `body` as row `i` of a repeater section: its paths read from
    /// `item`, its expressions for that row, its row name that row.
    fn in_row(
        &mut self,
        ui: &mut egui::Ui,
        item: Option<Path>,
        i: usize,
        row_name: &str,
        body: impl FnOnce(&mut Self, &mut egui::Ui),
    ) {
        let outer_item = std::mem::replace(&mut self.item, item);
        let outer_row = self.row.replace((row_name.to_owned(), i));
        let outer_contexts = self.contexts.clone();
        if let Some(contexts) = &mut self.contexts {
            contexts.insert(row_name.to_owned(), Value::Int(i as i64));
        }
        self.rows.push(i);
        body(self, ui);
        self.rows.pop();
        self.contexts = outer_contexts;
        self.row = outer_row;
        self.item = outer_item;
    }

    /// The canvas of a repeater section for one row: a Free canvas of the
    /// row's height.
    fn section_row(&mut self, ui: &mut egui::Ui, c: &Control, slot: &Slot) {
        self.at = slot.at.clone();
        self.free_children(ui, c, slot);
    }

    /// A context field: an editor of its type, whose value belongs to the
    /// form. What is typed is entered when the field loses the focus, if its
    /// type reads it.
    pub(super) fn context_field(&mut self, ui: &mut egui::Ui, c: &Control) {
        let Some(name) = c.context.clone() else {
            return;
        };
        let kind = c.context_kind();
        let value = self
            .contexts
            .as_ref()
            .and_then(|m| m.get(&name))
            .cloned()
            .unwrap_or(Value::Null);
        let label = c.label.clone().unwrap_or_else(|| name.clone());
        let key = (self.app.sheet.clone(), draft_path(&name));
        let width = match c.label_position {
            Some(LabelPosition::None) => 0.0,
            _ => c
                .label_width
                .or(self.label_width)
                .map_or(LABEL_WIDTH, |w| w as f32),
        };
        let top = c.label_position == Some(LabelPosition::Top);
        let body = |this: &mut Self, ui: &mut egui::Ui| {
            if kind == ContextType::Boolean {
                let mut on = truthy(&value);
                let r = ui.checkbox(&mut on, "");
                let accessible = format!("context.{name}");
                r.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::Role::CheckBox, true, &accessible)
                });
                if r.changed() {
                    this.actions
                        .push(FormAction::SetContext(name.clone(), Value::Bool(on)));
                }
                return;
            }
            let shown = value_text(&value);
            let mut text = this
                .drafts
                .get(&key)
                .map_or_else(|| shown.clone(), |d| d.text.clone());
            let editor_width = if this.sized {
                ui.available_width()
            } else {
                (ui.available_width()).min(360.0)
            };
            let response = ui.add(
                egui::TextEdit::singleline(&mut text)
                    .desired_width(editor_width)
                    .hint_text(kind.label()),
            );
            let accessible = format!("context.{name}");
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::TextInput, true, &accessible)
            });
            if response.changed() {
                let error = kind.read(&text).err();
                this.drafts.insert(
                    key.clone(),
                    Draft {
                        text: text.clone(),
                        error,
                    },
                );
            }
            let error = this.drafts.get(&key).and_then(|d| d.error.clone());
            if response.lost_focus()
                && let Some(draft) = this.drafts.get(&key).cloned()
                && let Ok(v) = kind.read(&draft.text)
            {
                this.drafts.remove(&key);
                if v != value {
                    this.actions.push(FormAction::SetContext(name.clone(), v));
                }
            }
            if let Some(error) = error {
                response.on_hover_text(RichText::new(&error).color(error_color(ui)));
            }
        };
        let help = c.help.clone();
        if top {
            ui.label(self.text(label));
            body(self, ui);
        } else {
            ui.horizontal(|ui| {
                label_cell(ui, self.text(label), false, help.as_deref(), width);
                body(self, ui);
            });
        }
    }

    /// Why the node of a bound control is not there to edit, on a canvas
    /// form: its path could not be bound with the contexts, or it goes past
    /// the end of an array. None when it can be edited (an absent optional
    /// node is made when a value is entered).
    pub(super) fn unbound(&self, c: &Control) -> Option<String> {
        let contexts = self.contexts.as_ref()?;
        let text = c.path.as_deref()?;
        let parsed: gossia_core::form::canvas::ContextPath = text.parse().ok()?;
        if !parsed.has_context() {
            return None;
        }
        let path = match parsed.bind(|n| contexts.get(n).cloned()) {
            Ok(own) => match &self.item {
                Some(item) => item.join(&own),
                None => own,
            },
            Err(why) => return Some(why),
        };
        let mut at = Path::root();
        for seg in path.segments() {
            if let Segment::Index(i) = seg {
                let len = self
                    .data
                    .get(&at)
                    .and_then(|n| n.value().as_array().map(<[Node]>::len));
                if len.is_some_and(|len| *i >= len) {
                    return Some(tr!("{path} is past the end of its list", path = path));
                }
            }
            at.push(seg.clone());
        }
        None
    }

    /// A field whose node is not there to edit: its label and an empty,
    /// disabled box saying why on hover.
    pub(super) fn unbound_field(&mut self, ui: &mut egui::Ui, c: &Control, why: &str) {
        let label = c.label.clone().unwrap_or_default();
        let width = match c.label_position {
            Some(LabelPosition::None) => 0.0,
            _ => c
                .label_width
                .or(self.label_width)
                .map_or(LABEL_WIDTH, |w| w as f32),
        };
        ui.horizontal(|ui| {
            label_cell(ui, self.text(label), false, c.help.as_deref(), width);
            let mut empty = String::new();
            let editor_width = if self.sized {
                ui.available_width()
            } else {
                ui.available_width().min(360.0)
            };
            let r = ui.add_enabled(
                false,
                egui::TextEdit::singleline(&mut empty).desired_width(editor_width),
            );
            // Named by its path as written, since it has no node.
            let accessible = format!("{}.{}", self.app.sheet, c.path.as_deref().unwrap_or(""));
            r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, false, &accessible));
            r.on_disabled_hover_text(why);
        });
    }
}
