//! The rows of the tree grid: the data tree laid out for drawing, following the
//! schema (DESIGN.md, "Application structure", "UI" and "Cells exist only where
//! the schema says").
//!
//! Every node the schema defines has a row, whether or not it has data: an
//! absent optional field is an empty cell that can be filled. Content the
//! schema does not define is shown greyed and read-only after the declared
//! fields. Only the rows of expanded nodes are made, and none for a node
//! hidden by hand or for what is inside it.

use std::collections::{BTreeSet, HashSet};

use cliotree_core::{Kind, Node, Path, Segment, TypeId, TypeSpec, Value};

use crate::tree_hidden::{HiddenKey, Step};

/// What a row stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    Object,
    Array,
    Map,
    Scalar,
}

#[derive(Debug, Clone)]
pub struct Row {
    pub path: Path,
    pub depth: usize,
    /// A field name, a key, `[2]` for an item, or the tab name for the root.
    pub name: String,
    /// `None` for content the schema does not define.
    pub ty: Option<TypeId>,
    pub type_label: String,
    /// The node, or `None` if it is absent.
    pub node: Option<Node>,
    pub kind: RowKind,
    pub required: bool,
    pub expanded: bool,
    /// Content in the data that the schema does not define.
    pub unrecognised: bool,
    /// An array shown as a table: its items are in the table, and only an
    /// item that was opened from the table (a drill-in) has rows here.
    pub tabular: bool,
    /// How many of its children are hidden by hand.
    pub hidden: usize,
    /// Hidden nodes come right after this row: their parent and how many.
    pub marker: Option<(Path, usize)>,
}

impl Row {
    /// Whether the row has rows of its own below it.
    pub fn expandable(&self) -> bool {
        self.kind != RowKind::Scalar && !self.unrecognised && !self.tabular
    }
}

/// The rows of a tab. `expanded` holds the paths of the nodes that are open,
/// `tabular` those of the arrays that are shown as tables, and `hidden` the
/// nodes hidden by hand.
pub fn build(
    spec: &TypeSpec,
    tab: &str,
    root: &Node,
    expanded: &HashSet<Path>,
    tabular: &HashSet<Path>,
    hidden: &HashSet<HiddenKey>,
) -> Vec<Row> {
    let mut out = Vec::new();
    let open = Open {
        expanded,
        tabular,
        hidden,
    };
    walk(
        spec,
        spec.root(),
        Path::root(),
        &mut Vec::new(),
        tab.to_owned(),
        Some(root),
        true,
        0,
        &open,
        &mut out,
    );
    out
}

struct Open<'a> {
    expanded: &'a HashSet<Path>,
    tabular: &'a HashSet<Path>,
    hidden: &'a HashSet<HiddenKey>,
}

/// Counts the hidden children of a row as they are passed over, and marks
/// the row they come right after.
struct Skipped {
    parent: usize,
    count: usize,
}

impl Skipped {
    /// Whether the child at `key` is hidden; if so it is counted.
    fn hidden(&mut self, open: &Open<'_>, key: &HiddenKey, out: &mut [Row]) -> bool {
        if open.hidden.is_empty() || !open.hidden.contains(key) {
            return false;
        }
        self.count += 1;
        let parent = out[self.parent].path.clone();
        if let Some(last) = out.last_mut() {
            match &mut last.marker {
                Some((p, n)) if *p == parent => *n += 1,
                marker => *marker = Some((parent, 1)),
            }
        }
        true
    }
}

impl Open<'_> {
    /// The items of the table at `array` that have something open inside them.
    fn drilled(&self, array: &Path) -> BTreeSet<usize> {
        self.expanded
            .iter()
            .filter(|p| p.len() > array.len() && p.segments().starts_with(array.segments()))
            .filter_map(|p| match p.segments()[array.len()] {
                Segment::Index(i) => Some(i),
                _ => None,
            })
            .collect()
    }
}

