//! The state of the view of the whole workbook, in `view.json` (the font of
//! the Data tree and Raw views), and of each tab, as the tab's overlay keeps it (DESIGN.md,
//! "File format", "Tabular view of arrays"): which view the tab shows, which
//! arrays are tables, for each table the hidden columns, the rows hidden by
//! hand and the filters, and the nodes of the tree hidden by hand. It is not part of the data, not undoable, and not part
//! of the trust hash.
//!
//! In the overlay it is the `view` object of the tab:
//!
//! ```json
//! "view": {
//!   "show": {"form": "Packing"},
//!   "tables": [
//!     {"array": "items", "hidden_columns": ["note"], "hidden_rows": [2],
//!      "filters": {"qty": {"values": ["1", "(blank)"],
//!                          "condition": {"is": "greater", "operands": ["1"]},
//!                          "ignore_case": false}}}
//!   ],
//!   "table": "items",
//!   "hidden": ["customer.city", "items[3]"]
//! }
//! ```
//!
//! `show` is `"raw"` or a form, and is left out for the data tree. Rows are
//! saved by position and become the items at those positions again when the
//! file is read. What a newer version put in the object is kept. State that no
//! longer fits the tab (a form or an array that is gone) is dropped quietly,
//! since it only changes what is drawn.

use std::collections::{BTreeMap, BTreeSet};

use gossia_core::form::FONT_SIZES;
use gossia_core::{Path, Value};
use serde_json::{Map, Value as Json, json};

use crate::app::GossiaApp;
use crate::form::View;
use crate::style::{DataFont, Typeface};

/// The key of the font of the data views in `view.json`.
const DATA_FONT: &str = "data_font";
use crate::table::{Condition, Filter, TableView, item_type};

/// The keys of the view object this version writes. Anything else in it was
/// written by a newer version and is kept.
const KNOWN: [&str; 4] = ["show", "tables", "table", "hidden"];

fn condition_to_json(condition: &Condition) -> Json {
    let (is, operands): (&str, Vec<&String>) = match condition {
        Condition::Equals(a) => ("equals", vec![a]),
        Condition::NotEquals(a) => ("not_equals", vec![a]),
        Condition::Greater(a) => ("greater", vec![a]),
        Condition::Less(a) => ("less", vec![a]),
        Condition::Between(a, b) => ("between", vec![a, b]),
        Condition::BeginsWith(a) => ("begins_with", vec![a]),
        Condition::EndsWith(a) => ("ends_with", vec![a]),
        Condition::Contains(a) => ("contains", vec![a]),
        Condition::IsBlank => ("is_blank", vec![]),
    };
    json!({ "is": is, "operands": operands })
}

