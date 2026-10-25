//! The tabular view as the window uses it: which arrays are tables, the view
//! state of each, and the commands on rows and columns. None of it draws, so
//! it is tested without a window (DESIGN.md, "Tabular view of arrays").

use std::collections::{BTreeSet, HashMap, HashSet};

use cliotree_core::{Node, NodeId, Path, Segment};

use crate::app::ClioApp;
use crate::table::{
    self, Column, Condition, Filter, TableView, Visible, distinct_values, item_type, sort_order,
    value_key,
};

/// What is drawn in the table pane, worked out again when the data or the view
/// state changes.
#[derive(Debug, Clone)]
pub struct Layout {
    pub array: Path,
    pub columns: Vec<Column>,
    /// The indexes into `columns` of the columns that are shown.
    pub shown: Vec<usize>,
    pub visible: Visible,
}

type LayoutKey = (String, Path, u64, u64);

/// A sort that did not happen, with the means to look at the rows involved.
#[derive(Debug, Clone)]
pub struct Refusal {
    pub array: Path,
    pub column: String,
    pub message: String,
    pub rows: Vec<usize>,
    /// The values the rows hold, as the filter's list names them.
    pub keys: BTreeSet<String>,
}

/// The kinds of condition the filter dialog offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    Nothing,
    Equals,
    NotEquals,
    Greater,
    Less,
    Between,
    BeginsWith,
    EndsWith,
    Contains,
    IsBlank,
}

impl Choice {
    pub const ALL: [Choice; 10] = [
        Choice::Nothing,
        Choice::Equals,
        Choice::NotEquals,
        Choice::Greater,
        Choice::Less,
        Choice::Between,
        Choice::BeginsWith,
        Choice::EndsWith,
        Choice::Contains,
        Choice::IsBlank,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Choice::Nothing => "no condition",
            Choice::Equals => "equals",
            Choice::NotEquals => "does not equal",
            Choice::Greater => "is greater than",
            Choice::Less => "is less than",
            Choice::Between => "is between",
            Choice::BeginsWith => "begins with",
            Choice::EndsWith => "ends with",
            Choice::Contains => "contains",
            Choice::IsBlank => "is blank",
        }
    }

    /// How many operands the condition needs.
    pub fn operands(self) -> usize {
        match self {
            Choice::Nothing | Choice::IsBlank => 0,
            Choice::Between => 2,
            _ => 1,
        }
    }

    fn of(condition: &Option<Condition>) -> (Choice, String, String) {
        match condition {
            None => (Choice::Nothing, String::new(), String::new()),
            Some(Condition::Equals(a)) => (Choice::Equals, a.clone(), String::new()),
            Some(Condition::NotEquals(a)) => (Choice::NotEquals, a.clone(), String::new()),
            Some(Condition::Greater(a)) => (Choice::Greater, a.clone(), String::new()),
            Some(Condition::Less(a)) => (Choice::Less, a.clone(), String::new()),
            Some(Condition::Between(a, b)) => (Choice::Between, a.clone(), b.clone()),
            Some(Condition::BeginsWith(a)) => (Choice::BeginsWith, a.clone(), String::new()),
            Some(Condition::EndsWith(a)) => (Choice::EndsWith, a.clone(), String::new()),
            Some(Condition::Contains(a)) => (Choice::Contains, a.clone(), String::new()),
            Some(Condition::IsBlank) => (Choice::IsBlank, String::new(), String::new()),
        }
    }

    fn condition(self, a: &str, b: &str) -> Option<Condition> {
        let (a, b) = (a.to_owned(), b.to_owned());
        match self {
            Choice::Nothing => None,
            Choice::Equals => Some(Condition::Equals(a)),
            Choice::NotEquals => Some(Condition::NotEquals(a)),
            Choice::Greater => Some(Condition::Greater(a)),
            Choice::Less => Some(Condition::Less(a)),
            Choice::Between => Some(Condition::Between(a, b)),
            Choice::BeginsWith => Some(Condition::BeginsWith(a)),
            Choice::EndsWith => Some(Condition::EndsWith(a)),
            Choice::Contains => Some(Condition::Contains(a)),
            Choice::IsBlank => Some(Condition::IsBlank),
        }
    }
}

