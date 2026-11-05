//! The logic of the tabular view of an array of objects (DESIGN.md, "Tabular
//! view of arrays"): which columns a table has, how a column is sorted and
//! filtered, and which rows are shown. Nothing here draws.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashSet};

use gossia_calc::entry::parse_literal;
use gossia_core::{DateTimeValue, Kind, Node, NodeId, Path, TypeId, TypeSpec, Value};

use crate::rows::display;

/// What the list of values of a filter calls an absent or empty value.
pub const BLANK: &str = "(blank)";
/// ...and an error value.
pub const ERRORS: &str = "(errors)";

/// A column of the table: a scalar field of the item type, or a summary of a
/// field that holds an array or a map.
#[derive(Debug, Clone, PartialEq)]
pub struct Column {
    /// The field names joined by dots: `address.city`.
    pub name: String,
    pub fields: Vec<String>,
    pub ty: TypeId,
    /// Whether the field is required, and so are the objects it is inside.
    pub required: bool,
    /// An array or a map: shown as a count, and opened as a tree.
    pub summary: bool,
}

/// The item type of the array at `array`, if it is an array of objects.
pub fn item_type(spec: &TypeSpec, array: &Path) -> Option<TypeId> {
    let here = spec.resolve(array)?;
    match &spec.get(here.ty).kind {
        Kind::Array { items, .. } if matches!(spec.get(*items).kind, Kind::Object { .. }) => {
            Some(*items)
        }
        _ => None,
    }
}

/// The columns for items of type `item`, in the schema's declared order, with
/// fields that are objects flattened into dotted columns.
pub fn columns(spec: &TypeSpec, item: TypeId) -> Vec<Column> {
    let mut out = Vec::new();
    collect(spec, item, &mut Vec::new(), true, &mut out, &mut vec![item]);
    out
}

fn collect(
    spec: &TypeSpec,
    ty: TypeId,
    prefix: &mut Vec<String>,
    required: bool,
    out: &mut Vec<Column>,
    open: &mut Vec<TypeId>,
) {
    let Kind::Object { fields } = &spec.get(ty).kind else {
        return;
    };
    for field in fields {
        prefix.push(field.name.clone());
        let required = required && field.required;
        match &spec.get(field.ty).kind {
            Kind::Object { .. } => {
                // A type that contains itself would never end.
                if !open.contains(&field.ty) {
                    open.push(field.ty);
                    collect(spec, field.ty, prefix, required, out, open);
                    open.pop();
                }
            }
            kind => out.push(Column {
                name: prefix.join("."),
                fields: prefix.clone(),
                ty: field.ty,
                required,
                summary: matches!(kind, Kind::Array { .. } | Kind::Map { .. }),
            }),
        }
        prefix.pop();
    }
}

impl Column {
    /// The path of this column's cell in item `index` of `array`.
    pub fn cell_path(&self, array: &Path, index: usize) -> Path {
        self.fields
            .iter()
            .fold(array.index(index), |path, field| path.key(field.as_str()))
    }

    /// The node in a cell of `item`, if it has one.
    pub fn cell<'a>(&self, item: &'a Node) -> Option<&'a Node> {
        let mut node = item;
        for field in &self.fields {
            node = node.value().as_object()?.get(field)?;
        }
        Some(node)
    }
}

// ------------------------------------------------------------------ ordering

/// Values that Python can order against each other. Everything else (absent
/// values, errors, arrays) cannot be ordered, and neither can values of
/// different classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Class {
    Number,
    Text,
    Date,
    Naive,
    Zoned,
}

fn class(value: Option<&Value>) -> Option<Class> {
    match value? {
        Value::Bool(_) | Value::Int(_) | Value::Float(_) => Some(Class::Number),
        Value::Str(_) => Some(Class::Text),
        Value::Date(_) => Some(Class::Date),
        Value::DateTime(DateTimeValue::Naive(_)) => Some(Class::Naive),
        Value::DateTime(DateTimeValue::Zoned(_)) => Some(Class::Zoned),
        _ => None,
    }
}

/// The name Python gives the type of a value, for its error messages.
fn python_type(value: Option<&Value>) -> &'static str {
    match value {
        None | Some(Value::Null) => "NoneType",
        Some(Value::Bool(_)) => "bool",
        Some(Value::Int(_)) => "int",
        Some(Value::Float(_)) => "float",
        Some(Value::Str(_)) => "str",
        Some(Value::Date(_)) => "date",
        Some(Value::DateTime(_)) => "datetime",
        Some(Value::Array(_)) => "list",
        Some(Value::Object(_)) => "dict",
        Some(Value::Error(_)) => "ErrorValue",
    }
}