#[allow(clippy::too_many_arguments)]
fn walk(
    spec: &TypeSpec,
    ty: TypeId,
    path: Path,
    key: &mut HiddenKey,
    name: String,
    node: Option<&Node>,
    required: bool,
    depth: usize,
    open_state: &Open<'_>,
    out: &mut Vec<Row>,
) {
    let expanded = open_state.expanded;
    let kind = match &spec.get(ty).kind {
        Kind::Object { .. } => RowKind::Object,
        Kind::Array { .. } => RowKind::Array,
        Kind::Map { .. } => RowKind::Map,
        _ => RowKind::Scalar,
    };
    let tabular = kind == RowKind::Array && open_state.tabular.contains(&path);
    let drilled = if tabular {
        open_state.drilled(&path)
    } else {
        BTreeSet::new()
    };
    let open =
        kind != RowKind::Scalar && expanded.contains(&path) && (!tabular || !drilled.is_empty());
    let me = out.len();
    out.push(Row {
        path: path.clone(),
        depth,
        name,
        ty: Some(ty),
        type_label: spec.describe(ty),
        node: node.cloned(),
        kind,
        required,
        expanded: open,
        unrecognised: false,
        tabular,
        hidden: 0,
        marker: None,
    });
    if !open {
        return;
    }
    let mut skipped = Skipped {
        parent: me,
        count: 0,
    };
    match &spec.get(ty).kind {
        Kind::Object { fields } => {
            for field in fields {
                key.push(Step::Key(field.name.clone()));
                if skipped.hidden(open_state, key, out) {
                    key.pop();
                    continue;
                }
                let child = node.and_then(|n| n.child(&Segment::Key(field.name.clone())));
                walk(
                    spec,
                    field.ty,
                    path.key(field.name.as_str()),
                    key,
                    field.name.clone(),
                    child,
                    field.required,
                    depth + 1,
                    open_state,
                    out,
                );
                key.pop();
            }
            if let Some(Value::Object(present)) = node.map(Node::value) {
                for (name, child) in present {
                    if fields.iter().any(|f| &f.name == name) {
                        continue;
                    }
                    key.push(Step::Key(name.clone()));
                    let hidden = skipped.hidden(open_state, key, out);
                    key.pop();
                    if hidden {
                        continue;
                    }
                    out.push(Row {
                        path: path.key(name.as_str()),
                        depth: depth + 1,
                        name: name.clone(),
                        ty: None,
                        type_label: "unrecognised".to_owned(),
                        node: Some(child.clone()),
                        kind: RowKind::Scalar,
                        required: false,
                        expanded: false,
                        unrecognised: true,
                        tabular: false,
                        hidden: 0,
                        marker: None,
                    });
                }
            }
        }
        Kind::Array { items, .. } => {
            if let Some(Value::Array(present)) = node.map(Node::value) {
                for (i, child) in present.iter().enumerate() {
                    if tabular && !drilled.contains(&i) {
                        continue;
                    }
                    key.push(Step::Item(child.id()));
                    if skipped.hidden(open_state, key, out) {
                        key.pop();
                        continue;
                    }
                    walk(
                        spec,
                        *items,
                        path.index(i),
                        key,
                        format!("[{i}]"),
                        Some(child),
                        true,
                        depth + 1,
                        open_state,
                        out,
                    );
                    key.pop();
                }
            }
        }
        Kind::Map { value } => {
            if let Some(Value::Object(present)) = node.map(Node::value) {
                for (name, child) in present {
                    key.push(Step::Key(name.clone()));
                    if skipped.hidden(open_state, key, out) {
                        key.pop();
                        continue;
                    }
                    walk(
                        spec,
                        *value,
                        path.key(name.as_str()),
                        key,
                        name.clone(),
                        Some(child),
                        true,
                        depth + 1,
                        open_state,
                        out,
                    );
                    key.pop();
                }
            }
        }
        _ => {}
    }
    out[me].hidden = skipped.count;
}

/// How a value is shown in its cell: in Python's canonical form, with dates in
/// ISO 8601 (DESIGN.md, "The Options dialog", "Display of values").
pub fn display(value: &Value) -> String {
    match value {
        Value::Null => "None".to_owned(),
        Value::Bool(true) => "True".to_owned(),
        Value::Bool(false) => "False".to_owned(),
        Value::Int(n) => n.to_string(),
        Value::Float(x) => float_text(*x),
        Value::Str(s) => s.clone(),
        Value::Date(d) => d.format("%Y-%m-%d").to_string(),
        Value::DateTime(cliotree_core::DateTimeValue::Naive(n)) => {
            n.format("%Y-%m-%dT%H:%M:%S%.f").to_string()
        }
        Value::DateTime(cliotree_core::DateTimeValue::Zoned(z)) => z.to_rfc3339(),
        Value::Array(items) => match items.len() {
            1 => "[1 item]".to_owned(),
            n => format!("[{n} items]"),
        },
        Value::Object(fields) => match fields.len() {
            1 => "{1 field}".to_owned(),
            n => format!("{{{n} fields}}"),
        },
        Value::Error(e) => e.kind.code().to_owned(),
    }
}