/// The filter dialog of one column, while it is open.
#[derive(Debug, Clone)]
pub struct FilterDialog {
    pub array: Path,
    pub column: String,
    /// The distinct values of the column, for the list.
    pub distinct: Vec<String>,
    pub checked: BTreeSet<String>,
    pub search: String,
    pub choice: Choice,
    pub first: String,
    pub second: String,
    pub ignore_case: bool,
    /// Why the dialog could not be applied.
    pub error: Option<String>,
}

#[derive(Debug, Default)]
pub struct Tables {
    /// The arrays shown as tables, by tab and path.
    pub tabular: HashSet<(String, Path)>,
    pub views: HashMap<(String, Path), TableView>,
    /// The table in the pane, for each tab.
    pub active: HashMap<String, Path>,
    /// Bumped whenever view state changes.
    pub rev: u64,
    pub layout: Option<(LayoutKey, Layout)>,
    /// The last column clicked to sort and the direction it was sorted in, so
    /// that the next click on it reverses. This is not shown anywhere.
    pub last_sort: Option<(String, Path, String, bool)>,
    pub refused: Option<Refusal>,
    pub dialog: Option<FilterDialog>,
    /// The column selected by clicking its header, if one is.
    pub column: Option<ColumnSel>,
}

/// A whole column of a table, selected by its header.
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnSel {
    pub sheet: String,
    pub array: Path,
    /// The name of the column: a field, or fields joined by dots.
    pub field: String,
}

impl ColumnSel {
    /// This column of the first item: where the column's formula is read from
    /// (`this`, `parent`) and what names are offered for it.
    pub fn place(&self) -> Path {
        self.array.index(0).key(self.field.as_str())
    }
}

impl ClioApp {
    // ------------------------------------------------------------ which

    /// Selects the whole column. What was being typed is entered first.
    pub fn select_column(&mut self, array: &Path, name: &str) {
        if self.editing.is_some() && !self.commit_editing() {
            return;
        }
        self.selection = None;
        self.tables.active.insert(self.sheet.clone(), array.clone());
        let column = ColumnSel {
            sheet: self.sheet.clone(),
            array: array.clone(),
            field: name.to_owned(),
        };
        self.notice = self.column_refusal(&column);
        self.tables.column = Some(column);
    }

    /// The column selected in this tab.
    pub fn column_selected(&self) -> Option<&ColumnSel> {
        self.tables
            .column
            .as_ref()
            .filter(|c| c.sheet == self.sheet && self.is_tabular(&c.array))
    }

    /// Why this column cannot have a formula, if it cannot.
    pub fn column_refusal(&self, column: &ColumnSel) -> Option<String> {
        let found = self
            .columns_of(&column.array)
            .into_iter()
            .find(|c| c.name == column.field)?;
        if found.summary {
            Some(format!(
                "{} holds other values, so it cannot have a column formula",
                column.field
            ))
        } else if found.fields.len() > 1 {
            Some(format!(
                "{} is inside an object of the items: a column formula is for a field of the items themselves",
                column.field
            ))
        } else if !self.can_edit(&column.array) {
            Some(format!("{} cannot be edited", self.name_of(&column.array)))
        } else {
            None
        }
    }

    /// The formula bar's text for a selected column: its formula, or nothing.
    pub fn column_text(&self, column: &ColumnSel) -> String {
        self.calc()
            .column_formula(&column.sheet, &column.array, &column.field)
            .map(|text| format!("={text}"))
            .unwrap_or_default()
    }

    pub fn is_tabular(&self, array: &Path) -> bool {
        self.tables
            .tabular
            .contains(&(self.sheet.clone(), array.clone()))
    }

    /// The arrays of this tab that are shown as tables.
    pub fn tabular_paths(&self) -> HashSet<Path> {
        self.tables
            .tabular
            .iter()
            .filter(|(sheet, _)| sheet == &self.sheet)
            .map(|(_, path)| path.clone())
            .collect()
    }

