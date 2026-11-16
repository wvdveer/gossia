//! Canvas forms in the form view (DESIGN.md, Later versions, "Form
//! canvases, 0.7.0"): the stack of section canvases, each as wide as the
//! form, the rows of each repeater section, the current row, and context
//! fields.
//!
//! A repeater section draws one row for each item, its canvas at the row's
//! height, one under another; only the rows on the screen are drawn, the
//! others only take their room, so a long array scrolls as a short one. A
//! press in a row, or the focus going into one, makes it the current row.
//! In layout mode the sections are canvases one under another, each with
//! its name in a strip on its left, the line under it dragged for its
//! height and the right edge of the stack for the form's width; a repeater
//! section is drawn as its first row, which is what is designed. The
//! controls on them are the form view's own widgets, moved and sized by the
//! designer of Free groups.

use egui::{Align, RichText, Sense, Stroke, StrokeKind, pos2, vec2};
use gossia_core::form::canvas::{ContextType, FORM_WIDTHS, MIN_ROW_HEIGHT, Section};
use gossia_core::form::{CONTROL_HEIGHTS, Control, ControlKind, FormDesign, LabelPosition, truthy};
use gossia_core::{Node, Path, Segment, TypeSpec, Value};
use gossia_i18n::tr;

use super::{
    Draft, DropZone, FormAction, LABEL_WIDTH, Painter, Slot, error_color, label_cell, slots,
    snapped, value_text,
};
use crate::form_layout::ControlAction;

/// The width of the mark on the left of the current row.
const CURRENT_MARK: f32 = 3.0;

/// The width of the strip on the left of the sections, with their names.
const STRIP: f32 = 140.0;

/// The room between two sections, which holds the line dragged for the
/// height of the one above.
const SECTION_GAP: f32 = 8.0;

/// How tall a hidden section (of height 0) is drawn in layout mode.
const HIDDEN_HEIGHT: f32 = 12.0;

/// The name of a section in the designer: *Form header*, *Repeater: lines
/// (row2)*, *Section*, *Form footer*.
pub(crate) fn section_title(design: &FormDesign, index: usize) -> String {
    match design.section(index) {
        Some(Section::Repeater(_)) => tr!(
            "Repeater: {path} ({row})",
            path = design.controls[index].path.as_deref().unwrap_or(""),
            row = design.row_name_of(index).unwrap_or_default()
        ),
        Some(section) => section.label().to_owned(),
        None => String::new(),
    }
}

/// *Add repeater*, asking for the array: a menu of the arrays of the
/// schema that are not inside another array. Gives the one chosen.
pub(crate) fn add_repeater_menu(ui: &mut egui::Ui, spec: &TypeSpec) -> Option<String> {
    let arrays = gossia_core::form::bindable(spec, ControlKind::Repeater);
    if arrays.is_empty() {
        ui.add_enabled(false, egui::Button::new(tr!("Add repeater")))
            .on_disabled_hover_text(tr!("The schema has no arrays to repeat."));
        return None;
    }
    let mut chosen = None;
    ui.menu_button(tr!("Add repeater"), |ui| {
        ui.weak(tr!("A row for each item of:"));
        for path in arrays {
            let text = path.to_string();
            if ui.button(&text).clicked() {
                chosen = Some(text);
                ui.close();
            }
        }
    });
    chosen
}

/// The key of the draft of a context field, which no node of a tab can have.
fn draft_path(name: &str) -> Path {
    Path::root().key("\u{1}context").key(name)
}

