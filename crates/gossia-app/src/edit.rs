//! Editing: what the window does when the user changes something. None of it
//! draws anything, so it is tested without a window.

use gossia_i18n::tr;
use std::collections::BTreeSet;

use gossia_calc::entry;
use gossia_calc::python::catalog::{Function, catalog};
use gossia_calc::python::completion::complete;
use gossia_core::{Kind, Num, Path, Segment, TypeId, TypeSpec, Value};

use crate::app::{CompletionUi, Editing, Focus, GossiaApp, InsertFunction};
use crate::replace_inside::Target;
use crate::rows::{Row, RowKind, display};

impl GossiaApp {
    // ----------------------------------------------------- what a cell says

    /// The text of a cell as it is edited: `=` and the formula if it has one,
    /// otherwise the value. Text that starts with `=` or an apostrophe gets an
    /// apostrophe in front, so that it enters as the same text.
    pub fn cell_text(&self, path: &Path) -> String {
        let calc = self.session.calc();
        if let Some(formula) = calc.formula_text(&self.sheet, path) {
            return format!("={formula}");
        }
        match calc
            .data(&self.sheet)
            .and_then(|d| d.get(path))
            .map(|n| n.value())
        {
            Some(Value::Str(s)) if s.starts_with('=') || s.starts_with('\'') => {
                format!("'{s}")
            }
            Some(Value::Str(s)) => s.clone(),
            Some(Value::Array(_) | Value::Object(_) | Value::Error(_)) | None => String::new(),
            Some(other) => display(other),
        }
    }

    /// Whether the node can be edited by the user: the schema defines it, and
    /// it is not inside the result of a formula, which owns it.
    pub fn can_edit(&self, path: &Path) -> bool {
        let calc = self.session.calc();
        let defined = calc
            .spec(&self.sheet)
            .is_some_and(|spec| spec.resolve(path).is_some());
        defined
            && calc
                .cell(&self.sheet, path)
                .is_some_and(|cell| cell.computed_by.is_none())
    }

    // ---------------------------------------------------------- editing

    /// Starts editing the selected cell, with its own text or `initial`.
    pub fn begin_editing(&mut self, path: &Path, in_cell: bool, initial: Option<String>) -> bool {
        if self.picking() {
            return false;
        }
        if !self.can_edit(path) {
            self.notice = Some(self.why_read_only(path));
            return false;
        }
        if self.editing.is_some() && !self.commit_editing() {
            return false;
        }
        let text = initial.unwrap_or_else(|| self.cell_text(path));
        self.editing = Some(Editing {
            sheet: self.sheet.clone(),
            path: path.clone(),
            cursor: text.chars().count(),
            text,
            focus: if in_cell { Focus::Cell } else { Focus::Bar },
            changed: false,
            in_cell,
            set_cursor: None,
            column: None,
        });
        self.notice = None;
        self.completion = CompletionUi::default();
        true
    }

    /// Starts editing the formula of the selected column, in the formula bar.
    pub fn begin_editing_column(&mut self) -> bool {
        if self.picking() {
            return false;
        }
        let Some(column) = self.column_selected().cloned() else {
            return false;
        };
        if let Some(why) = self.column_refusal(&column) {
            self.notice = Some(why);
            return false;
        }
        if self.editing.is_some() && !self.commit_editing() {
            return false;
        }
        let text = self.column_text(&column);
        self.editing = Some(Editing {
            sheet: self.sheet.clone(),
            path: column.place(),
            cursor: text.chars().count(),
            text,
            focus: Focus::Bar,
            changed: false,
            in_cell: false,
            set_cursor: None,
            column: Some((column.array, column.field)),
        });
        self.notice = None;
        self.completion = CompletionUi::default();
        true
    }

    fn why_read_only(&self, path: &Path) -> String {
        let name = self.name_of(path);
        match self.session.calc().cell(&self.sheet, path) {
            Some(cell) => match cell.computed_by {
                Some(by) => format!(
                    "{name} is computed by the formula at {}; change that formula instead",
                    self.name_of(&by)
                ),
                None => format!("{name} cannot be edited"),
            },
            None => format!("{name} cannot be edited"),
        }
    }

    pub fn cancel_editing(&mut self) {
        if self
            .replace_inside
            .as_ref()
            .is_some_and(|ask| !ask.from_form)
        {
            self.replace_inside = None;
        }
        self.editing = None;
        self.notice = None;
        self.completion = CompletionUi::default();
    }