    /// Whether the node is an array of objects, which can be a table.
    pub fn can_tabulate(&self, path: &Path) -> bool {
        self.calc()
            .spec(&self.sheet)
            .is_some_and(|spec| item_type(spec, path).is_some())
    }

    /// Whether the order of the items of the array can be changed: it must not
    /// be computed.
    pub fn can_reorder(&self, array: &Path) -> bool {
        self.can_edit(array) && self.calc().formula_text(&self.sheet, array).is_none()
    }

    pub fn set_tabular(&mut self, array: &Path, on: bool) {
        if on && !self.can_tabulate(array) {
            return;
        }
        let key = (self.sheet.clone(), array.clone());
        let changed = if on {
            self.tables.active.insert(self.sheet.clone(), array.clone());
            self.tables.tabular.insert(key)
        } else {
            if self.tables.active.get(&self.sheet) == Some(array) {
                self.tables.active.remove(&self.sheet);
            }
            self.tables.tabular.remove(&key)
        };
        if changed {
            // Nothing is open inside a table that has just been made.
            if let Some(set) = self.expanded.get_mut(&self.sheet) {
                set.retain(|p| {
                    !(p.len() > array.len() && p.segments().starts_with(array.segments()))
                });
            }
            self.tables.rev += 1;
            self.expansion_rev += 1;
        }
    }

    /// The table shown in the pane, if there is one on this tab.
    pub fn active_table(&self) -> Option<&Path> {
        self.tables
            .active
            .get(&self.sheet)
            .filter(|path| self.is_tabular(path))
    }

    /// The tabular array that holds `path` as an item or inside one, and the
    /// index of the item.
    pub fn table_row_of(&self, path: &Path) -> Option<(Path, usize)> {
        self.tables
            .tabular
            .iter()
            .filter(|(sheet, _)| sheet == &self.sheet)
            .find_map(|(_, array)| {
                let segments = path.segments();
                if segments.len() <= array.len() || !segments.starts_with(array.segments()) {
                    return None;
                }
                match segments[array.len()] {
                    Segment::Index(i) => Some((array.clone(), i)),
                    _ => None,
                }
            })
    }

    /// Called when a node is selected: a table the selection is in becomes the
    /// one shown.
    pub(crate) fn note_selection_in_table(&mut self, path: &Path) {
        if let Some((array, _)) = self.table_row_of(path) {
            self.tables.active.insert(self.sheet.clone(), array);
        }
    }

    fn items_of(&self, array: &Path) -> Option<&[Node]> {
        self.calc()
            .data(&self.sheet)?
            .get(array)?
            .value()
            .as_array()
    }

    pub(crate) fn columns_of(&self, array: &Path) -> Vec<Column> {
        let calc = self.calc();
        calc.spec(&self.sheet)
            .and_then(|spec| Some(table::columns(spec, item_type(spec, array)?)))
            .unwrap_or_default()
    }

    fn view_mut(&mut self, array: &Path) -> &mut TableView {
        self.tables.rev += 1;
        self.tables
            .views
            .entry((self.sheet.clone(), array.clone()))
            .or_default()
    }

    pub fn view_of(&self, array: &Path) -> Option<&TableView> {
        self.tables.views.get(&(self.sheet.clone(), array.clone()))
    }

    // ----------------------------------------------------------- layout

    /// Works out what the table pane shows, if the data or the view state
    /// changed since it was last worked out.
    pub fn refresh_table(&mut self) {
        let Some(array) = self.active_table().cloned() else {
            self.tables.layout = None;
            return;
        };
        let key = (
            self.sheet.clone(),
            array.clone(),
            self.calc().revision(),
            self.tables.rev,
        );
        if self.tables.layout.as_ref().is_some_and(|(k, _)| k == &key) {
            return;
        }
        let columns = self.columns_of(&array);
        let none = TableView::default();
        let view = self.view_of(&array).unwrap_or(&none);
        let shown = (0..columns.len())
            .filter(|&i| !view.hidden_columns.contains(&columns[i].name))
            .collect();
        let visible = match (self.calc().spec(&self.sheet), self.items_of(&array)) {
            (Some(spec), Some(items)) => table::visible(spec, items, &columns, view),
            _ => Visible::default(),
        };
        self.tables.layout = Some((
            key,
            Layout {
                array,
                columns,
                shown,
                visible,
            },
        ));
        self.keep_selection_in_view();
    }