fn as_float(value: &Value) -> f64 {
    match value {
        Value::Bool(b) => f64::from(u8::from(*b)),
        Value::Int(n) => *n as f64,
        Value::Float(x) => *x,
        _ => 0.0,
    }
}

/// Python's ordering of two values of the same class.
fn compare(a: &Value, b: &Value) -> Ordering {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => x.cmp(y),
        (Value::Str(x), Value::Str(y)) => x.chars().cmp(y.chars()),
        (Value::Date(x), Value::Date(y)) => x.cmp(y),
        (Value::DateTime(DateTimeValue::Naive(x)), Value::DateTime(DateTimeValue::Naive(y))) => {
            x.cmp(y)
        }
        (Value::DateTime(DateTimeValue::Zoned(x)), Value::DateTime(DateTimeValue::Zoned(y))) => {
            x.cmp(y)
        }
        _ => as_float(a)
            .partial_cmp(&as_float(b))
            .unwrap_or(Ordering::Equal),
    }
}

/// Python's ordering of two values, or `None` where Python would raise.
pub fn python_cmp(a: Option<&Value>, b: Option<&Value>) -> Option<Ordering> {
    let (ca, cb) = (class(a)?, class(b)?);
    (ca == cb).then(|| compare(a.expect("classed"), b.expect("classed")))
}

/// Why a sort did not happen.
#[derive(Debug, Clone, PartialEq)]
pub struct SortRefused {
    /// Python's own message.
    pub message: String,
    /// The items that cannot be ordered with the others.
    pub rows: Vec<usize>,
}

/// The order of the items of an array when sorted by `column`: place `i` gets
/// the item at `order[i]`. Python's ordering, and stable, descending as well.
pub fn sort_order(
    items: &[Node],
    column: &Column,
    descending: bool,
) -> Result<Vec<usize>, SortRefused> {
    let values: Vec<Option<&Value>> = items
        .iter()
        .map(|item| column.cell(item).map(Node::value))
        .collect();
    if values.len() > 1 {
        // Sorting compares items with each other. The first value that can be
        // ordered sets the class the others must belong to.
        let reference = values.iter().position(|v| class(*v).is_some());
        let anchor = reference.unwrap_or(0);
        let wanted = class(values[anchor]);
        let offenders: Vec<usize> = (0..values.len())
            .filter(|&i| i != anchor && (class(values[i]).is_none() || class(values[i]) != wanted))
            .collect();
        let anchor_is_bad = wanted.is_none();
        if !offenders.is_empty() || anchor_is_bad {
            let mut rows = offenders;
            if anchor_is_bad {
                rows.insert(0, anchor);
            }
            let first = rows[0];
            let other = if first == anchor {
                (anchor + 1) % values.len()
            } else {
                anchor
            };
            let message = format!(
                "'<' not supported between instances of '{}' and '{}'",
                python_type(values[first]),
                python_type(values[other])
            );
            return Err(SortRefused { message, rows });
        }
    }
    let mut order: Vec<usize> = (0..values.len()).collect();
    order.sort_by(|&a, &b| {
        let ordering = compare(values[a].expect("checked"), values[b].expect("checked"));
        if descending {
            ordering.reverse()
        } else {
            ordering
        }
    });
    Ok(order)
}

// ------------------------------------------------------------------ filtering

/// The condition of a filter, chosen for the column's type. The operands are
/// the text the user typed, read as the column's type when the filter is used.
#[derive(Debug, Clone, PartialEq)]
pub enum Condition {
    Equals(String),
    NotEquals(String),
    Greater(String),
    Less(String),
    Between(String, String),
    BeginsWith(String),
    EndsWith(String),
    Contains(String),
    IsBlank,
}

/// The filter of one column: a list of the values to show, and a condition.
/// Both must hold.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Filter {
    /// `None` shows every value. The entries are the keys from [`value_key`].
    pub values: Option<BTreeSet<String>>,
    pub condition: Option<Condition>,
    /// For the text comparisons.
    pub ignore_case: bool,
}

impl Filter {
    pub fn is_active(&self) -> bool {
        self.values.is_some() || self.condition.is_some()
    }