    /// Enters what is being typed. Returns false, and keeps editing, if it was
    /// refused; the reason is in `notice`.
    pub fn commit_editing(&mut self) -> bool {
        let Some(editing) = self.editing.take() else {
            return true;
        };
        // A formula that would erase what is inside its node waits for the
        // answer to the question about it.
        let target = match &editing.column {
            Some((array, field)) => Target::Column(array.clone(), field.clone()),
            None => Target::Node(editing.path.clone()),
        };
        let sheet = editing.sheet.clone();
        let text = editing.text.clone();
        if self.replace_inside.is_some() || self.ask_before_erasing(&sheet, target, &text, false) {
            self.editing = Some(editing);
            return false;
        }
        let result = match &editing.column {
            Some((array, field)) => entry::enter_column(
                self.session.calc_mut(),
                &editing.sheet,
                array,
                field,
                &editing.text,
            ),
            None => entry::enter(
                self.session.calc_mut(),
                &editing.sheet,
                &editing.path,
                &editing.text,
                false,
            ),
        };
        match result {
            Ok(()) => {
                self.notice = None;
                self.completion = CompletionUi::default();
                if editing.column.is_none() {
                    self.pin_row_of(&editing.path);
                }
                self.note_unevaluated_formula(&editing.text);
                self.session.tick();
                true
            }
            Err(e) => {
                self.notice = Some(e.to_string());
                self.editing = Some(editing);
                false
            }
        }
    }

    /// Enters a literal at once, for the editors that have no text: a toggle
    /// for a boolean, a choice from a list.
    pub fn enter_directly(&mut self, path: &Path, text: &str) {
        let sheet = self.sheet.clone();
        match entry::enter(self.session.calc_mut(), &sheet, path, text, false) {
            Ok(()) => {
                self.notice = None;
                self.pin_row_of(path);
                self.session.tick();
            }
            Err(e) => self.notice = Some(e.to_string()),
        }
    }

    // ------------------------------------------------------ the selection

    /// Selects a node, entering what was being typed first. Returns false if
    /// that was refused.
    pub fn select(&mut self, path: &Path) -> bool {
        if self.editing.as_ref().is_some_and(|e| &e.path != path) && !self.commit_editing() {
            return false;
        }
        self.selection = Some(path.clone());
        self.tables.column = None;
        self.tables.anchor = None;
        self.note_selection_in_table(path);
        if self.editing.is_none() {
            self.notice = None;
        }
        true
    }

    /// Moves the selection by `delta` rows.
    pub fn move_selection(&mut self, delta: isize) {
        if self.rows.is_empty() {
            return;
        }
        let current = self
            .selection
            .as_ref()
            .and_then(|p| self.row_index(p))
            .map_or(-1, |i| i as isize);
        let next = (current + delta).clamp(0, self.rows.len() as isize - 1) as usize;
        let path = self.rows[next].path.clone();
        if self.select(&path) {
            self.scroll_to = Some(next);
        }
    }

    pub fn selected_row(&self) -> Option<&Row> {
        let path = self.selection.as_ref()?;
        self.rows.iter().find(|r| &r.path == path)
    }

    // ------------------------------------------------- structural edits

    /// A value for a new item or field: required fields filled in, the first
    /// choice of an enumeration, the lower bound of a range.
    pub fn default_value(spec: &TypeSpec, ty: TypeId) -> Value {
        match &spec.get(ty).kind {
            Kind::String(_) | Kind::Any => Value::Str(String::new()),
            Kind::Enum { values, .. } => Value::Str(values.first().cloned().unwrap_or_default()),
            Kind::Integer(range) => match range.min {
                Some(Num::Int(n)) => Value::Int(n),
                Some(Num::Float(x)) => Value::Int(x.ceil() as i64),
                None => Value::Int(0),
            },
            Kind::Number(range) => match range.min {
                Some(Num::Int(n)) => Value::Int(n),
                Some(Num::Float(x)) => Value::Float(x),
                None => Value::Int(0),
            },
            Kind::Bool => Value::Bool(false),
            Kind::Date => Value::Date(chrono::NaiveDate::from_ymd_opt(1970, 1, 1).expect("a date")),
            Kind::DateTime => Value::DateTime(gossia_core::DateTimeValue::Naive(
                chrono::NaiveDate::from_ymd_opt(1970, 1, 1)
                    .and_then(|d| d.and_hms_opt(0, 0, 0))
                    .expect("a date and time"),
            )),
            Kind::Array { .. } => Value::Array(Vec::new()),
            Kind::Map { .. } => Value::Object(Default::default()),
            Kind::Object { fields } => Value::Object(
                fields
                    .iter()
                    .filter(|f| f.required)
                    .map(|f| {
                        (
                            f.name.clone(),
                            gossia_core::Node::new(GossiaApp::default_value(spec, f.ty)),
                        )
                    })
                    .collect(),
            ),
        }
    }

