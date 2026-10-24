//! Reading and writing XML data against a schema lowered from an XSD.
//!
//! XML text becomes a [`Node`] tree and back (DESIGN.md, "Core model"). An
//! attribute is a field `@name`, the text of an element that has attributes is
//! the field `#text`, a child element is a field named after it, and an element
//! that may repeat is an array. An element of a simple type with no attributes
//! is just its value. The evaluator never sees XML.
//!
//! Reading needs the schema, because the schema says which elements are
//! arrays and how text is typed. Text that does not fit its type is kept as a
//! string, so that it can be shown and corrected (invalid data is kept and
//! flagged, never refused); content the schema does not define is kept too,
//! under its own name, for validation to flag as unrecognised.
//!
//! What is kept: the declaration, the prefix and the namespace declarations of
//! the root (in [`XmlDocument`]), attribute order, element order, text exactly,
//! and the comment before an element (as that node's comment). What is not:
//! see [`crate::xmltree`] and DESIGN.md, "Not preserved (accepted)". A comment
//! anywhere but before an element or the root is dropped, with a warning.

use std::collections::HashMap;

use chrono::{DateTime, NaiveDate, NaiveDateTime};
use indexmap::IndexMap;

use crate::json::{Warning, WriteError};
use crate::path::Path;
use crate::typespec::{Kind, TypeId, TypeSpec};
use crate::value::{DateTimeValue, Node, Value};
use crate::xmltree::{self, Attribute, Child, Declaration, Element, XmlError};

/// What a document says about itself beyond its tree.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct XmlDocument {
    pub declaration: Option<Declaration>,
    /// The prefix the root element was written with.
    pub root_prefix: Option<String>,
    /// The namespace declarations on the root element.
    pub declarations: Vec<(Option<String>, String)>,
    /// Comments after the root element.
    pub after: Vec<String>,
}

/// The result of reading XML text.
#[derive(Debug, Clone)]
pub struct Parsed {
    pub node: Node,
    pub document: XmlDocument,
    pub warnings: Vec<Warning>,
}

/// The text of a scalar as XML writes it, or `None` if there is none (a null,
/// an error value, a container).
pub fn scalar_text(value: &Value) -> Option<String> {
    Some(match value {
        Value::Str(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Int(i) => i.to_string(),
        Value::Float(x) if x.is_nan() => "NaN".to_string(),
        Value::Float(x) if x.is_infinite() => if *x > 0.0 { "INF" } else { "-INF" }.to_string(),
        Value::Float(x) => x.to_string(),
        Value::Date(d) => d.format("%Y-%m-%d").to_string(),
        Value::DateTime(DateTimeValue::Naive(dt)) => dt.format("%Y-%m-%dT%H:%M:%S%.f").to_string(),
        Value::DateTime(DateTimeValue::Zoned(dt)) => dt.to_rfc3339(),
        Value::Null | Value::Array(_) | Value::Object(_) | Value::Error(_) => return None,
    })
}

/// Reads XML text as data of the schema's type.
pub fn parse(text: &str, spec: &TypeSpec) -> Result<Parsed, XmlError> {
    let Some(info) = spec.xml() else {
        return Err(XmlError {
            position: None,
            message: "the schema is not an XSD, so it cannot describe XML data".to_string(),
        });
    };
    let doc = xmltree::parse(text)?;
    if doc.root.name != info.root {
        return Err(XmlError {
            position: None,
            message: format!(
                "the root element is `{}`, but the schema describes `{}`",
                doc.root.name, info.root
            ),
        });
    }
    if doc.root.namespace != info.target_namespace {
        return Err(XmlError {
            position: None,
            message: format!(
                "the root element is in {}, but the schema's namespace is {}",
                doc.root.namespace.as_deref().unwrap_or("no namespace"),
                info.target_namespace.as_deref().unwrap_or("no namespace"),
            ),
        });
    }
    let mut reader = Reader {
        spec,
        warnings: doc
            .warnings
            .iter()
            .map(|message| Warning {
                path: Path::root(),
                message: message.clone(),
            })
            .collect(),
    };
    let mut node = reader.element(&doc.root, spec.root(), &Path::root());
    if !doc.before.is_empty() {
        node.set_comment(Some(doc.before.join("\n")));
    }
    Ok(Parsed {
        node,
        document: XmlDocument {
            declaration: doc.declaration,
            root_prefix: doc.root.prefix.clone(),
            declarations: doc.root.declarations.clone(),
            after: doc.after,
        },
        warnings: reader.warnings,
    })
}

struct Reader<'a> {
    spec: &'a TypeSpec,
    warnings: Vec<Warning>,
}

