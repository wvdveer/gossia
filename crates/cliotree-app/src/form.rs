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
use cliotree_core::validate::validate_at;
use cliotree_core::{Kind, Node, Path, TypeId, TypeSpec, Value};
use egui::{Align, Color32, Key, RichText, Sense, Stroke, StrokeKind};
use egui_extras::{Column as TableColumn, TableBuilder};

use crate::app::ClioApp;
use crate::icons;
use crate::rows::display;
use crate::table::{self, Column};
use crate::view::{ROW_HEIGHT, computed_fill, error_color};

/// The form every Data Tab has. It cannot be renamed or deleted.
pub const DEFAULT_FORM: &str = "Default";

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

    /// The names of the views of the current tab, the data tree first.
    pub fn view_names(&self) -> Vec<View> {
        vec![View::Tree, View::Form(DEFAULT_FORM.to_owned()), View::Raw]
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

    /// The generated Default form, filling the central panel.
    pub(crate) fn form_view(&mut self, ui: &mut egui::Ui) {
        if let Some(notice) = &self.notice {
            let color = error_color(ui);
            ui.colored_label(color, notice);
        }
        let mut drafts = std::mem::take(&mut self.form.drafts);
        let mut new_keys = std::mem::take(&mut self.form.new_keys);
        let scroll_to = self.form.scroll_to.take();
        let mut actions = Vec::new();
        {
            let calc = self.calc();
            if let (Some(spec), Some(data)) = (calc.spec(&self.sheet), calc.data(&self.sheet)) {
                let mut painter = Painter {
                    app: self,
                    spec,
                    data,
                    drafts: &mut drafts,
                    new_keys: &mut new_keys,
                    actions: Vec::new(),
                    scroll_to,
                };
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let title = spec
                            .get(spec.root())
                            .title
                            .clone()
                            .unwrap_or_else(|| self.sheet.clone());
                        painter.node(ui, &Path::root(), spec.root(), &title, true);
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
    spec: &'a TypeSpec,
    data: &'a Node,
    drafts: &'a mut HashMap<(String, Path), Draft>,
    new_keys: &'a mut HashMap<(String, Path), String>,
    actions: Vec<FormAction>,
    scroll_to: Option<Path>,
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

    fn help(&self, ty: TypeId) -> Option<&str> {
        self.spec.get(ty).description.as_deref()
    }

    fn object(&mut self, ui: &mut egui::Ui, path: &Path, ty: TypeId, label: &str, required: bool) {
        let Kind::Object { fields } = &self.spec.get(ty).kind else {
            return;
        };
        let root = path.is_root();
        if self.data.get(path).is_none() {
            // Absent: an empty control, and the object is made on request.
            ui.horizontal(|ui| {
                label_cell(ui, label, required, self.help(ty));
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
            if let Some(help) = self.help(ty) {
                ui.label(RichText::new(help).weak());
            }
            ui.add_space(4.0);
            draw(self, ui);
        } else {
            ui.group(|ui| {
                ui.set_width(ui.available_width());
                let response = ui.strong(label);
                self.arrive(path, &response);
                if let Some(help) = self.help(ty) {
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
                label_cell(ui, label, required, self.help(ty));
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
            (_, Kind::Bool) => {
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
            (_, Kind::Enum { values, .. }) => {
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
                let mut text = self
                    .drafts
                    .get(&key)
                    .map_or_else(|| current.clone(), |d| d.text.clone());
                let long = matches!(kind, Kind::String(c) if c.max_length.is_some_and(|m| m > 200));
                let width = if compact {
                    f32::INFINITY
                } else {
                    ui.available_width().min(360.0)
                };
                let edit = if long {
                    egui::TextEdit::multiline(&mut text).desired_rows(3)
                } else {
                    egui::TextEdit::singleline(&mut text)
                };
                // Numbers line up on the right.
                let align = if matches!(kind, Kind::Integer(_) | Kind::Number(_)) {
                    egui::Align2::RIGHT_CENTER
                } else {
                    egui::Align2::LEFT_CENTER
                };
                let r = ui.add(
                    edit.desired_width(width)
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
                    Stroke::new(1.5, Color32::from_rgb(220, 60, 60)),
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
            ui.label(RichText::new("…").weak())
                .on_hover_text("Waiting to be calculated");
        }
        problem
    }

    // ----------------------------------------------------------- repeaters

    fn repeater(&mut self, ui: &mut egui::Ui, path: &Path, ty: TypeId, label: &str) {
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
                let response = ui.strong(label);
                self.arrive(path, &response);
                ui.label(RichText::new(count_text(items.len())).weak());
                if ui.add_enabled(can_add, egui::Button::new("New")).clicked() {
                    self.actions.push(FormAction::Add(path.clone()));
                }
            });
            if let Some(help) = self.help(ty) {
                ui.label(RichText::new(help).weak().small());
            }
            match self.table_columns(item_ty) {
                Some(columns) if !items.is_empty() => {
                    self.table(ui, path, items.len(), &columns);
                }
                Some(_) => {}
                None => self.stacked(ui, path, item_ty, items.len()),
            }
        });
    }

    /// The columns of a repeater drawn as a table: its items are scalars, or
    /// flat objects (no array or map inside). None for any other item.
    fn table_columns(&self, item_ty: TypeId) -> Option<Vec<Column>> {
        match &self.spec.get(item_ty).kind {
            Kind::Object { .. } => {
                let columns = table::columns(self.spec, item_ty);
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

    fn table(&mut self, ui: &mut egui::Ui, array: &Path, len: usize, columns: &[Column]) {
        let editable = self.app.can_reorder(array);
        let sortable = editable && columns.iter().any(|c| !c.fields.is_empty());
        let mut table = TableBuilder::new(ui)
            .id_salt(("form_table", &self.app.sheet, array.to_string()))
            .striped(true)
            .vscroll(false)
            .cell_layout(egui::Layout::left_to_right(Align::Center))
            .column(TableColumn::auto().at_least(96.0));
        for _ in columns {
            table = table.column(TableColumn::initial(140.0).at_least(60.0).clip(true));
        }
        let actions = &mut self.actions;
        table
            .header(22.0, |mut header| {
                header.col(|ui| {
                    ui.strong("#");
                });
                for column in columns {
                    header.col(|ui| {
                        let text = RichText::new(&column.name).strong();
                        if sortable {
                            let response = ui
                                .add(egui::Label::new(text).sense(Sense::CLICK))
                                .on_hover_text("Sort by this column");
                            if response.clicked() {
                                actions.push(FormAction::Sort(array.clone(), column.name.clone()));
                            }
                        } else {
                            ui.label(text);
                        }
                    });
                }
            })
            .body(|body| {
                body.rows(ROW_HEIGHT, len, |mut row| {
                    let index = row.index();
                    row.col(|ui| {
                        ui.label(RichText::new(format!("[{index}]")).weak());
                        if icons::button(ui, editable && index > 0, icons::ARROW_UP, "^", "Move up")
                            .clicked()
                        {
                            self.actions
                                .push(FormAction::Move(array.clone(), index, true));
                        }
                        if icons::button(
                            ui,
                            editable && index + 1 < len,
                            icons::ARROW_DOWN,
                            "v",
                            "Move down",
                        )
                        .clicked()
                        {
                            self.actions
                                .push(FormAction::Move(array.clone(), index, false));
                        }
                        if icons::button(ui, editable, icons::X, "x", "Delete row").clicked() {
                            self.actions.push(FormAction::Delete(array.clone(), index));
                        }
                    });
                    for column in columns {
                        row.col(|ui| {
                            let cell = column.cell_path(array, index);
                            self.editor(ui, &cell, column.ty, column.required, true);
                        });
                    }
                });
            });
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
            if let Some(help) = self.help(ty) {
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