impl Painter<'_> {
    /// The sections of a canvas form, one under another, each as wide as
    /// the form. A fixed section with a height of 0 is hidden, except in
    /// layout mode.
    pub(super) fn canvas(&mut self, ui: &mut egui::Ui, design: &FormDesign) {
        self.prepare_repeaters(design);
        if self.layout.is_some() {
            self.designer_stack(ui, design);
            return;
        }
        let width = design.form_width() as f32;
        let all = slots(&design.controls, &[]);
        for (index, (c, slot)) in design.controls.iter().zip(all).enumerate() {
            match design.section(index) {
                Some(Section::Repeater(_)) => {
                    let name = design.row_name_of(index).unwrap_or_default();
                    self.repeater_section(ui, c, &slot, &name, width);
                }
                _ => {
                    if c.height == Some(0) {
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
        let Some(array) = array else {
            return;
        };
        let edit = self.row_edit_of(c, &array);
        let blank = usize::from(edit.allows.adds() && edit.editable);
        if len == 0 && blank == 0 {
            self.in_width(ui, width, |_, ui| {
                ui.add_space(4.0);
                ui.weak(tr!("No items"));
            });
            return;
        }
        let total = len + blank;
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(width, height * total as f32),
            egui::Sense::hover(),
        );
        let clip = ui.clip_rect();
        let first = (((clip.top() - rect.top()) / height).floor().max(0.0)) as usize;
        let last = ((((clip.bottom() - rect.top()) / height).ceil()).max(0.0) as usize).min(total);
        let outer_edit = self.row_edit.replace(edit);
        if blank == 1 {
            let row = egui::Rect::from_min_size(
                egui::pos2(rect.left(), rect.top() + height * len as f32),
                egui::vec2(width, height),
            );
            if (first..last).contains(&len) {
                self.blank_row(ui, c, slot, &array, row_name, row);
            } else if self.wants_blank_focus(&array) {
                // Off the screen, so not drawn: scrolled to first.
                ui.scroll_to_rect(row, None);
            }
        }
        let current = match self.contexts.as_ref().and_then(|c| c.get(row_name)) {
            Some(Value::Int(i)) => Some(*i as usize),
            _ => None,
        };
        // Going to a node inside a row (from the data tree, or the Issues
        // pane): that row becomes the current one and is scrolled to, since
        // a row off the screen is not drawn to scroll to itself.
        if let Some(wanted) = &self.scroll_to
            && wanted.len() > array.len()
            && wanted.segments().starts_with(array.segments())
            && let gossia_core::Segment::Index(i) = wanted.segments()[array.len()]
            && i < len
        {
            let row = egui::Rect::from_min_size(
                egui::pos2(rect.left(), rect.top() + height * i as f32),
                egui::vec2(width, height),
            );
            if !clip.contains_rect(row) {
                ui.scroll_to_rect(row, Some(Align::Center));
            }
            // Also when it is current already: the action keeps the node
            // looked for until its row is drawn (a choice that changes
            // nothing does nothing).
            self.actions.push(FormAction::SetContext(
                row_name.to_owned(),
                Value::Int(i as i64),
            ));
        }
        let pressed = ui.input(|i| {
            i.pointer
                .press_origin()
                .filter(|_| i.pointer.primary_pressed())
        });
        // The row a rows button has just added, off the screen: scrolled to,
        // to be drawn and focused in the next frame.
        if let Some(i) = (0..len)
            .rev()
            .find(|&i| self.wants_row_focus(&array.index(i)))
            && !(first..last).contains(&i)
        {
            let row = egui::Rect::from_min_size(
                egui::pos2(rect.left(), rect.top() + height * i as f32),
                egui::vec2(width, height),
            );
            ui.scroll_to_rect(row, None);
        }
        let mut drawn = Vec::new();
        for i in first..last.max(first).min(len) {
            let row = egui::Rect::from_min_size(
                egui::pos2(rect.left(), rect.top() + height * i as f32),
                egui::vec2(width, height),
            );
            let mut row_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(row)
                    .layout(egui::Layout::top_down(Align::Min)),
            );
            let stops = self.nav.len();
            self.in_row(
                &mut row_ui,
                Some(array.index(i)),
                i,
                row_name,
                |this, ui| {
                    this.section_row(ui, c, slot);
                },
            );
            if self.wants_row_focus(&array.index(i)) {
                self.focus_first(ui, stops, row);
                self.actions.push(FormAction::RowFocused);
            }
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
        self.row_edit = outer_edit;
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

    /// The canvas of a repeater section for the blank row after its items,
    /// whose fields are typed into its draft, and whose add/delete button
    /// is *Add*.
    fn blank_row(
        &mut self,
        ui: &mut egui::Ui,
        c: &Control,
        slot: &Slot,
        array: &Path,
        row_name: &str,
        row: egui::Rect,
    ) {
        let item = array.index(super::rows::BLANK_ROW);
        let node = self
            .app
            .form
            .row_drafts
            .get(&(self.app.sheet.clone(), array.clone()))
            .cloned()
            .unwrap_or_else(super::rows::empty_draft);
        let outer = self.blank.replace(super::rows::Blank {
            item: item.clone(),
            node,
        });
        let actions = self.actions.len();
        let stops = self.nav.len();
        let mut row_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(row)
                .layout(egui::Layout::top_down(Align::Min)),
        );
        self.in_row(
            &mut row_ui,
            Some(item),
            super::rows::BLANK_ROW,
            row_name,
            |this, ui| this.section_row(ui, c, slot),
        );
        self.blank = outer;
        // What was typed there goes into the draft; nothing else it asked
        // for is for a row that is not there.
        let asked: Vec<FormAction> = self.actions.drain(actions..).collect();
        for action in asked {
            match action {
                FormAction::Commit(..) | FormAction::Direct(..) | FormAction::Create(..) => self
                    .actions
                    .push(FormAction::InDraft(array.clone(), Box::new(action))),
                FormAction::AddRow(_) | FormAction::Control(_) => self.actions.push(action),
                _ => {}
            }
        }
        if self.wants_blank_focus(array) {
            self.focus_first(ui, stops, row);
            self.actions.push(FormAction::BlankFocused);
        }
    }

    /// The sections in layout mode: each a canvas with a grid, its name in
    /// the strip on its left (a click selects the section, its menu adds
    /// and removes repeaters), the line under it dragged for its height; the
    /// right edge of the stack is dragged for the form's width. A click on
    /// the form outside them selects nothing, for the form's own properties.
    fn designer_stack(&mut self, ui: &mut egui::Ui, design: &FormDesign) {
        let background = ui.interact(
            ui.clip_rect(),
            egui::Id::unique(("form_background", &self.app.sheet, self.form)),
            Sense::click(),
        );
        if background.clicked() {
            self.actions
                .push(FormAction::Control(ControlAction::SelectNone));
        }
        let width = design.form_width() as f32;
        let top = ui.cursor().top();
        let left = ui.cursor().left();
        let mut bottom = top;
        let all = slots(&design.controls, &[]);
        for (index, (c, slot)) in design.controls.iter().zip(all).enumerate() {
            let Some(section) = design.section(index) else {
                continue;
            };
            let height = c.height.unwrap_or(0) as f32;
            let hidden = height == 0.0 && !section.is_repeater();
            let shown = if hidden {
                HIDDEN_HEIGHT
            } else if section.is_repeater() {
                height.max(MIN_ROW_HEIGHT as f32)
            } else {
                height
            };
            let (rect, _) = ui.allocate_exact_size(vec2(STRIP + width, shown), Sense::hover());
            let strip =
                egui::Rect::from_min_max(rect.min, pos2(rect.left() + STRIP, rect.bottom()));
            let region = egui::Rect::from_min_max(pos2(rect.left() + STRIP, rect.top()), rect.max);
            let mut name = section_title(design, index);
            if hidden {
                name = tr!("{name} (hidden)", name = name);
            }
            self.section_sheet(ui, region, index, hidden);
            self.section_strip(ui, design, strip, region, index, &name);
            let mut inside = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(region)
                    .layout(egui::Layout::top_down(Align::Min)),
            );
            if section.is_repeater() {
                // The first row, which is what is designed.
                let row_name = design.row_name_of(index).unwrap_or_default();
                let item = self.own_path(c).map(|a| a.index(0));
                self.in_row(&mut inside, item, 0, &row_name, |this, ui| {
                    this.section_controls(ui, c, &slot, region);
                });
            } else {
                self.section_controls(&mut inside, c, &slot, region);
            }
            self.section_line(ui, region, (index, height), section, &name);
            ui.add_space((SECTION_GAP - ui.spacing().item_spacing.y).max(0.0));
            bottom = rect.bottom();
        }
        self.width_edge(ui, left + STRIP, top, bottom, width);
    }

    /// The sheet of a section: its colour and grid, a line every 16 points,
    /// or a grey bar for a hidden one. A click on it, where no control is,
    /// selects the section.
    fn section_sheet(&mut self, ui: &mut egui::Ui, region: egui::Rect, index: usize, hidden: bool) {
        let visuals = ui.visuals();
        let border = visuals.widgets.noninteractive.bg_stroke.color;
        let painter = ui.painter();
        if hidden {
            painter.rect_filled(region, 0.0, border.gamma_multiply(0.3));
        } else {
            painter.rect_filled(region, 0.0, visuals.extreme_bg_color);
            let faint = Stroke::new(0.5, border.gamma_multiply(0.4));
            let step = 16.0;
            let mut x = region.left() + step;
            while x < region.right() {
                painter.vline(x, region.y_range(), faint);
                x += step;
            }
            let mut y = region.top() + step;
            while y < region.bottom() {
                painter.hline(region.x_range(), y, faint);
                y += step;
            }
        }
        painter.rect_stroke(region, 0.0, Stroke::new(1.0, border), StrokeKind::Inside);
        let id = egui::Id::unique(("form_section_area", &self.app.sheet, self.form, index));
        if ui.interact(region, id, Sense::click()).clicked() {
            self.actions
                .push(FormAction::Control(ControlAction::Select(vec![index])));
        }
    }

    /// The name of a section in the strip on its left: a click selects the
    /// section; its menu adds a repeater, and removes a repeater section.
    fn section_strip(
        &mut self,
        ui: &mut egui::Ui,
        design: &FormDesign,
        strip: egui::Rect,
        region: egui::Rect,
        index: usize,
        name: &str,
    ) {
        let id = egui::Id::unique(("form_section_strip", &self.app.sheet, self.form, index));
        let response = ui.interact(strip, id, Sense::click());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, name));
        let selected = self.layout.as_ref().and_then(Option::as_ref) == Some(&vec![index]);
        let problem = self.problem_of(&design.controls[index]);
        let colour = if problem.is_some() {
            error_color(ui)
        } else if selected {
            ui.visuals().strong_text_color()
        } else {
            ui.visuals().text_color()
        };
        let text = egui::Rect::from_min_max(
            strip.min,
            pos2(strip.right() - 6.0, strip.bottom().max(strip.top() + 14.0)),
        );
        ui.painter().with_clip_rect(text).text(
            pos2(text.right(), text.top()),
            egui::Align2::RIGHT_TOP,
            name,
            egui::FontId::proportional(11.0),
            colour,
        );
        if selected {
            ui.painter().rect_stroke(
                region.expand(2.0),
                0.0,
                Stroke::new(1.5, ui.visuals().selection.stroke.color),
                StrokeKind::Outside,
            );
        }
        if response.clicked() {
            self.actions
                .push(FormAction::Control(ControlAction::Select(vec![index])));
        }
        let response = match problem {
            Some(problem) => response.on_hover_text(problem),
            None => response,
        };
        let repeater = design.section(index).is_some_and(Section::is_repeater);
        response.context_menu(|ui| {
            if let Some(path) = add_repeater_menu(ui, self.spec) {
                self.actions
                    .push(FormAction::Control(ControlAction::AddRepeater(path)));
            }
            if repeater && ui.button(tr!("Remove repeater")).clicked() {
                self.actions
                    .push(FormAction::Control(ControlAction::RemoveRepeater(index)));
                ui.close();
            }
        });
    }

    /// The controls of a section, at their places in `region`, which is
    /// where a control is dropped or moved into it.
    fn section_controls(
        &mut self,
        ui: &mut egui::Ui,
        c: &Control,
        slot: &Slot,
        region: egui::Rect,
    ) {
        self.at = slot.at.clone();
        let outer = (self.sized, self.label_width);
        self.sized = false;
        self.label_width = c.label_width.or(self.label_width);
        self.free_in(ui, c, slot, region);
        (self.sized, self.label_width) = outer;
        self.rects.free.insert(slot.at.clone(), region);
        self.drops.push(DropZone {
            at: slot.at.clone(),
            rect: region,
            group: true,
            free: Some((region, c.children.len())),
            grid: None,
            section: true,
        });
    }

    /// The line under a section, dragged for its height: snapped to the
    /// grid (Alt drags without), not above its lowest control, at least the
    /// least height of a row for a repeater section; 0 hides a fixed one.
    /// Letting go sets it, one step of undo.
    fn section_line(
        &mut self,
        ui: &mut egui::Ui,
        region: egui::Rect,
        (index, stored): (usize, f32),
        section: Section,
        name: &str,
    ) {
        let line = egui::Rect::from_min_max(
            pos2(region.left(), region.bottom() - 3.0),
            pos2(region.right(), region.bottom() + SECTION_GAP),
        );
        let id = egui::Id::unique(("form_section_height", &self.app.sheet, self.form, index));
        let response = ui.interact(line, id, Sense::drag());
        let label = tr!("Height of {name}", name = name);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Splitter, true, &label));
        if response.hovered() || response.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
        }
        if response.drag_started()
            && let Some(p) = ui.input(|i| i.pointer.press_origin())
        {
            self.actions
                .push(FormAction::Control(ControlAction::ResizeStart));
            ui.data_mut(|d| d.insert_temp(id, (stored, p.y)));
        }
        let Some((start, grab)) = ui.data(|d| d.get_temp::<(f32, f32)>(id)) else {
            return;
        };
        let Some(p) = ui.input(|i| i.pointer.latest_pos()) else {
            return;
        };
        let lowest = self
            .last
            .controls
            .iter()
            .filter(|(a, _)| a.len() == 2 && a[0] == index)
            .map(|(_, r)| r.bottom() - region.top())
            .fold(0.0, f32::max);
        let least = if section.is_repeater() {
            MIN_ROW_HEIGHT
        } else {
            0
        };
        let snap = !ui.input(|i| i.modifiers.alt);
        let range = least..=*CONTROL_HEIGHTS.end();
        let height = snapped(start + p.y - grab, snap, &range).max(lowest.ceil() as u32);
        if response.dragged() {
            let painter = ui.ctx().layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::unique("form_guides"),
            ));
            let accent = ui.visuals().selection.stroke.color;
            let y = region.top() + height as f32;
            painter.hline(region.x_range(), y, Stroke::new(1.5, accent));
            painter.text(
                pos2(region.right() + 6.0, y),
                egui::Align2::LEFT_CENTER,
                format!("{height} pt"),
                egui::FontId::proportional(11.0),
                accent,
            );
        }
        if response.drag_stopped() {
            ui.data_mut(|d| d.remove::<(f32, f32)>(id));
            if height as f32 != start {
                self.actions.push(FormAction::Control(ControlAction::Resize(
                    vec![index],
                    None,
                    Some(Some(height)),
                )));
            }
        }
    }

    /// The right edge of the stack of sections, dragged for the width of
    /// the form, within 240 to 2000 points.
    fn width_edge(&mut self, ui: &mut egui::Ui, left: f32, top: f32, bottom: f32, width: f32) {
        let x = left + width;
        let edge = egui::Rect::from_x_y_ranges((x - 3.0)..=(x + 5.0), top..=bottom.max(top + 8.0));
        let id = egui::Id::unique(("form_width_edge", &self.app.sheet, self.form));
        let response = ui.interact(edge, id, Sense::drag());
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::Splitter, true, tr!("Width of the form"))
        });
        if response.hovered() || response.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        if response.drag_started()
            && let Some(p) = ui.input(|i| i.pointer.press_origin())
        {
            ui.data_mut(|d| d.insert_temp(id, p.x));
        }
        let Some(grab) = ui.data(|d| d.get_temp::<f32>(id)) else {
            return;
        };
        let Some(p) = ui.input(|i| i.pointer.latest_pos()) else {
            return;
        };
        let snap = !ui.input(|i| i.modifiers.alt);
        let to = snapped(width + p.x - grab, snap, &FORM_WIDTHS);
        if response.dragged() {
            let painter = ui.ctx().layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::unique("form_guides"),
            ));
            let accent = ui.visuals().selection.stroke.color;
            let x = left + to as f32;
            painter.vline(x, top..=bottom, Stroke::new(1.5, accent));
            painter.text(
                pos2(x + 6.0, top),
                egui::Align2::LEFT_TOP,
                format!("{to} pt"),
                egui::FontId::proportional(11.0),
                accent,
            );
        }
        if response.drag_stopped() {
            ui.data_mut(|d| d.remove::<f32>(id));
            if to as f32 != width {
                self.actions
                    .push(FormAction::Control(ControlAction::FormWidth(to)));
            }
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
            // The blank row is no row yet.
            let value = if i == super::rows::BLANK_ROW {
                Value::Null
            } else {
                Value::Int(i as i64)
            };
            contexts.insert(row_name.to_owned(), value);
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
            if let Segment::Index(i) = seg
                && *i != super::rows::BLANK_ROW
            {
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