    /// If the selected row was hidden, whether by hand or by a filter, the
    /// selection moves to the nearest row that is shown.
    fn keep_selection_in_view(&mut self) {
        if self.editing.is_some() {
            return;
        }
        let (Some(selection), Some((_, layout))) = (self.selection.clone(), &self.tables.layout)
        else {
            return;
        };
        let Some((array, index)) = self.table_row_of(&selection) else {
            return;
        };
        if array != layout.array || layout.visible.rows.binary_search(&index).is_ok() {
            return;
        }
        let rows = &layout.visible.rows;
        let at = rows.partition_point(|&r| r < index);
        let target = match rows.get(at).or(rows.last()) {
            Some(&row) => {
                let mut path = array.index(row);
                for segment in &selection.segments()[array.len() + 1..] {
                    path.push(segment.clone());
                }
                path
            }
            None => array,
        };
        self.selection = Some(target);
    }

    /// The layout, for drawing.
    pub fn layout(&self) -> Option<&Layout> {
        self.tables.layout.as_ref().map(|(_, layout)| layout)
    }

    // --------------------------------------------------- the selection

    /// Whether the selection is a cell of the table in the pane.
    pub fn table_cell_selected(&self) -> bool {
        match (&self.selection, self.layout()) {
            (Some(selection), Some(layout)) => self
                .table_row_of(selection)
                .is_some_and(|(array, _)| array == layout.array),
            _ => false,
        }
    }

    /// Moves the selection among the cells that are shown.
    pub fn table_move_selection(&mut self, dx: isize, dy: isize) {
        let (Some(selection), Some(layout)) = (self.selection.clone(), self.layout().cloned())
        else {
            return;
        };
        let Some((array, index)) = self.table_row_of(&selection) else {
            return;
        };
        let column = layout
            .shown
            .iter()
            .position(|&c| layout.columns[c].cell_path(&array, index) == selection)
            .unwrap_or(0);
        let Ok(row) = layout.visible.rows.binary_search(&index) else {
            return;
        };
        if layout.shown.is_empty() || layout.visible.rows.is_empty() {
            return;
        }
        let row = (row as isize + dy).clamp(0, layout.visible.rows.len() as isize - 1) as usize;
        let column = (column as isize + dx).clamp(0, layout.shown.len() as isize - 1) as usize;
        let path = layout.columns[layout.shown[column]].cell_path(&array, layout.visible.rows[row]);
        self.select(&path);
    }

    // ------------------------------------------------------------- rows

    /// A row added or edited stays shown until the filters change.
    pub(crate) fn pin_row_of(&mut self, path: &Path) {
        let Some((array, index)) = self.table_row_of(path) else {
            return;
        };
        let id = self
            .items_of(&array)
            .and_then(|items| items.get(index))
            .map(Node::id);
        if let Some(id) = id {
            self.view_mut(&array).pinned.insert(id);
        }
    }

    /// Adds a row at the end, or at `at`, and keeps it shown.
    pub fn add_row(&mut self, array: &Path, at: Option<usize>) {
        let length = self.items_of(array).map_or(0, <[Node]>::len);
        let at = at.unwrap_or(length).min(length);
        let history = self.calc().history_len().0;
        self.insert_item(array, Some(at));
        if self.calc().history_len().0 != history {
            self.pin_row_of(&array.index(at));
        }
    }

    pub fn delete_row(&mut self, array: &Path, index: usize) {
        self.delete_node(&array.index(index));
    }

    /// Reverses the whole array, hidden items included.
    pub fn reverse_rows(&mut self, array: &Path) {
        let length = self.items_of(array).map_or(0, <[Node]>::len);
        let order: Vec<usize> = (0..length).rev().collect();
        self.reorder(array, &order, "Reverse");
    }