    /// Reads the operands as the column's type. Fails with the reason if one
    /// is not a value of that type.
    pub fn compile(&self, kind: &Kind) -> Result<Compiled, String> {
        let operand = |text: &str| parse_literal(kind, text).map_err(|e| e.to_string());
        let condition = match &self.condition {
            None => None,
            Some(Condition::Equals(t)) => Some(Test::Equals(operand(t)?)),
            Some(Condition::NotEquals(t)) => Some(Test::NotEquals(operand(t)?)),
            Some(Condition::Greater(t)) => Some(Test::Greater(operand(t)?)),
            Some(Condition::Less(t)) => Some(Test::Less(operand(t)?)),
            Some(Condition::Between(a, b)) => Some(Test::Between(operand(a)?, operand(b)?)),
            Some(Condition::BeginsWith(t)) => Some(Test::Begins(t.clone())),
            Some(Condition::EndsWith(t)) => Some(Test::Ends(t.clone())),
            Some(Condition::Contains(t)) => Some(Test::Contains(t.clone())),
            Some(Condition::IsBlank) => Some(Test::Blank),
        };
        Ok(Compiled {
            values: self.values.clone(),
            condition,
            ignore_case: self.ignore_case,
        })
    }
}

#[derive(Debug, Clone)]
enum Test {
    Equals(Value),
    NotEquals(Value),
    Greater(Value),
    Less(Value),
    Between(Value, Value),
    Begins(String),
    Ends(String),
    Contains(String),
    Blank,
}

/// A filter with its operands read, ready to test many values.
#[derive(Debug, Clone)]
pub struct Compiled {
    values: Option<BTreeSet<String>>,
    condition: Option<Test>,
    ignore_case: bool,
}

/// An absent value, or empty text.
fn is_blank(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => true,
        Some(Value::Str(s)) => s.is_empty(),
        _ => false,
    }
}

/// How a value is listed in a filter's list of values.
pub fn value_key(value: Option<&Value>) -> String {
    match value {
        v if is_blank(v) => BLANK.to_owned(),
        Some(Value::Error(_)) => ERRORS.to_owned(),
        Some(v) => display(v),
        None => BLANK.to_owned(),
    }
}

impl Compiled {
    pub fn passes(&self, value: Option<&Value>) -> bool {
        if let Some(listed) = &self.values
            && !listed.contains(&value_key(value))
        {
            return false;
        }
        let Some(test) = &self.condition else {
            return true;
        };
        let fold = |s: &str| {
            if self.ignore_case {
                s.to_lowercase()
            } else {
                s.to_owned()
            }
        };
        let same = |a: Option<&Value>, b: &Value| match (a, b) {
            (Some(Value::Str(x)), Value::Str(y)) => fold(x) == fold(y),
            _ => python_cmp(a, Some(b)) == Some(Ordering::Equal),
        };
        let text = || match value {
            Some(Value::Str(s)) => s.clone(),
            other => value_key(other),
        };
        match test {
            Test::Blank => is_blank(value),
            Test::Equals(operand) => same(value, operand),
            Test::NotEquals(operand) => !same(value, operand),
            Test::Greater(operand) => python_cmp(value, Some(operand)) == Some(Ordering::Greater),
            Test::Less(operand) => python_cmp(value, Some(operand)) == Some(Ordering::Less),
            Test::Between(low, high) => {
                matches!(
                    python_cmp(value, Some(low)),
                    Some(Ordering::Greater | Ordering::Equal)
                ) && matches!(
                    python_cmp(value, Some(high)),
                    Some(Ordering::Less | Ordering::Equal)
                )
            }
            Test::Begins(t) => fold(&text()).starts_with(&fold(t)),
            Test::Ends(t) => fold(&text()).ends_with(&fold(t)),
            Test::Contains(t) => fold(&text()).contains(&fold(t)),
        }
    }
}

/// The distinct values of a column for the filter's list, in order: numbers,
/// text and dates by Python's ordering, then blank, then errors.
pub fn distinct_values(items: &[Node], column: &Column) -> Vec<String> {
    let mut seen: BTreeMap<String, Option<&Value>> = BTreeMap::new();
    for item in items {
        let value = column.cell(item).map(Node::value);
        seen.entry(value_key(value)).or_insert(value);
    }
    let mut keys: Vec<(String, Option<&Value>)> = seen.into_iter().collect();
    let rank = |key: &str| match key {
        BLANK => 2,
        ERRORS => 3,
        _ => 1,
    };
    keys.sort_by(|(ka, a), (kb, b)| {
        rank(ka)
            .cmp(&rank(kb))
            .then_with(|| class(*a).cmp(&class(*b)))
            .then_with(|| match (a, b) {
                (Some(a), Some(b)) if class(Some(a)) == class(Some(b)) => compare(a, b),
                _ => ka.cmp(kb),
            })
    });
    keys.into_iter().map(|(key, _)| key).collect()
}