impl Reader<'_> {
    fn warn(&mut self, path: &Path, message: impl Into<String>) {
        self.warnings.push(Warning {
            path: path.clone(),
            message: message.into(),
        });
    }

    /// The node for an element, typed as `ty`.
    fn element(&mut self, element: &Element, ty: TypeId, path: &Path) -> Node {
        self.comments_inside(element, path);
        match &self.spec.get(ty).kind {
            Kind::Object { fields } => {
                let fields = fields.clone();
                self.object(element, &fields, path)
            }
            kind => {
                let kind = kind.clone();
                if element.elements().next().is_some() {
                    // Not a simple value: keep what is there, for validation to flag.
                    return self.generic(element, path);
                }
                if !element.attributes.is_empty() {
                    self.warn(
                        path,
                        "attributes on an element of a simple type were dropped",
                    );
                }
                Node::new(scalar_of(&kind, &element.text()))
            }
        }
    }

    /// Comments that are not before an element are not kept.
    fn comments_inside(&mut self, element: &Element, path: &Path) {
        let mut pending = false;
        for child in &element.children {
            match child {
                Child::Comment(_) => pending = true,
                Child::Element(_) => pending = false,
                Child::Text(t) if t.trim().is_empty() => {}
                Child::Text(_) => {
                    if pending {
                        self.warn(path, "a comment next to text was dropped");
                        pending = false;
                    }
                }
            }
        }
        if pending {
            self.warn(path, "a comment at the end of an element was dropped");
        }
    }

    fn object(
        &mut self,
        element: &Element,
        fields: &[crate::typespec::Field],
        path: &Path,
    ) -> Node {
        let mut map: IndexMap<String, Node> = IndexMap::new();
        // Attributes, in the order of the schema, then any the schema lacks.
        let mut attributes: HashMap<String, &Attribute> = HashMap::new();
        for a in &element.attributes {
            let key = match &a.prefix {
                Some(p) => format!("@{p}:{}", a.name),
                None => format!("@{}", a.name),
            };
            attributes.insert(key, a);
        }
        for field in fields.iter().filter(|f| f.name.starts_with('@')) {
            if let Some(a) = attributes.remove(&field.name) {
                let kind = self.spec.get(field.ty).kind.clone();
                map.insert(field.name.clone(), Node::new(scalar_of(&kind, &a.value)));
            }
        }
        for a in &element.attributes {
            let key = match &a.prefix {
                Some(p) => format!("@{p}:{}", a.name),
                None => format!("@{}", a.name),
            };
            if attributes.contains_key(&key) {
                map.insert(key, Node::new(Value::Str(a.value.clone())));
            }
        }
        // The text.
        let text_field = fields.iter().find(|f| f.name == "#text");
        let has_children = element.elements().next().is_some();
        let text = element.text();
        match text_field {
            Some(field) => {
                if has_children && element.has_text() {
                    self.warn(path, "text next to child elements was kept as text");
                }
                if !text.is_empty() || field.required {
                    let kind = self.spec.get(field.ty).kind.clone();
                    map.insert("#text".to_string(), Node::new(scalar_of(&kind, &text)));
                }
            }
            None if element.has_text() => {
                map.insert("#text".to_string(), Node::new(Value::Str(text)));
            }
            None => {}
        }
        // The child elements, grouped by name; a comment goes to the next element.
        let mut groups: IndexMap<String, Vec<(&Element, Option<String>)>> = IndexMap::new();
        let mut pending: Vec<&str> = Vec::new();
        for child in &element.children {
            match child {
                Child::Comment(c) => pending.push(c),
                Child::Element(e) => {
                    let comment = (!pending.is_empty()).then(|| pending.join("\n"));
                    pending.clear();
                    groups.entry(e.name.clone()).or_default().push((e, comment));
                }
                Child::Text(_) => {}
            }
        }
        for field in fields
            .iter()
            .filter(|f| !f.name.starts_with('@') && f.name != "#text")
        {
            let Some(found) = groups.shift_remove(&field.name) else {
                continue;
            };
            let here = path.key(field.name.clone());
            let node = match &self.spec.get(field.ty).kind {
                Kind::Array { items, .. } => {
                    let items = *items;
                    self.occurrences(&found, items, &here)
                }
                _ if found.len() == 1 => {
                    let (e, comment) = &found[0];
                    self.element(e, field.ty, &here)
                        .with_comment(comment.clone())
                }
                // More occurrences than the schema allows: keep them all.
                _ => self.occurrences(&found, field.ty, &here),
            };
            map.insert(field.name.clone(), node);
        }
        // Elements the schema does not define.
        for (name, found) in groups {
            let here = path.key(name.clone());
            let nodes: Vec<Node> = found
                .iter()
                .map(|(e, comment)| self.generic(e, &here).with_comment(comment.clone()))
                .collect();
            let node = if nodes.len() == 1 {
                nodes.into_iter().next().expect("one")
            } else {
                Node::new(Value::Array(nodes))
            };
            map.insert(name, node);
        }
        Node::new(Value::Object(map))
    }

    fn occurrences(
        &mut self,
        found: &[(&Element, Option<String>)],
        ty: TypeId,
        path: &Path,
    ) -> Node {
        let items: Vec<Node> = found
            .iter()
            .enumerate()
            .map(|(i, (e, comment))| {
                self.element(e, ty, &path.index(i))
                    .with_comment(comment.clone())
            })
            .collect();
        Node::new(Value::Array(items))
    }

    /// An element with no schema to type it: attributes, text and children, with
    /// repeated children as arrays, every simple value a string.
    fn generic(&mut self, element: &Element, path: &Path) -> Node {
        self.comments_inside(element, path);
        let has_children = element.elements().next().is_some();
        if element.attributes.is_empty() && !has_children {
            return Node::new(Value::Str(element.text()));
        }
        let mut map: IndexMap<String, Node> = IndexMap::new();
        for a in &element.attributes {
            let key = match &a.prefix {
                Some(p) => format!("@{p}:{}", a.name),
                None => format!("@{}", a.name),
            };
            map.insert(key, Node::new(Value::Str(a.value.clone())));
        }
        if element.has_text() {
            map.insert("#text".to_string(), Node::new(Value::Str(element.text())));
        }
        let mut groups: IndexMap<String, Vec<&Element>> = IndexMap::new();
        for e in element.elements() {
            groups.entry(e.name.clone()).or_default().push(e);
        }
        for (name, found) in groups {
            let here = path.key(name.clone());
            let nodes: Vec<Node> = found.iter().map(|e| self.generic(e, &here)).collect();
            let node = if nodes.len() == 1 {
                nodes.into_iter().next().expect("one")
            } else {
                Node::new(Value::Array(nodes))
            };
            map.insert(name, node);
        }
        Node::new(Value::Object(map))
    }
}