    /// Swaps the item with the next one that is shown. Items hidden in between
    /// stay where they are.
    pub fn move_row(&mut self, array: &Path, index: usize, up: bool) {
        self.refresh_table();
        let Some(layout) = self.layout().filter(|l| &l.array == array) else {
            return;
        };
        let rows = &layout.visible.rows;
        let Ok(at) = rows.binary_search(&index) else {
            return;
        };
        let other = if up {
            at.checked_sub(1).map(|i| rows[i])
        } else {
            rows.get(at + 1).copied()
        };
        let Some(other) = other else {
            return;
        };
        let length = self.items_of(array).map_or(0, <[Node]>::len);
        let mut order: Vec<usize> = (0..length).collect();
        order.swap(index, other);
        self.reorder(array, &order, if up { "Move up" } else { "Move down" });
    }

    pub(crate) fn reorder(&mut self, array: &Path, order: &[usize], what: &str) {
        let sheet = self.sheet.clone();
        match self.session.calc_mut().reorder(&sheet, array, order, what) {
            Ok(()) => {
                self.notice = None;
                self.session.tick();
            }
            Err(e) => self.notice = Some(e.to_string()),
        }
    }

    /// Sorts the array by a column: ascending, then descending when the same
    /// column is clicked again. A real, undoable reorder of the whole array.
    pub fn sort_table(&mut self, array: &Path, column: &str, descending: Option<bool>) {
        let Some(column) = self
            .columns_of(array)
            .into_iter()
            .find(|c| c.name == column)
        else {
            return;
        };
        if !self.can_reorder(array) {
            self.notice = Some(format!(
                "{} is computed by a formula and cannot be sorted; use SORT in a formula",
                self.name_of(array)
            ));
            return;
        }
        let key = (self.sheet.clone(), array.clone(), column.name.clone());
        let descending = descending.unwrap_or_else(|| {
            matches!(&self.tables.last_sort,
                Some((s, a, c, desc)) if (s, a, c) == (&key.0, &key.1, &key.2) && !desc)
        });
        let Some(items) = self.items_of(array) else {
            return;
        };
        match sort_order(items, &column, descending) {
            Ok(order) => {
                self.tables.refused = None;
                self.reorder(array, &order, "Sort");
                self.tables.last_sort = Some((key.0, key.1, key.2, descending));
            }
            Err(refused) => {
                let keys = refused
                    .rows
                    .iter()
                    .map(|&i| {
                        value_key(
                            items
                                .get(i)
                                .and_then(|item| column.cell(item))
                                .map(Node::value),
                        )
                    })
                    .collect();
                self.notice = None;
                self.tables.refused = Some(Refusal {
                    array: array.clone(),
                    column: column.name.clone(),
                    message: refused.message,
                    rows: refused.rows,
                    keys,
                });
            }
        }
    }

    /// Filters the table to the rows a sort could not order.
    pub fn show_refused_rows(&mut self) {
        let Some(refusal) = self.tables.refused.take() else {
            return;
        };
        let filter = Filter {
            values: Some(refusal.keys),
            ..Filter::default()
        };
        self.set_filter(&refusal.array, &refusal.column, Some(filter));
    }

    // -------------------------------------------------- hiding, filters

    pub fn hide_column(&mut self, array: &Path, column: &str) {
        self.view_mut(array)
            .hidden_columns
            .insert(column.to_owned());
    }

    pub fn show_column(&mut self, array: &Path, column: &str) {
        self.view_mut(array).hidden_columns.remove(column);
    }

    pub fn hide_row(&mut self, array: &Path, index: usize) {
        let id = self
            .items_of(array)
            .and_then(|items| items.get(index))
            .map(Node::id);
        if let Some(id) = id {
            self.view_mut(array).hidden_rows.insert(id);
        }
    }

    pub fn show_row(&mut self, array: &Path, id: NodeId) {
        self.view_mut(array).hidden_rows.remove(&id);
    }