// -------------------------------------------------------------------- view

/// The view state of one table. It is not part of the data, not undoable, and
/// not part of the trust hash.
#[derive(Debug, Clone, Default)]
pub struct TableView {
    /// Columns hidden by hand, by name.
    pub hidden_columns: BTreeSet<String>,
    /// Rows hidden by hand. The state belongs to the item, so a row stays
    /// hidden when a sort moves it.
    pub hidden_rows: HashSet<NodeId>,
    /// Filters, by column name.
    pub filters: BTreeMap<String, Filter>,
    /// Rows added or edited since the filters last changed. They stay shown
    /// even if they no longer pass, so that a row does not vanish while it is
    /// being typed into.
    pub pinned: HashSet<NodeId>,
}

/// Which rows of a table are shown.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Visible {
    /// The indexes of the shown items, in order.
    pub rows: Vec<usize>,
    pub total: usize,
    /// Hidden by hand.
    pub hidden: usize,
    /// Hidden by a filter (and not by hand).
    pub filtered: usize,
}

/// Decides which items are shown. A filter whose operands are not valid for
/// its column is ignored.
pub fn visible(spec: &TypeSpec, items: &[Node], columns: &[Column], view: &TableView) -> Visible {
    let filters: Vec<(&Column, Compiled)> = columns
        .iter()
        .filter_map(|column| {
            let filter = view.filters.get(&column.name)?;
            let compiled = filter.compile(&spec.get(column.ty).kind).ok()?;
            Some((column, compiled))
        })
        .collect();
    let mut out = Visible {
        total: items.len(),
        ..Visible::default()
    };
    for (index, item) in items.iter().enumerate() {
        if view.hidden_rows.contains(&item.id()) {
            out.hidden += 1;
            continue;
        }
        let passes = view.pinned.contains(&item.id())
            || filters
                .iter()
                .all(|(column, filter)| filter.passes(column.cell(item).map(Node::value)));
        if passes {
            out.rows.push(index);
        } else {
            out.filtered += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use gossia_core::{json, jsonschema};

    const SCHEMA: &str = r#"{
      "type": "object",
      "properties": {
        "items": {"type": "array", "items": {
          "type": "object",
          "properties": {
            "sku": {"type": "string"},
            "qty": {"type": "integer"},
            "price": {"type": "number"},
            "address": {"type": "object", "properties": {"city": {"type": "string"}}},
            "tags": {"type": "array", "items": {"type": "string"}},
            "when": {"type": "string", "format": "date"}
          },
          "required": ["sku"]
        }}
      }
    }"#;

    const DATA: &str = r#"{"items": [
      {"sku": "b", "qty": 3, "price": 2.5, "address": {"city": "Utrecht"}},
      {"sku": "a", "qty": 10, "price": 1, "address": {"city": "Delft"}, "tags": ["x"]},
      {"sku": "C", "qty": 3, "address": {"city": "Utrecht"}},
      {"sku": "", "qty": 2, "price": 9.75}
    ]}"#;

    struct Fixture {
        spec: TypeSpec,
        items: Vec<Node>,
        columns: Vec<Column>,
    }

    fn fixture() -> Fixture {
        let spec = jsonschema::lower_str(SCHEMA).unwrap();
        let data = json::parse(DATA).unwrap().node;
        let array: Path = "items".parse().unwrap();
        let items = data
            .get(&array)
            .unwrap()
            .value()
            .as_array()
            .unwrap()
            .to_vec();
        let columns = columns(&spec, item_type(&spec, &array).unwrap());
        Fixture {
            spec,
            items,
            columns,
        }
    }

    fn column<'a>(f: &'a Fixture, name: &str) -> &'a Column {
        f.columns.iter().find(|c| c.name == name).unwrap()
    }

    #[test]
    fn the_columns_are_the_scalar_fields_with_objects_flattened() {
        let f = fixture();
        let names: Vec<_> = f.columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            names,
            ["sku", "qty", "price", "address.city", "tags", "when"]
        );
        assert!(column(&f, "tags").summary && !column(&f, "qty").summary);
        assert!(column(&f, "sku").required && !column(&f, "address.city").required);
        let array: Path = "items".parse().unwrap();
        assert_eq!(
            column(&f, "address.city").cell_path(&array, 2).to_string(),
            "items[2].address.city"
        );
        let city = column(&f, "address.city");
        assert_eq!(
            city.cell(&f.items[0]).unwrap().value().as_str(),
            Some("Utrecht")
        );
        assert!(city.cell(&f.items[3]).is_none());
    }

    #[test]
    fn only_an_array_of_objects_can_be_a_table() {
        let spec = jsonschema::lower_str(SCHEMA).unwrap();
        assert!(item_type(&spec, &"items".parse().unwrap()).is_some());
        assert!(item_type(&spec, &"items[0].tags".parse().unwrap()).is_none());
        assert!(item_type(&spec, &Path::root()).is_none());
    }

    #[test]
    fn sorting_uses_pythons_order_and_is_stable_both_ways() {
        let f = fixture();
        // Code points: "" < "C" < "a" < "b".
        assert_eq!(
            sort_order(&f.items, column(&f, "sku"), false).unwrap(),
            [3, 2, 1, 0]
        );
        // Equal quantities keep their order: items 0 and 2 both have 3.
        assert_eq!(
            sort_order(&f.items, column(&f, "qty"), false).unwrap(),
            [3, 0, 2, 1]
        );
        assert_eq!(
            sort_order(&f.items, column(&f, "qty"), true).unwrap(),
            [1, 0, 2, 3],
            "descending is stable as well"
        );
    }

    #[test]
    fn a_column_with_absent_values_cannot_be_sorted_and_the_rows_are_named() {
        let f = fixture();
        let refused = sort_order(&f.items, column(&f, "price"), false).unwrap_err();
        assert_eq!(refused.rows, [2]);
        assert_eq!(
            refused.message,
            "'<' not supported between instances of 'NoneType' and 'float'"
        );
        let refused = sort_order(&f.items, column(&f, "address.city"), false).unwrap_err();
        assert_eq!(refused.rows, [3]);
        assert!(sort_order(&f.items[..1], column(&f, "price"), false).is_ok());
    }

    #[test]
    fn numbers_and_booleans_order_together_but_text_and_numbers_do_not() {
        let nodes = |values: &[Value]| -> Vec<Node> {
            values
                .iter()
                .map(|v| {
                    let mut node = json::parse(r#"{"v": null}"#).unwrap().node;
                    node.set(&"v".parse().unwrap(), v.clone()).unwrap();
                    node
                })
                .collect()
        };
        let column = Column {
            name: "v".into(),
            fields: vec!["v".into()],
            ty: fixture().columns[0].ty,
            required: true,
            summary: false,
        };
        let mixed = nodes(&[Value::Int(2), Value::Float(1.5), Value::Bool(true)]);
        assert_eq!(sort_order(&mixed, &column, false).unwrap(), [2, 1, 0]);
        let bad = nodes(&[Value::Int(2), Value::Str("a".into())]);
        let refused = sort_order(&bad, &column, false).unwrap_err();
        assert_eq!(refused.rows, [1]);
        assert_eq!(
            refused.message,
            "'<' not supported between instances of 'str' and 'int'"
        );
    }

    fn shown(f: &Fixture, view: &TableView) -> Vec<usize> {
        visible(&f.spec, &f.items, &f.columns, view).rows
    }

    fn view_with(column: &str, filter: Filter) -> TableView {
        let mut view = TableView::default();
        view.filters.insert(column.to_owned(), filter);
        view
    }

    #[test]
    fn a_filter_by_a_list_of_values_includes_blank() {
        let f = fixture();
        let values = |keys: &[&str]| Filter {
            values: Some(keys.iter().map(|k| (*k).to_owned()).collect()),
            ..Filter::default()
        };
        assert_eq!(
            shown(&f, &view_with("address.city", values(&["Utrecht"]))),
            [0, 2]
        );
        assert_eq!(shown(&f, &view_with("address.city", values(&[BLANK]))), [3]);
        assert_eq!(
            distinct_values(&f.items, column(&f, "address.city")),
            ["Delft", "Utrecht", BLANK]
        );
        assert_eq!(
            distinct_values(&f.items, column(&f, "qty")),
            ["2", "3", "10"],
            "numbers by value, not by text"
        );
    }

    #[test]
    fn conditions_compare_as_the_columns_type() {
        let f = fixture();
        let with = |column: &str, condition: Condition, ignore_case: bool| {
            shown(
                &f,
                &view_with(
                    column,
                    Filter {
                        values: None,
                        condition: Some(condition),
                        ignore_case,
                    },
                ),
            )
        };
        assert_eq!(
            with("qty", Condition::Greater("2".into()), false),
            [0, 1, 2]
        );
        assert_eq!(with("qty", Condition::Less("3".into()), false), [3]);
        assert_eq!(
            with("qty", Condition::Between("3".into(), "10".into()), false),
            [0, 1, 2]
        );
        assert_eq!(with("qty", Condition::NotEquals("3".into()), false), [1, 3]);
        assert_eq!(with("sku", Condition::Equals("c".into()), true), [2]);
        assert!(with("sku", Condition::Equals("c".into()), false).is_empty());
        assert_eq!(with("sku", Condition::BeginsWith("A".into()), true), [1]);
        assert_eq!(
            with("sku", Condition::Contains("".into()), false),
            [0, 1, 2, 3]
        );
        assert_eq!(with("sku", Condition::IsBlank, false), [3]);
        assert_eq!(with("price", Condition::IsBlank, false), [2]);
        assert_eq!(
            with("address.city", Condition::EndsWith("ft".into()), false),
            [1]
        );
    }

    #[test]
    fn filters_on_several_columns_must_all_pass_and_a_bad_operand_is_ignored() {
        let f = fixture();
        let mut view = view_with(
            "qty",
            Filter {
                condition: Some(Condition::Equals("3".into())),
                ..Filter::default()
            },
        );
        view.filters.insert(
            "address.city".into(),
            Filter {
                condition: Some(Condition::Equals("Delft".into())),
                ..Filter::default()
            },
        );
        assert!(shown(&f, &view).is_empty());
        view.filters.insert(
            "address.city".into(),
            Filter {
                condition: Some(Condition::Equals("Utrecht".into())),
                ..Filter::default()
            },
        );
        assert_eq!(shown(&f, &view), [0, 2]);
        view.filters.insert(
            "qty".into(),
            Filter {
                condition: Some(Condition::Equals("many".into())),
                ..Filter::default()
            },
        );
        assert!(
            view.filters["qty"]
                .compile(&Kind::Integer(Default::default()))
                .is_err(),
            "not a whole number"
        );
        assert_eq!(shown(&f, &view), [0, 2], "only the city filter applies");
    }

    #[test]
    fn hidden_and_filtered_rows_are_counted_apart_and_pinned_rows_stay() {
        let f = fixture();
        let mut view = view_with(
            "qty",
            Filter {
                condition: Some(Condition::Equals("3".into())),
                ..Filter::default()
            },
        );
        view.hidden_rows.insert(f.items[0].id());
        let v = visible(&f.spec, &f.items, &f.columns, &view);
        assert_eq!(
            (v.rows.clone(), v.hidden, v.filtered, v.total),
            (vec![2], 1, 2, 4)
        );
        view.pinned.insert(f.items[3].id());
        assert_eq!(shown(&f, &view), [2, 3], "an edited row is not taken away");
    }

    #[test]
    fn dates_are_compared_chronologically() {
        use chrono::NaiveDate;
        let day = |d: u32| Value::Date(NaiveDate::from_ymd_opt(2026, 1, d).unwrap());
        let items: Vec<Node> = [day(20), day(3), day(11)]
            .iter()
            .map(|v| {
                let mut node = json::parse(r#"{"when": null}"#).unwrap().node;
                node.set(&"when".parse().unwrap(), v.clone()).unwrap();
                node
            })
            .collect();
        let column = Column {
            name: "when".into(),
            fields: vec!["when".into()],
            ty: fixture().columns[0].ty,
            required: false,
            summary: false,
        };
        assert_eq!(sort_order(&items, &column, false).unwrap(), [1, 2, 0]);
        let after = Test::Greater(day(10));
        let compiled = Compiled {
            values: None,
            condition: Some(after),
            ignore_case: false,
        };
        assert!(compiled.passes(Some(&day(11))) && !compiled.passes(Some(&day(3))));
        assert!(!compiled.passes(None), "absent is not after anything");
    }
}