fn condition_from_json(value: &Json) -> Option<Condition> {
    let operands: Vec<String> = value
        .get("operands")
        .and_then(Json::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Json::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let first = || operands.first().cloned();
    Some(match value.get("is")?.as_str()? {
        "equals" => Condition::Equals(first()?),
        "not_equals" => Condition::NotEquals(first()?),
        "greater" => Condition::Greater(first()?),
        "less" => Condition::Less(first()?),
        "between" => Condition::Between(first()?, operands.get(1)?.clone()),
        "begins_with" => Condition::BeginsWith(first()?),
        "ends_with" => Condition::EndsWith(first()?),
        "contains" => Condition::Contains(first()?),
        "is_blank" => Condition::IsBlank,
        _ => return None,
    })
}

fn filter_to_json(filter: &Filter) -> Json {
    let mut out = Map::new();
    if let Some(values) = &filter.values {
        out.insert("values".to_owned(), json!(values));
    }
    if let Some(condition) = &filter.condition {
        out.insert("condition".to_owned(), condition_to_json(condition));
    }
    out.insert("ignore_case".to_owned(), json!(filter.ignore_case));
    Json::Object(out)
}

fn filter_from_json(value: &Json) -> Option<Filter> {
    let values = value.get("values").and_then(Json::as_array).map(|items| {
        items
            .iter()
            .filter_map(Json::as_str)
            .map(str::to_owned)
            .collect::<BTreeSet<_>>()
    });
    let condition = value.get("condition").and_then(condition_from_json);
    let filter = Filter {
        values,
        condition,
        ignore_case: value
            .get("ignore_case")
            .and_then(Json::as_bool)
            .unwrap_or(false),
    };
    filter.is_active().then_some(filter)
}

fn strings(value: Option<&Json>) -> Vec<String> {
    value
        .and_then(Json::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Json::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

impl GossiaApp {
    /// The view state of every tab that has any, merged over what the file
    /// had, for the overlays of a save.
    pub fn view_state(&self) -> BTreeMap<String, Json> {
        let mut out = BTreeMap::new();
        for tab in self.calc().sheet_names() {
            let mut object = self
                .doc
                .views
                .get(tab)
                .and_then(Json::as_object)
                .cloned()
                .unwrap_or_default();
            for key in KNOWN {
                object.remove(key);
            }
            match self.views.get(tab) {
                Some(View::Raw) => {
                    object.insert("show".to_owned(), json!("raw"));
                }
                Some(View::Form(name)) => {
                    object.insert("show".to_owned(), json!({ "form": name }));
                }
                Some(View::Tree) | None => {}
            }
            let tables = self.table_state(tab);
            if !tables.is_empty() {
                object.insert("tables".to_owned(), Json::Array(tables));
                if let Some(active) = self.tables.active.get(tab)
                    && self
                        .tables
                        .tabular
                        .contains(&(tab.to_owned(), active.clone()))
                {
                    object.insert("table".to_owned(), json!(active.to_string()));
                }
            }
            let hidden = self.hidden_paths(tab);
            if !hidden.is_empty() {
                object.insert("hidden".to_owned(), json!(hidden));
            }
            if !object.is_empty() {
                out.insert(tab.to_owned(), Json::Object(object));
            }
        }
        out
    }

    /// The tables of a tab, in path order, so that the same state is the same
    /// bytes.
    fn table_state(&self, tab: &str) -> Vec<Json> {
        let mut arrays: Vec<&Path> = self
            .tables
            .tabular
            .iter()
            .filter(|(sheet, _)| sheet == tab)
            .map(|(_, path)| path)
            .collect();
        arrays.sort_by_key(|p| p.to_string());
        arrays
            .into_iter()
            .map(|array| {
                let mut entry = Map::new();
                entry.insert("array".to_owned(), json!(array.to_string()));
                if let Some(view) = self.tables.views.get(&(tab.to_owned(), array.clone())) {
                    if !view.hidden_columns.is_empty() {
                        entry.insert("hidden_columns".to_owned(), json!(view.hidden_columns));
                    }
                    let items = self
                        .calc()
                        .data(tab)
                        .and_then(|d| d.get(array))
                        .and_then(|n| n.value().as_array());
                    let rows: Vec<usize> = items
                        .map(|items| {
                            items
                                .iter()
                                .enumerate()
                                .filter(|(_, item)| view.hidden_rows.contains(&item.id()))
                                .map(|(i, _)| i)
                                .collect()
                        })
                        .unwrap_or_default();
                    if !rows.is_empty() {
                        entry.insert("hidden_rows".to_owned(), json!(rows));
                    }
                    if !view.filters.is_empty() {
                        let filters: Map<String, Json> = view
                            .filters
                            .iter()
                            .map(|(column, f)| (column.clone(), filter_to_json(f)))
                            .collect();
                        entry.insert("filters".to_owned(), Json::Object(filters));
                    }
                }
                Json::Object(entry)
            })
            .collect()
    }

    /// Puts back the view state read from a file. The raw objects are kept in
    /// the document, for what this version does not know.
    pub fn restore_view_state(&mut self, views: &BTreeMap<String, Json>) {
        for (tab, state) in views {
            if self.calc().data(tab).is_none() {
                continue;
            }
            match state.get("show") {
                Some(Json::String(s)) if s == "raw" => {
                    self.views.insert(tab.clone(), View::Raw);
                }
                Some(show) => {
                    if let Some(name) = show.get("form").and_then(Json::as_str)
                        && self.calc().forms().all_names(tab).iter().any(|n| n == name)
                    {
                        self.views.insert(tab.clone(), View::Form(name.to_owned()));
                    }
                }
                None => {}
            }
            for table in state
                .get("tables")
                .and_then(Json::as_array)
                .into_iter()
                .flatten()
            {
                self.restore_table(tab, table);
            }
            if let Some(active) = state
                .get("table")
                .and_then(Json::as_str)
                .and_then(|t| t.parse::<Path>().ok())
                && self.tables.tabular.contains(&(tab.clone(), active.clone()))
            {
                self.tables.active.insert(tab.clone(), active);
            }
            self.restore_hidden(tab, &strings(state.get("hidden")));
        }
        self.tables.rev += 1;
        self.expansion_rev += 1;
        self.rows_key = None;
    }

    /// The state of the view of the whole workbook, for `view.json`, merged
    /// over what the file had: the font of the data views, left out when it is
    /// the default.
    ///
    /// ```json
    /// {"data_font": {"family": "serif", "size": 16}}
    /// ```
    pub fn workbook_view(&self) -> Option<Json> {
        let mut object = self
            .doc
            .workbook_view
            .as_ref()
            .and_then(Json::as_object)
            .cloned()
            .unwrap_or_default();
        object.remove(DATA_FONT);
        if self.data_font != DataFont::default() {
            object.insert(
                DATA_FONT.to_owned(),
                json!({"family": self.data_font.family.key(), "size": self.data_font.size}),
            );
        }
        (!object.is_empty()).then_some(Json::Object(object))
    }

    /// Puts back the state of the view of the whole workbook read from a file.
    /// A value this version cannot use is the default.
    pub fn restore_workbook_view(&mut self, view: Option<&Json>) {
        let font = view.and_then(|v| v.get(DATA_FONT));
        let defaults = DataFont::default();
        self.data_font = DataFont {
            family: font
                .and_then(|f| f.get("family"))
                .and_then(Json::as_str)
                .and_then(Typeface::from_key)
                .unwrap_or(defaults.family),
            size: font
                .and_then(|f| f.get("size"))
                .and_then(Json::as_u64)
                .and_then(|s| u32::try_from(s).ok())
                .filter(|s| FONT_SIZES.contains(s))
                .unwrap_or(defaults.size),
        };
    }

    /// Changes the font of the data views. Like the rest of the view state it
    /// is not an edit: it is kept with the next save.
    pub fn set_data_font(&mut self, font: DataFont) {
        self.data_font = font;
    }

    fn restore_table(&mut self, tab: &str, table: &Json) {
        let Some(array) = table
            .get("array")
            .and_then(Json::as_str)
            .and_then(|t| t.parse::<Path>().ok())
        else {
            return;
        };
        let calc = self.session.calc();
        if !calc
            .spec(tab)
            .is_some_and(|spec| item_type(spec, &array).is_some())
        {
            return;
        }
        let ids: Vec<_> = match calc
            .data(tab)
            .and_then(|d| d.get(&array))
            .map(|n| n.value())
        {
            Some(Value::Array(items)) => items.iter().map(|item| item.id()).collect(),
            _ => Vec::new(),
        };
        let mut view = TableView {
            hidden_columns: strings(table.get("hidden_columns")).into_iter().collect(),
            ..TableView::default()
        };
        for index in table
            .get("hidden_rows")
            .and_then(Json::as_array)
            .into_iter()
            .flatten()
            .filter_map(Json::as_u64)
        {
            if let Some(id) = usize::try_from(index).ok().and_then(|i| ids.get(i)) {
                view.hidden_rows.insert(*id);
            }
        }
        if let Some(filters) = table.get("filters").and_then(Json::as_object) {
            for (column, value) in filters {
                if let Some(filter) = filter_from_json(value) {
                    view.filters.insert(column.clone(), filter);
                }
            }
        }
        let key = (tab.to_owned(), array);
        self.tables
            .active
            .entry(tab.to_owned())
            .or_insert(key.1.clone());
        self.tables.tabular.insert(key.clone());
        self.tables.views.insert(key, view);
    }
}