    /// Cancels the hiding done by hand. Rows hidden by a filter stay hidden.
    pub fn show_all_hidden(&mut self, array: &Path) {
        let view = self.view_mut(array);
        view.hidden_rows.clear();
        view.hidden_columns.clear();
    }

    /// The items hidden by hand, with their positions, for the list of hidden
    /// rows.
    pub fn hidden_rows(&self, array: &Path) -> Vec<(usize, NodeId)> {
        let (Some(view), Some(items)) = (self.view_of(array), self.items_of(array)) else {
            return Vec::new();
        };
        items
            .iter()
            .enumerate()
            .filter(|(_, item)| view.hidden_rows.contains(&item.id()))
            .map(|(i, item)| (i, item.id()))
            .collect()
    }

    /// Sets or clears the filter of a column. Changing a filter shows the rows
    /// that were kept only because they had been edited under the old one
    /// only if they pass.
    pub fn set_filter(&mut self, array: &Path, column: &str, filter: Option<Filter>) {
        let view = self.view_mut(array);
        view.pinned.clear();
        match filter {
            Some(filter) if filter.is_active() => {
                view.filters.insert(column.to_owned(), filter);
            }
            _ => {
                view.filters.remove(column);
            }
        }
    }

    pub fn clear_all_filters(&mut self, array: &Path) {
        let view = self.view_mut(array);
        view.pinned.clear();
        view.filters.clear();
    }

    /// Takes away rows that stayed shown only because they were edited.
    pub fn reapply_filters(&mut self, array: &Path) {
        self.view_mut(array).pinned.clear();
    }

    pub fn open_filter_dialog(&mut self, array: &Path, column: &str) {
        let columns = self.columns_of(array);
        let Some(found) = columns.iter().find(|c| c.name == column) else {
            return;
        };
        let distinct = self
            .items_of(array)
            .map(|items| distinct_values(items, found))
            .unwrap_or_default();
        let current = self
            .view_of(array)
            .and_then(|view| view.filters.get(column))
            .cloned()
            .unwrap_or_default();
        let (choice, first, second) = Choice::of(&current.condition);
        self.tables.dialog = Some(FilterDialog {
            array: array.clone(),
            column: column.to_owned(),
            checked: current
                .values
                .unwrap_or_else(|| distinct.iter().cloned().collect()),
            distinct,
            search: String::new(),
            choice,
            first,
            second,
            ignore_case: current.ignore_case,
            error: None,
        });
    }

    /// Applies the dialog. If an operand is not a value of the column's type
    /// the dialog stays open and says so.
    pub fn apply_filter_dialog(&mut self) {
        let Some(mut dialog) = self.tables.dialog.take() else {
            return;
        };
        let values = if dialog.checked.len() == dialog.distinct.len()
            && dialog.distinct.iter().all(|v| dialog.checked.contains(v))
        {
            None
        } else {
            Some(dialog.checked.clone())
        };
        let filter = Filter {
            values,
            condition: dialog.choice.condition(&dialog.first, &dialog.second),
            ignore_case: dialog.ignore_case,
        };
        let columns = self.columns_of(&dialog.array);
        let kind_error = columns
            .iter()
            .find(|c| c.name == dialog.column)
            .zip(self.calc().spec(&self.sheet))
            .and_then(|(column, spec)| filter.compile(&spec.get(column.ty).kind).err());
        if let Some(error) = kind_error {
            dialog.error = Some(error);
            self.tables.dialog = Some(dialog);
            return;
        }
        self.set_filter(&dialog.array, &dialog.column, Some(filter));
    }

    pub fn filter_of(&self, array: &Path, column: &str) -> Option<&Filter> {
        self.view_of(array)?.filters.get(column)
    }

    /// The value of a cell as the filter's list names it, for tests and for the
    /// table's tooltips.
    pub fn cell_key(&self, path: &Path) -> String {
        value_key(
            self.calc()
                .data(&self.sheet)
                .and_then(|d| d.get(path))
                .map(Node::value),
        )
    }
}
