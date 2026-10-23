//! Name completion in the formula bar (DESIGN.md, "Formulas", "Name
//! completion").
//!
//! While the author types, a popup offers the names a formula can use: tab
//! names, `this`, `parent` and `root`, the functions and modules in the
//! catalogue. After a tab name and a dot (`Orders.`) it offers the fields of
//! that node, taken from the schema, and goes on doing so through the path:
//! `Orders.items[0].`. Completion for arbitrary Python objects is not
//! attempted.

use cliotree_core::{Kind, Path, Segment, TypeId, TypeSpec};

use super::catalog::{Group, catalog};
use crate::Calc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionKind {
    Tab,
    Relative,
    Field,
    Function,
    Module,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    /// What the popup shows.
    pub label: String,
    /// What replaces the word being typed.
    pub insert: String,
    pub kind: CompletionKind,
    /// The type of a field, or the signature of a function.
    pub detail: String,
}

/// The completions for the text before the cursor, and where the word that
/// they replace starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completions {
    /// A byte offset into the text.
    pub start: usize,
    pub items: Vec<Completion>,
}

const LIMIT: usize = 50;

/// `here` is the node the formula is on, for `this` and `parent`.
pub fn complete(calc: &Calc, sheet: &str, here: &Path, text: &str, cursor: usize) -> Completions {
    let cursor = cursor.min(text.len());
    let cursor = (0..=cursor)
        .rev()
        .find(|&i| text.is_char_boundary(i))
        .unwrap_or(0);
    let before = &text[..cursor];
    let chain_start = chain_start(before);
    let chain = &before[chain_start..];
    // Names inside a string are not completed.
    if inside_string(before) {
        return Completions {
            start: cursor,
            items: Vec::new(),
        };
    }
    match chain.rfind('.') {
        None => Completions {
            start: chain_start,
            items: names(calc, chain),
        },
        Some(dot) => {
            let head = &chain[..dot];
            let prefix = &chain[dot + 1..];
            Completions {
                start: chain_start + dot + 1,
                items: fields(calc, sheet, here, head, prefix),
            }
        }
    }
}