/// Reads the text of an element or attribute as a value of `kind`. Text that is
/// not a valid literal of the kind stays a string.
fn scalar_of(kind: &Kind, text: &str) -> Value {
    let trimmed = text.trim();
    match kind {
        Kind::String(_) | Kind::Enum { .. } => Value::Str(text.to_string()),
        Kind::Bool => match trimmed {
            "true" | "1" => Value::Bool(true),
            "false" | "0" => Value::Bool(false),
            _ => Value::Str(text.to_string()),
        },
        Kind::Integer(_) => match trimmed.strip_prefix('+').unwrap_or(trimmed).parse::<i64>() {
            Ok(i) => Value::Int(i),
            Err(_) => Value::Str(text.to_string()),
        },
        Kind::Number(_) => match trimmed {
            "NaN" => Value::Float(f64::NAN),
            "INF" => Value::Float(f64::INFINITY),
            "-INF" => Value::Float(f64::NEG_INFINITY),
            // Rust accepts words XML does not (`inf`, `infinity`); XML does not
            // have them, and a number written so stays text.
            t if t
                .chars()
                .any(|c| c.is_ascii_alphabetic() && c != 'e' && c != 'E') =>
            {
                Value::Str(text.to_string())
            }
            t => match t.parse::<f64>() {
                Ok(x) if x.is_finite() => Value::Float(x),
                _ => Value::Str(text.to_string()),
            },
        },
        Kind::Date => match NaiveDate::parse_from_str(trimmed, "%Y-%m-%d") {
            Ok(d) if trimmed.len() == 10 => Value::Date(d),
            _ => Value::Str(text.to_string()),
        },
        Kind::DateTime => {
            if let Ok(dt) = DateTime::parse_from_rfc3339(trimmed) {
                Value::DateTime(DateTimeValue::Zoned(dt))
            } else if let Ok(dt) = NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%dT%H:%M:%S%.f") {
                Value::DateTime(DateTimeValue::Naive(dt))
            } else {
                Value::Str(text.to_string())
            }
        }
        Kind::Array { .. } | Kind::Object { .. } | Kind::Map { .. } => Value::Str(text.to_string()),
    }
}