    /// Adds an item at the end of the array at `array`, or at `index`.
    pub fn insert_item(&mut self, array: &Path, index: Option<usize>) {
        let sheet = self.sheet.clone();
        let calc = self.session.calc_mut();
        let Some(spec) = calc.spec(&sheet) else {
            return;
        };
        let item_type = match spec.resolve(array).map(|r| (r.ty, &spec.get(r.ty).kind)) {
            Some((_, Kind::Array { items, .. })) => *items,
            // A list held by an `any`: its items are `any` too.
            Some((ty, Kind::Any)) => ty,
            _ => return,
        };
        let value = GossiaApp::default_value(spec, item_type);
        let length = match calc
            .data(&sheet)
            .and_then(|d| d.get(array))
            .map(|n| n.value())
        {
            Some(Value::Array(items)) => items.len(),
            _ => 0,
        };
        let at = index.unwrap_or(length).min(length);
        match calc.insert_item(&sheet, array, at, value) {
            Ok(()) => {
                self.notice = None;
                self.set_open(array, true);
                self.session.tick();
            }
            Err(e) => self.notice = Some(e.to_string()),
        }
    }

    /// Removes the node at `path`: an item of an array, a key of a map, or a
    /// field of an object (which then shows as missing, if it is required).
    pub fn delete_node(&mut self, path: &Path) {
        let sheet = self.sheet.clone();
        if matches!(
            path.segments().last(),
            Some(Segment::Index(_) | Segment::Key(_))
        ) {
            match self.session.calc_mut().remove(&sheet, path) {
                Ok(()) => {
                    self.notice = None;
                    self.session.tick();
                }
                Err(e) => self.notice = Some(e.to_string()),
            }
        }
    }

    // ----------------------------------------------------- undo and redo

    pub fn undo(&mut self) {
        if self.editing.is_some() || self.session.running_procedure().is_some() {
            return;
        }
        // What was typed in the code editor is the entry undone first.
        self.commit_code();
        match self.session.calc_mut().undo() {
            Ok(_) => self.notice = None,
            Err(e) => self.notice = Some(e.to_string()),
        }
        self.follow_tabs();
        self.session.tick();
    }

    pub fn redo(&mut self) {
        if self.editing.is_some() || self.session.running_procedure().is_some() {
            return;
        }
        // Typing ends what can be redone, as any edit does.
        self.commit_code();
        match self.session.calc_mut().redo() {
            Ok(_) => self.notice = None,
            Err(e) => self.notice = Some(e.to_string()),
        }
        self.follow_tabs();
        self.session.tick();
    }

    // ------------------------------------------------------- completion

    /// Works out the names to offer for the text being typed.
    pub fn update_completion(&mut self) {
        let Some(editing) = &self.editing else {
            self.completion = CompletionUi::default();
            return;
        };
        let formula = editing.text.starts_with('=');
        if !formula || self.completion.dismissed_for.as_deref() == Some(editing.text.as_str()) {
            self.completion.items.clear();
            return;
        }
        // The formula starts after the `=`.
        let body = &editing.text[1..];
        let cursor_chars = editing.cursor.saturating_sub(1).min(body.chars().count());
        let cursor_bytes = body
            .char_indices()
            .nth(cursor_chars)
            .map_or(body.len(), |(i, _)| i);
        let found = complete(
            self.session.calc(),
            &editing.sheet,
            &editing.path,
            body,
            cursor_bytes,
        );
        // Nothing to offer for an empty word at the start of a name: wait for
        // the first letter, or a dot.
        let word = &body[found.start..cursor_bytes];
        let offer = !word.is_empty() || body[..found.start].ends_with('.');
        let items = if offer { found.items } else { Vec::new() };
        if items != self.completion.items {
            self.completion.selected = 0;
        }
        self.completion.start = found.start + 1;
        self.completion.items = items;
    }