/// Where the dotted name that ends at the cursor starts: letters, digits,
/// underscores, dots, and bracketed indexes.
fn chain_start(before: &str) -> usize {
    let bytes = before.as_bytes();
    let mut i = bytes.len();
    while i > 0 {
        let c = bytes[i - 1];
        if c == b']' {
            // Skip back over the brackets.
            let mut depth = 0;
            let mut j = i;
            while j > 0 {
                match bytes[j - 1] {
                    b']' => depth += 1,
                    b'[' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                j -= 1;
            }
            if j == 0 {
                return i;
            }
            i = j - 1;
        } else if c.is_ascii_alphanumeric() || c == b'_' || c == b'.' || c >= 0x80 {
            i -= 1;
        } else {
            break;
        }
    }
    i
}

fn inside_string(before: &str) -> bool {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for c in before.chars() {
        match quote {
            Some(q) => {
                if escaped {
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else if c == q {
                    quote = None;
                }
            }
            None if c == '\'' || c == '"' => quote = Some(c),
            None => {}
        }
    }
    quote.is_some()
}

fn starts_with_ignoring_case(word: &str, prefix: &str) -> bool {
    word.to_lowercase().starts_with(&prefix.to_lowercase())
}

fn names(calc: &Calc, prefix: &str) -> Vec<Completion> {
    let mut out = Vec::new();
    for tab in calc.sheet_names() {
        if starts_with_ignoring_case(tab, prefix) {
            out.push(Completion {
                label: tab.to_owned(),
                insert: tab.to_owned(),
                kind: CompletionKind::Tab,
                detail: "tab".to_owned(),
            });
        }
    }
    for (name, detail) in [
        ("this", "this node"),
        ("parent", "the node that holds this one"),
        ("root", "the root of this tab"),
    ] {
        if starts_with_ignoring_case(name, prefix) {
            out.push(Completion {
                label: name.to_owned(),
                insert: name.to_owned(),
                kind: CompletionKind::Relative,
                detail: detail.to_owned(),
            });
        }
    }
    for function in catalog() {
        if starts_with_ignoring_case(function.name, prefix) {
            out.push(Completion {
                label: function.name.to_owned(),
                insert: function.insert.to_owned(),
                kind: if function.group == Group::Modules {
                    CompletionKind::Module
                } else {
                    CompletionKind::Function
                },
                detail: function.signature.to_owned(),
            });
        }
    }
    // A prefix that is a whole name and nothing more needs no popup.
    if out.len() == 1 && out[0].label == prefix && out[0].kind != CompletionKind::Function {
        // Keep it: accepting it adds the dot of a module.
    }
    out.truncate(LIMIT);
    out
}

/// The steps of `Orders.items[0].price`-style text after its first name.
fn parse_chain(head: &str) -> Option<(&str, Vec<Segment>)> {
    let end = head.find(['.', '[']).unwrap_or(head.len());
    let first = &head[..end];
    if first.is_empty() {
        return None;
    }
    let mut rest = &head[end..];
    let mut steps = Vec::new();
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix('.') {
            let len = after.find(['.', '[']).unwrap_or(after.len());
            if len == 0 {
                return None;
            }
            steps.push(Segment::Key(after[..len].to_owned()));
            rest = &after[len..];
        } else {
            let after = rest.strip_prefix('[')?;
            let close = after.find(']')?;
            let inside = after[..close].trim();
            let step = if let Ok(n) = inside.parse::<i64>() {
                // A negative index is some item; which one does not matter
                // for the type.
                Segment::Index(n.unsigned_abs() as usize)
            } else if inside.len() >= 2
                && (inside.starts_with('\'') || inside.starts_with('"'))
                && inside.ends_with(inside.chars().next()?)
            {
                Segment::Key(inside[1..inside.len() - 1].to_owned())
            } else {
                return None;
            };
            steps.push(step);
            rest = &after[close + 1..];
        }
    }
    Some((first, steps))
}

fn fields(calc: &Calc, sheet: &str, here: &Path, head: &str, prefix: &str) -> Vec<Completion> {
    let Some((first, steps)) = parse_chain(head) else {
        return Vec::new();
    };
    // Where the chain starts.
    let (tab, mut path) = match first {
        "this" => (sheet, here.clone()),
        "parent" => (sheet, here.parent().unwrap_or_default()),
        "root" => (sheet, Path::root()),
        name => {
            if !calc.sheet_names().any(|t| t == name) {
                return Vec::new();
            }
            (name, Path::root())
        }
    };
    let Some(spec) = calc.spec(tab) else {
        return Vec::new();
    };
    for step in steps {
        path.push(step);
    }
    let Some(resolved) = spec.resolve(&path) else {
        return Vec::new();
    };
    // On an array, the fields are those of its items: `Orders.items.price`.
    let (ty, each) = match &spec.get(resolved.ty).kind {
        Kind::Array { items, .. } => (*items, true),
        _ => (resolved.ty, false),
    };
    field_completions(spec, ty, prefix, each)
}

fn field_completions(spec: &TypeSpec, ty: TypeId, prefix: &str, each: bool) -> Vec<Completion> {
    let Kind::Object { fields } = &spec.get(ty).kind else {
        return Vec::new();
    };
    fields
        .iter()
        .filter(|f| is_identifier(&f.name) && starts_with_ignoring_case(&f.name, prefix))
        .map(|f| {
            let mut detail = spec.describe(f.ty);
            if !f.required {
                detail.push_str(", optional");
            }
            if each {
                detail.push_str(", of each item");
            }
            Completion {
                label: f.name.clone(),
                insert: f.name.clone(),
                kind: CompletionKind::Field,
                detail,
            }
        })
        .take(LIMIT)
        .collect()
}

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_alphabetic() || c == '_')
        && chars.all(|c| c.is_alphanumeric() || c == '_')
        && !name.starts_with("__")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::python::PythonSyntax;
    use cliotree_core::{json, jsonschema};
    use std::sync::Arc;

    const SCHEMA: &str = r#"{
      "type": "object",
      "properties": {
        "customer": {"type": "object", "properties": {
            "name": {"type": "string"}, "city": {"type": "string"}, "vip": {"type": "boolean"}
          }, "required": ["name"]},
        "items": {"type": "array", "items": {"type": "object", "properties": {
            "qty": {"type": "integer"}, "price": {"type": "number"}, "tag": {"type": "string"},
            "my key": {"type": "string"}
          }, "required": ["qty", "price"]}},
        "prices": {"type": "object", "additionalProperties": {"type": "number"}},
        "total": {"type": "number"}
      },
      "required": ["customer", "items", "prices", "total"]
    }"#;

    fn book() -> Calc {
        let mut calc = Calc::new(Arc::new(PythonSyntax));
        calc.add_sheet(
            "Orders",
            jsonschema::lower_str(SCHEMA).unwrap(),
            json::parse(r#"{"customer": {"name": "x"}, "items": [], "prices": {}, "total": 0}"#)
                .unwrap()
                .node,
        )
        .unwrap();
        let other = r#"{"type":"object","properties":{"n":{"type":"integer"}},"required":["n"]}"#;
        calc.add_sheet(
            "Other",
            jsonschema::lower_str(other).unwrap(),
            json::parse(r#"{"n": 1}"#).unwrap().node,
        )
        .unwrap();
        calc
    }

    fn at(text: &str) -> Completions {
        complete(
            &book(),
            "Orders",
            &"items[1].total".parse().unwrap(),
            text,
            text.len(),
        )
    }

    fn labels(c: &Completions) -> Vec<&str> {
        c.items.iter().map(|i| i.label.as_str()).collect()
    }

    #[test]
    fn names_offer_tabs_relative_names_functions_and_modules() {
        let c = at("O");
        assert_eq!(c.start, 0);
        let l = labels(&c);
        assert!(l.contains(&"Orders") && l.contains(&"Other"));
        assert!(!l.contains(&"Customer"));
        let c = at("sum(Or");
        assert_eq!(c.start, 4);
        assert!(labels(&c).contains(&"Orders"));
        // Case does not matter for the match.
        assert!(labels(&at("iferr")).contains(&"IFERROR"));
        assert!(labels(&at("ma")).contains(&"math"));
        let math = at("ma")
            .items
            .into_iter()
            .find(|i| i.label == "math")
            .unwrap();
        assert_eq!(math.insert, "math.");
        let fun = at("IFE")
            .items
            .into_iter()
            .find(|i| i.label == "IFERROR")
            .unwrap();
        assert_eq!(fun.insert, "IFERROR(");
        assert_eq!(fun.kind, CompletionKind::Function);
        let c = at("p");
        assert!(labels(&c).contains(&"parent"));
        assert!(!at("").items.is_empty(), "everything, for an empty word");
    }

    /// After a tab name and a dot, the fields of that node, from the schema.
    #[test]
    fn a_dot_after_a_tab_offers_its_fields_in_declared_order() {
        let c = at("Orders.");
        assert_eq!(labels(&c), ["customer", "items", "prices", "total"]);
        assert_eq!(c.start, "Orders.".len());
        let c = at("1 + Orders.cu");
        assert_eq!(labels(&c), ["customer"]);
        assert_eq!(c.start, "1 + Orders.".len());
        let details: Vec<_> = at("Orders.")
            .items
            .iter()
            .map(|i| i.detail.clone())
            .collect();
        assert_eq!(details[1], "array of object");
        assert_eq!(details[3], "number");
    }

    #[test]
    fn it_goes_on_through_the_path_with_dots_and_indexes() {
        assert_eq!(labels(&at("Orders.customer.")), ["name", "city", "vip"]);
        assert_eq!(
            at("Orders.customer.").items[1].detail,
            "string, optional",
            "city is optional"
        );
        assert_eq!(labels(&at("Orders.items[0].")), ["qty", "price", "tag"]);
        assert_eq!(labels(&at("Orders.items[-1].p")), ["price"]);
        assert_eq!(labels(&at("Orders.items[0]['tag'].")), Vec::<&str>::new());
        assert_eq!(
            labels(&at("f(Orders.items[2].qty, Orders.customer.n")),
            ["name"]
        );
    }

    /// `Orders.items.price` is that field of every item.
    #[test]
    fn an_array_offers_the_fields_of_its_items() {
        let c = at("Orders.items.");
        assert_eq!(labels(&c), ["qty", "price", "tag"]);
        assert!(c.items[0].detail.contains("of each item"));
    }

    #[test]
    fn names_that_cannot_be_written_as_attributes_are_left_out() {
        assert!(!labels(&at("Orders.items[0].")).contains(&"my key"));
    }

    #[test]
    fn this_parent_and_root_start_a_chain() {
        // The formula is on Orders.items[1].total.
        assert_eq!(labels(&at("parent.")), ["qty", "price", "tag"]);
        assert_eq!(
            labels(&at("root.")),
            ["customer", "items", "prices", "total"]
        );
        assert_eq!(labels(&at("root.items[0].q")), ["qty"]);
    }

    #[test]
    fn nothing_is_offered_where_it_would_be_wrong() {
        assert!(at("Orders.prices.").items.is_empty(), "a map has no fields");
        assert!(at("Orders.total.").items.is_empty(), "a number has none");
        assert!(at("Orders.nothing.").items.is_empty());
        assert!(at("Nothing.").items.is_empty());
        assert!(at("'Orders.").items.is_empty(), "inside a string");
        assert!(at("\"say Orders.c").items.is_empty());
        assert!(
            !at("'done' + Or").items.is_empty(),
            "after the string it ends"
        );
        assert!(
            at("len(x).").items.is_empty(),
            "arbitrary objects are not completed"
        );
    }

    #[test]
    fn the_cursor_can_be_in_the_middle() {
        let text = "Orders.cu + 1";
        let c = complete(&book(), "Orders", &Path::root(), text, "Orders.cu".len());
        assert_eq!(labels(&c), ["customer"]);
        assert_eq!(c.start, 7);
        // A cursor past the end, or inside a multi-byte letter, does not panic.
        let c = complete(&book(), "Orders", &Path::root(), "Ordérs.", 5);
        let _ = c;
        let c = complete(&book(), "Orders", &Path::root(), "Orders.", 99);
        assert_eq!(c.items.len(), 4);
    }
}