// ----------------------------------------------------------------- writing

/// Writes a tree as an XML document of the schema's root element.
///
/// A node that holds an error value is written **absent** (DESIGN.md, "Errors
/// in saved files"). A field is written as an attribute if its name starts with
/// `@`, as the text if it is `#text`, and as an element otherwise; an array is
/// written as repeated elements.
pub fn to_string(
    node: &Node,
    spec: &TypeSpec,
    document: &XmlDocument,
    pretty: bool,
) -> Result<String, WriteError> {
    let Some(info) = spec.xml() else {
        return Err(WriteError {
            path: Path::root(),
            message: "the schema is not an XSD, so it cannot describe XML data".to_string(),
        });
    };
    if matches!(node.value(), Value::Error(_)) {
        return Err(WriteError {
            path: Path::root(),
            message: "the root holds an error value".to_string(),
        });
    }
    // Which prefix the elements are written with, and what is declared.
    let mut declarations = document.declarations.clone();
    let mut root_prefix = document.root_prefix.clone();
    let mut child_prefix: Option<String> = None;
    if let Some(namespace) = &info.target_namespace {
        let bound = |p: &Option<String>| {
            declarations
                .iter()
                .any(|(q, uri)| q == p && uri == namespace)
        };
        if !bound(&root_prefix) {
            // A default namespace would put the unqualified local elements in
            // it as well, so those are written with a prefix of their own.
            root_prefix = if info.qualified {
                None
            } else {
                Some("tns".to_string())
            };
            declarations.retain(|(p, _)| p != &root_prefix);
            declarations.push((root_prefix.clone(), namespace.clone()));
        } else if root_prefix.is_none() && !info.qualified {
            root_prefix = Some("tns".to_string());
            declarations.retain(|(p, _)| p != &root_prefix);
            declarations.push((root_prefix.clone(), namespace.clone()));
        }
        if info.qualified {
            child_prefix = root_prefix.clone();
        }
    } else {
        root_prefix = None;
    }
    let mut root = emit(&info.root, node, &child_prefix);
    root.prefix = root_prefix;
    root.declarations = declarations;
    let doc = xmltree::Document {
        declaration: Some(document.declaration.clone().unwrap_or_default()),
        before: node
            .comment()
            .map(|c| vec![c.to_string()])
            .unwrap_or_default(),
        root,
        after: document.after.clone(),
        warnings: Vec::new(),
    };
    Ok(xmltree::write(&doc, pretty))
}

/// The element for a node, named `name`.
fn emit(name: &str, node: &Node, prefix: &Option<String>) -> Element {
    let mut element = Element::new(name);
    element.prefix = prefix.clone();
    match node.value() {
        Value::Object(fields) => {
            for (key, child) in fields {
                if let Some(attribute) = key.strip_prefix('@') {
                    if let Some(value) = scalar_text(child.value()) {
                        let (p, local) = match attribute.split_once(':') {
                            Some((p, l)) => (Some(p.to_string()), l.to_string()),
                            None => (None, attribute.to_string()),
                        };
                        element.attributes.push(Attribute {
                            prefix: p,
                            name: local,
                            value,
                        });
                    }
                } else if key == "#text" {
                    if let Some(text) = scalar_text(child.value())
                        && !text.is_empty()
                    {
                        element.children.push(Child::Text(text));
                    }
                } else {
                    push_children(&mut element, key, child, prefix);
                }
            }
        }
        other => {
            if let Some(text) = scalar_text(other)
                && !text.is_empty()
            {
                element.children.push(Child::Text(text));
            }
        }
    }
    element
}

/// Adds the element or elements a field stands for. A node that holds an error
/// value, or null, is absent.
fn push_children(parent: &mut Element, name: &str, node: &Node, prefix: &Option<String>) {
    match node.value() {
        Value::Error(_) | Value::Null => {}
        Value::Array(items) => {
            for item in items {
                push_children(parent, name, item, prefix);
            }
        }
        _ => {
            if let Some(comment) = node.comment() {
                parent.children.push(Child::Comment(comment.to_string()));
            }
            parent
                .children
                .push(Child::Element(emit(name, node, prefix)));
        }
    }
}

#[cfg(test)]
#[path = "xml_tests.rs"]
mod tests;