    /// Puts the chosen name in place of the word being typed.
    pub fn accept_completion(&mut self, index: usize) {
        let Some(item) = self.completion.items.get(index).cloned() else {
            return;
        };
        let start = self.completion.start;
        if let Some(editing) = &mut self.editing {
            let chars: Vec<char> = editing.text.chars().collect();
            let start = start.min(chars.len());
            let cursor = editing.cursor.clamp(start, chars.len());
            let mut text: String = chars[..start].iter().collect();
            text.push_str(&item.insert);
            let after = text.chars().count();
            text.extend(&chars[cursor..]);
            editing.text = text;
            editing.cursor = after;
            editing.set_cursor = Some(after);
            editing.changed = true;
        }
        self.completion.items.clear();
    }

    // ------------------------------------------------- Insert Function

    /// Opens the dialog.
    pub fn open_insert_function(&mut self) {
        self.insert_function = Some(InsertFunction::default());
    }

    /// The functions that match the search, by group.
    pub fn functions_matching(search: &str) -> Vec<&'static Function> {
        let needle = search.trim().to_lowercase();
        catalog()
            .iter()
            .filter(|f| {
                needle.is_empty()
                    || f.name.to_lowercase().contains(&needle)
                    || gossia_i18n::tr(f.description)
                        .to_lowercase()
                        .contains(&needle)
            })
            .collect()
    }

    /// Writes `name(` at the cursor of the formula bar, starting a formula if
    /// there is none.
    pub fn insert_function_text(&mut self, insert: &str) {
        if self.editing.is_none() {
            let Some(path) = self.selection.clone() else {
                self.notice = Some(tr!("select a cell first").to_owned());
                return;
            };
            if !self.begin_editing(&path, false, Some("=".to_owned())) {
                return;
            }
        }
        if let Some(editing) = &mut self.editing {
            if !editing.text.starts_with('=') {
                editing.text = format!("={}", editing.text);
                editing.cursor += 1;
            }
            let chars: Vec<char> = editing.text.chars().collect();
            let at = editing.cursor.min(chars.len());
            let mut text: String = chars[..at].iter().collect();
            text.push_str(insert);
            let after = text.chars().count();
            text.extend(&chars[at..]);
            editing.text = text;
            editing.cursor = after;
            editing.set_cursor = Some(after);
            editing.changed = true;
            editing.focus = Focus::Bar;
        }
    }

    // ------------------------------------------------------------ rows

    /// The paths of the rows that have a problem: errors and invalid data.
    pub fn issue_paths(&self) -> BTreeSet<String> {
        let calc = self.session.calc();
        let mut out = BTreeSet::new();
        if let (Some(spec), Some(data)) = (calc.spec(&self.sheet), calc.data(&self.sheet)) {
            for issue in gossia_core::validate::validate(spec, data) {
                out.insert(issue.path.to_string());
            }
        }
        out
    }

    /// The problems in the current tab, for the issues panel: the nodes that
    /// hold an error, then the data that does not fit the schema.
    pub fn issue_list(&self) -> Vec<(Path, String)> {
        let calc = self.session.calc();
        let mut out = Vec::new();
        if let (Some(spec), Some(data)) = (calc.spec(&self.sheet), calc.data(&self.sheet)) {
            collect_errors(data, &Path::root(), &mut out);
            for issue in gossia_core::validate::validate(spec, data) {
                out.push((issue.path, issue.message));
            }
        }
        out.truncate(500);
        out
    }

    /// Opens everything above `path` and selects it.
    pub fn reveal(&mut self, path: &Path) {
        let mut at = Path::root();
        self.set_open(&at, true);
        for segment in path.segments() {
            at.push(segment.clone());
            if &at != path {
                self.set_open(&at, true);
            }
        }
        self.refresh_rows();
        if self.select(path) {
            self.scroll_to = self.row_index(path);
        }
    }

    pub fn is_container(row: &Row) -> bool {
        matches!(row.kind, RowKind::Object | RowKind::Array | RowKind::Map)
    }
}

fn collect_errors(node: &gossia_core::Node, path: &Path, out: &mut Vec<(Path, String)>) {
    match node.value() {
        Value::Error(e) => out.push((path.clone(), format!("{} {}", e.kind.code(), e.message))),
        Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                collect_errors(item, &path.index(i), out);
            }
        }
        Value::Object(fields) => {
            for (key, item) in fields {
                collect_errors(item, &path.key(key.as_str()), out);
            }
        }
        _ => {}
    }
}