/// A float as Python writes it: `2.0`, `0.21`, `1e-07`.
fn float_text(x: f64) -> String {
    if x.is_finite() && x == x.trunc() && x.abs() < 1e16 {
        format!("{x:.1}")
    } else {
        format!("{x}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cliotree_core::{json, jsonschema};

    const SCHEMA: &str = r#"{
      "type": "object",
      "properties": {
        "name": {"type": "string"},
        "note": {"type": "string"},
        "tags": {"type": "array", "items": {"type": "string"}},
        "by": {"type": "object", "additionalProperties": {"type": "integer"}},
        "address": {"type": "object", "properties": {"city": {"type": "string"}}, "required": ["city"]}
      },
      "required": ["name", "tags", "by", "address"]
    }"#;

    const DATA: &str = r#"{"name": "x", "tags": ["a", "b"], "by": {"k": 1, "j": 2},
        "address": {"city": "Utrecht"}, "extra": 5}"#;

    fn rows(open: &[&str]) -> Vec<Row> {
        let spec = jsonschema::lower_str(SCHEMA).unwrap();
        let root = json::parse(DATA).unwrap().node;
        let expanded: HashSet<Path> = open
            .iter()
            .map(|p| {
                if p.is_empty() {
                    Path::root()
                } else {
                    p.parse().unwrap()
                }
            })
            .collect();
        build(
            &spec,
            "T",
            &root,
            &expanded,
            &HashSet::new(),
            &HashSet::new(),
        )
    }

    fn names(rows: &[Row]) -> Vec<String> {
        rows.iter()
            .map(|r| format!("{}{}", "  ".repeat(r.depth), r.name))
            .collect()
    }

    #[test]
    fn a_closed_tree_is_one_row() {
        let r = rows(&[]);
        assert_eq!(names(&r), ["T"]);
        assert_eq!(r[0].type_label, "object");
        assert!(r[0].expandable() && !r[0].expanded);
    }

    /// Every node the schema defines has a row, in declared order, whether or
    /// not it has data; content the schema does not define comes after, marked.
    #[test]
    fn the_schema_decides_the_rows_and_unknown_content_is_marked() {
        let r = rows(&[""]);
        assert_eq!(
            names(&r),
            [
                "T",
                "  name",
                "  note",
                "  tags",
                "  by",
                "  address",
                "  extra"
            ]
        );
        let note = &r[2];
        assert!(
            note.node.is_none() && !note.required,
            "absent optional field"
        );
        let extra = &r[6];
        assert!(extra.unrecognised && extra.ty.is_none() && !extra.expandable());
        assert!(r[1].required);
    }

    #[test]
    fn arrays_maps_and_objects_open_to_their_children() {
        let r = rows(&["", "tags", "by", "address"]);
        assert_eq!(
            names(&r),
            [
                "T",
                "  name",
                "  note",
                "  tags",
                "    [0]",
                "    [1]",
                "  by",
                "    k",
                "    j",
                "  address",
                "    city",
                "  extra"
            ]
        );
        assert_eq!(r[4].path.to_string(), "tags[0]");
        assert_eq!(r[10].path.to_string(), "address.city");
        assert_eq!(r[3].type_label, "array of string");
        assert_eq!(r[6].type_label, "map of integer");
    }

    #[test]
    fn an_absent_object_can_be_opened_and_shows_empty_fields() {
        let spec = jsonschema::lower_str(
            r#"{"type":"object","properties":{"o":{"type":"object","properties":{"n":{"type":"integer"}}}}}"#,
        )
        .unwrap();
        let root = json::parse("{}").unwrap().node;
        let open: HashSet<Path> = [Path::root(), "o".parse().unwrap()].into();
        let r = build(&spec, "T", &root, &open, &HashSet::new(), &HashSet::new());
        assert_eq!(names(&r), ["T", "  o", "    n"]);
        assert!(r[1].node.is_none() && r[2].node.is_none());
    }

    #[test]
    fn only_expanded_rows_are_built() {
        let spec = jsonschema::lower_str(
            r#"{"type":"object","properties":{"big":{"type":"array","items":{"type":"integer"}}}}"#,
        )
        .unwrap();
        let big: Vec<String> = (0..50_000).map(|i| i.to_string()).collect();
        let root = json::parse(&format!(r#"{{"big": [{}]}}"#, big.join(",")))
            .unwrap()
            .node;
        let closed: HashSet<Path> = [Path::root()].into();
        assert_eq!(
            build(&spec, "T", &root, &closed, &HashSet::new(), &HashSet::new()).len(),
            2
        );
        let open: HashSet<Path> = [Path::root(), "big".parse().unwrap()].into();
        assert_eq!(
            build(&spec, "T", &root, &open, &HashSet::new(), &HashSet::new()).len(),
            50_002
        );
    }

    /// An array shown as a table has no rows for its items, except the items
    /// that something was opened in.
    #[test]
    fn a_tabular_array_shows_only_the_items_opened_from_its_table() {
        let spec = jsonschema::lower_str(
            r#"{"type":"object","properties":{"o":{"type":"array","items":{"type":"object","properties":{"n":{"type":"integer"}}}}}}"#,
        )
        .unwrap();
        let root = json::parse(r#"{"o": [{"n": 1}, {"n": 2}, {"n": 3}]}"#)
            .unwrap()
            .node;
        let tabular: HashSet<Path> = ["o".parse().unwrap()].into();
        let mut open: HashSet<Path> = [Path::root(), "o".parse().unwrap()].into();
        let r = build(&spec, "T", &root, &open, &tabular, &HashSet::new());
        assert_eq!(names(&r), ["T", "  o"]);
        assert!(r[1].tabular && !r[1].expandable() && !r[1].expanded);
        open.insert("o[1]".parse().unwrap());
        let r = build(&spec, "T", &root, &open, &tabular, &HashSet::new());
        assert_eq!(names(&r), ["T", "  o", "    [1]", "      n"]);
        assert!(r[1].expanded);
    }

    /// A node hidden by hand has no row, nor has anything inside it; its
    /// parent counts it and the row before it carries the marker.
    #[test]
    fn hidden_nodes_have_no_rows_and_are_counted() {
        use crate::tree_hidden::key_of;
        let spec = jsonschema::lower_str(SCHEMA).unwrap();
        let root = json::parse(DATA).unwrap().node;
        let open: HashSet<Path> = ["", "tags", "by", "address"]
            .iter()
            .map(|p| {
                if p.is_empty() {
                    Path::root()
                } else {
                    p.parse().unwrap()
                }
            })
            .collect();
        let hide = |paths: &[&str]| -> HashSet<HiddenKey> {
            paths
                .iter()
                .map(|p| key_of(&root, &p.parse().unwrap()).unwrap())
                .collect()
        };
        // An absent field, an item, a key of a map, and an object with its
        // fields.
        let hidden = hide(&["note", "tags[0]", "by.j", "address"]);
        let r = build(&spec, "T", &root, &open, &HashSet::new(), &hidden);
        assert_eq!(
            names(&r),
            [
                "T", "  name", "  tags", "    [1]", "  by", "    k", "  extra"
            ]
        );
        assert_eq!(r[0].hidden, 2);
        assert_eq!(r[2].hidden, 1);
        assert_eq!(r[4].hidden, 1);
        // `note` comes after `name`; `tags[0]` is first, after its parent;
        // `by.j` and then `address` both come after `k`.
        assert_eq!(r[1].marker, Some((Path::root(), 1)));
        assert_eq!(r[2].marker, Some(("tags".parse().unwrap(), 1)));
        assert_eq!(r[5].marker, Some((Path::root(), 1)));
        assert_eq!(r[3].marker, None);
    }

    /// Values are shown in Python's canonical form.
    #[test]
    fn values_are_displayed_as_python_writes_them() {
        use chrono::NaiveDate;
        assert_eq!(display(&Value::Int(-3)), "-3");
        assert_eq!(display(&Value::Float(2.0)), "2.0");
        assert_eq!(display(&Value::Float(0.21)), "0.21");
        assert_eq!(display(&Value::Float(1e20)), "100000000000000000000");
        assert_eq!(display(&Value::Bool(true)), "True");
        assert_eq!(display(&Value::Str("hi".into())), "hi");
        assert_eq!(
            display(&Value::Date(NaiveDate::from_ymd_opt(2026, 10, 23).unwrap())),
            "2026-10-23"
        );
        let two = json::parse("[1, 2]").unwrap().node;
        assert_eq!(display(two.value()), "[2 items]");
        let one = json::parse(r#"{"a": 1}"#).unwrap().node;
        assert_eq!(display(one.value()), "{1 field}");
    }
}
