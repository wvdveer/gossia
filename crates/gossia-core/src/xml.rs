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
//! What is kept: the declaration, the document type declaration and the
//! processing instructions outside the root, the prefix and the namespace
//! declarations of the root (in [`XmlDocument`]), the namespace declarations
//! of other elements (on their nodes), attribute order, element order, text
//! exactly, and comments: the comment before an element is that node's
//! comment, and a comment inside an element that is before no child element
//! is the node's comment at the start (before its text) or at the end
//! ([`CommentPlace`]). An element of mixed content holds its inside as
//! written, a string of markup ([`crate::StringFormat::Markup`]).
//! What is not kept: see [`crate::xmltree`] and DESIGN.md, "Not preserved
//! (accepted)".

use gossia_i18n::tr;
use std::cell::RefCell;
use std::collections::HashMap;

use chrono::{DateTime, NaiveDate, NaiveDateTime};
use indexmap::IndexMap;

use crate::json::{Warning, WriteError};
use crate::path::Path;
use crate::typespec::{Field, Kind, TypeId, TypeSpec};
use crate::value::{CommentPlace, DateTimeValue, Node, Value};
use crate::xmltree::{self, Attribute, Child, Declaration, Element, Misc, XmlError};

/// What a document says about itself beyond its tree.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct XmlDocument {
    pub declaration: Option<Declaration>,
    /// The prefix the root element was written with.
    pub root_prefix: Option<String>,
    /// The namespace declarations on the root element.
    pub declarations: Vec<(Option<String>, String)>,
    /// The document type declaration and processing instructions before the
    /// root element, in order. Comments there are the root node's comment.
    #[serde(default)]
    pub prolog: Vec<Misc>,
    /// Comments and processing instructions after the root element.
    pub after: Vec<Misc>,
    /// The file had no XML declaration, so none is written. A new document
    /// (where `declaration` is `None` too) gets the default one.
    #[serde(default)]
    pub without_declaration: bool,
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
            message: tr!("the schema is not an XSD, so it cannot describe XML data").to_string(),
        });
    };
    let doc = xmltree::parse(text)?;
    if doc.root.name != info.root {
        return Err(XmlError {
            position: None,
            message: tr!(
                "the root element is `{name}`, but the schema describes `{root}`",
                name = doc.root.name,
                root = info.root
            ),
        });
    }
    if doc.root.namespace != info.target_namespace {
        return Err(XmlError {
            position: None,
            message: tr!(
                "the root element is in {namespace}, but the schema's namespace is {target}",
                namespace = doc.root.namespace.as_deref().unwrap_or(tr!("no namespace")),
                target = info
                    .target_namespace
                    .as_deref()
                    .unwrap_or(tr!("no namespace")),
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
    // The root's declarations are the document's.
    node.set_xml_declarations(Vec::new());
    let comments: Vec<&str> = doc.before.iter().filter_map(Misc::comment).collect();
    if !comments.is_empty() {
        node.set_comment(Some(comments.join("\n")));
    }
    let prolog = doc
        .before
        .iter()
        .filter(|m| m.comment().is_none())
        .cloned()
        .collect();
    Ok(Parsed {
        node,
        document: XmlDocument {
            without_declaration: doc.declaration.is_none(),
            declaration: doc.declaration,
            root_prefix: doc.root.prefix.clone(),
            declarations: doc.root.declarations.clone(),
            prolog,
            after: doc.after,
        },
        warnings: reader.warnings,
    })
}

struct Reader<'a> {
    spec: &'a TypeSpec,
    warnings: Vec<Warning>,
}

/// A field's name without its `@` and prefix.
fn local_name(field: &str) -> &str {
    let name = field.strip_prefix('@').unwrap_or(field);
    name.rsplit_once(':').map_or(name, |(_, local)| local)
}

impl Reader<'_> {
    /// The key of content the schema does not define: bare when it is in the
    /// schema's namespace or in none, or written without a prefix; otherwise
    /// with the prefix it was written with.
    fn undefined_key(&self, namespace: Option<&str>, prefix: Option<&str>, local: &str) -> String {
        let target = self.spec.xml().and_then(|x| x.target_namespace.as_deref());
        match prefix {
            Some(p) if namespace.is_some() && namespace != target => format!("{p}:{local}"),
            _ => local.to_string(),
        }
    }

    /// The key of a child element: the field for its namespace and name, or
    /// what [`Self::undefined_key`] says.
    fn element_key(&self, fields: &[Field], e: &Element) -> String {
        fields
            .iter()
            .find(|f| {
                !f.name.starts_with('@')
                    && f.name != "#text"
                    && local_name(&f.name) == e.name
                    && f.namespace == e.namespace
            })
            .map(|f| f.name.clone())
            .unwrap_or_else(|| {
                self.undefined_key(e.namespace.as_deref(), e.prefix.as_deref(), &e.name)
            })
    }

    /// The key of an attribute, as [`Self::element_key`] does for elements.
    fn attribute_key(&self, fields: &[Field], a: &Attribute) -> String {
        fields
            .iter()
            .find(|f| {
                f.name.starts_with('@')
                    && local_name(&f.name) == a.name
                    && f.namespace == a.namespace
            })
            .map(|f| f.name.clone())
            .unwrap_or_else(|| {
                format!(
                    "@{}",
                    self.undefined_key(a.namespace.as_deref(), a.prefix.as_deref(), &a.name)
                )
            })
    }

    fn warn(&mut self, path: &Path, message: impl Into<String>) {
        self.warnings.push(Warning {
            path: path.clone(),
            message: message.into(),
        });
    }

    /// The node for an element, typed as `ty`, with the comments inside it
    /// (those of mixed content are part of its markup) and the namespaces it
    /// declares.
    fn element(&mut self, element: &Element, ty: TypeId, path: &Path) -> Node {
        let inner = if holds_markup(self.spec, ty) {
            (None, None)
        } else {
            self.comments_inside(element, path)
        };
        let mut node = self.element_value(element, ty, path);
        node.set_xml_declarations(element.declarations.clone());
        with_inner(node, inner)
    }

    fn element_value(&mut self, element: &Element, ty: TypeId, path: &Path) -> Node {
        match &self.spec.get(ty).kind {
            Kind::Object { fields } => {
                let fields = fields.clone();
                self.object(element, &fields, path)
            }
            kind if is_markup(kind) => {
                if !element.attributes.is_empty() {
                    self.warn(
                        path,
                        tr!("attributes on an element of a simple type were dropped"),
                    );
                }
                Node::new(Value::Str(xmltree::write_fragment(&element.children)))
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
                        tr!("attributes on an element of a simple type were dropped"),
                    );
                }
                typed(&kind, &element.text())
            }
        }
    }

    /// The comments inside an element that are before no child element
    /// (those are the child's): the ones before its text, at the start, and
    /// the rest, at the end. A comment inside the text, or after a child
    /// element and before text, goes to the end, with a warning, since the
    /// text is one value.
    fn comments_inside(&mut self, element: &Element, path: &Path) -> Inner {
        let (mut start, mut end, mut pending) = (Vec::new(), Vec::new(), Vec::new());
        let (mut content, mut moved) = (false, false);
        for child in &element.children {
            match child {
                Child::Comment(c) => pending.push(c.as_str()),
                Child::Element(_) => {
                    pending.clear();
                    content = true;
                }
                Child::Text(t) if t.trim().is_empty() => {}
                Child::Text(_) => {
                    if !pending.is_empty() {
                        if content {
                            moved = true;
                            end.append(&mut pending);
                        } else {
                            start.append(&mut pending);
                        }
                    }
                    content = true;
                }
            }
        }
        end.append(&mut pending);
        if moved {
            self.warn(
                path,
                tr!("a comment inside the text was moved to the end of the element"),
            );
        }
        let joined = |list: Vec<&str>| (!list.is_empty()).then(|| list.join("\n"));
        (joined(start), joined(end))
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
            attributes.insert(self.attribute_key(fields, a), a);
        }
        for field in fields.iter().filter(|f| f.name.starts_with('@')) {
            if let Some(a) = attributes.remove(&field.name) {
                let kind = self.spec.get(field.ty).kind.clone();
                map.insert(field.name.clone(), typed(&kind, &a.value));
            }
        }
        for a in &element.attributes {
            let key = self.attribute_key(fields, a);
            if attributes.contains_key(&key) {
                map.insert(key, Node::new(Value::Str(a.value.clone())));
            }
        }
        // The text.
        let text_field = fields.iter().find(|f| f.name == "#text");
        if let Some(field) = text_field.filter(|f| is_markup(&self.spec.get(f.ty).kind)) {
            // Mixed content: everything inside is the markup.
            let markup = xmltree::write_fragment(&element.children);
            if !markup.is_empty() || field.required {
                map.insert("#text".to_string(), Node::new(Value::Str(markup)));
            }
            return Node::new(Value::Object(map));
        }
        let has_children = element.elements().next().is_some();
        let text = element.text();
        match text_field {
            Some(field) => {
                if has_children && element.has_text() {
                    self.warn(path, tr!("text next to child elements was kept as text"));
                }
                if !text.is_empty() || field.required {
                    let kind = self.spec.get(field.ty).kind.clone();
                    map.insert("#text".to_string(), typed(&kind, &text));
                }
            }
            None if element.has_text() => {
                map.insert("#text".to_string(), Node::new(Value::Str(text)));
            }
            None => {}
        }
        if let Some((list, choices)) = self.children_list(fields) {
            let items = self.children(element, &choices, &path.key("#children"));
            if !items.is_empty() || list.required {
                map.insert("#children".to_string(), Node::new(Value::Array(items)));
            }
            return Node::new(Value::Object(map));
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
                    groups
                        .entry(self.element_key(fields, e))
                        .or_default()
                        .push((e, comment));
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

    /// The field `#children` among `fields`, if there is one: the type of the
    /// list and the fields of its choice.
    fn children_list<'f>(&self, fields: &'f [Field]) -> Option<(&'f Field, Vec<Field>)> {
        let field = fields.iter().find(|f| f.name == "#children")?;
        let Kind::Array { items, .. } = &self.spec.get(field.ty).kind else {
            return None;
        };
        match &self.spec.get(*items).kind {
            Kind::Choice { fields } => Some((field, fields.clone())),
            _ => None,
        }
    }

    /// The child elements as one ordered list, each item `{name: value}`;
    /// one the choice does not name is kept untyped, for validation to flag.
    /// A comment goes with the element after it.
    fn children(&mut self, element: &Element, choices: &[Field], path: &Path) -> Vec<Node> {
        let mut items: Vec<Node> = Vec::new();
        let mut pending: Vec<&str> = Vec::new();
        for child in &element.children {
            match child {
                Child::Comment(c) => pending.push(c),
                Child::Element(e) => {
                    let comment = (!pending.is_empty()).then(|| {
                        pending.join(
                            "
",
                        )
                    });
                    pending.clear();
                    let key = self.element_key(choices, e);
                    let here = path.index(items.len()).key(key.clone());
                    let node = match choices.iter().find(|f| f.name == key) {
                        Some(field) => self.element(e, field.ty, &here),
                        None => self.generic(e, &here),
                    };
                    let mut item = IndexMap::new();
                    item.insert(key, node.with_comment(comment));
                    items.push(Node::new(Value::Object(item)));
                }
                Child::Text(_) => {}
            }
        }
        items
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
    /// repeated children as arrays, every simple value a string. Its comments
    /// are kept as on any element.
    fn generic(&mut self, element: &Element, path: &Path) -> Node {
        let inner = self.comments_inside(element, path);
        let mut node = self.generic_value(element, path);
        node.set_xml_declarations(element.declarations.clone());
        with_inner(node, inner)
    }

    fn generic_value(&mut self, element: &Element, path: &Path) -> Node {
        let has_children = element.elements().next().is_some();
        if element.attributes.is_empty() && !has_children {
            return Node::new(Value::Str(element.text()));
        }
        let mut map: IndexMap<String, Node> = IndexMap::new();
        for a in &element.attributes {
            map.insert(
                self.attribute_key(&[], a),
                Node::new(Value::Str(a.value.clone())),
            );
        }
        if element.has_text() {
            map.insert("#text".to_string(), Node::new(Value::Str(element.text())));
        }
        let mut groups: IndexMap<String, Vec<(&Element, Option<String>)>> = IndexMap::new();
        let mut pending: Vec<&str> = Vec::new();
        for child in &element.children {
            match child {
                Child::Comment(c) => pending.push(c),
                Child::Element(e) => {
                    let comment = (!pending.is_empty()).then(|| pending.join("\n"));
                    pending.clear();
                    groups
                        .entry(self.element_key(&[], e))
                        .or_default()
                        .push((e, comment));
                }
                Child::Text(_) => {}
            }
        }
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
}

/// The comments inside an element: at its start and at its end.
type Inner = (Option<String>, Option<String>);

/// Whether values of this kind are markup, the inside of mixed content.
fn is_markup(kind: &Kind) -> bool {
    kind.is_markup()
}

/// Whether an element of this type holds mixed content: it is markup, or its
/// text is.
fn holds_markup(spec: &TypeSpec, ty: TypeId) -> bool {
    match &spec.get(ty).kind {
        Kind::Object { fields } => fields
            .iter()
            .any(|f| f.name == "#text" && is_markup(&spec.get(f.ty).kind)),
        kind => is_markup(kind),
    }
}

fn with_inner(mut node: Node, (start, end): Inner) -> Node {
    node.set_comment_at(CommentPlace::Start, start);
    node.set_comment_at(CommentPlace::End, end);
    node
}

/// The node for the text of an element or attribute, typed as `kind`. When the
/// value would be written differently (`1.0` read as 1), the text is kept
/// with it, so that the file is written back as it was.
fn typed(kind: &Kind, text: &str) -> Node {
    let value = scalar_of(kind, text);
    let mut node = Node::new(value);
    if !matches!(node.value(), Value::Str(_)) && scalar_text(node.value()).as_deref() != Some(text)
    {
        node.set_lexical(Some(text.to_string()));
    }
    node
}

/// Reads the text of an element or attribute as a value of `kind`. Text that is
/// not a valid literal of the kind stays a string.
fn scalar_of(kind: &Kind, text: &str) -> Value {
    let trimmed = text.trim();
    match kind {
        Kind::String(_) | Kind::Enum { .. } | Kind::Any => Value::Str(text.to_string()),
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
        Kind::Array { .. } | Kind::Object { .. } | Kind::Choice { .. } | Kind::Map { .. } => {
            Value::Str(text.to_string())
        }
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
            message: tr!("the schema is not an XSD, so it cannot describe XML data").to_string(),
        });
    };
    if matches!(node.value(), Value::Error(_)) {
        return Err(WriteError {
            path: Path::root(),
            message: tr!("the root holds an error value").to_string(),
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
    let writer = Writer {
        spec,
        target: info.target_namespace.clone(),
        root_prefix: root_prefix.clone(),
        prefix: child_prefix,
        added: RefCell::new(Vec::new()),
    };
    let mut root = writer.emit(&info.root, node, Some(spec.root()), &declarations);
    root.prefix = root_prefix;
    for added in writer.added.into_inner() {
        if !declarations.contains(&added) {
            declarations.push(added);
        }
    }
    root.declarations = declarations;
    let mut before = document.prolog.clone();
    before.extend(node.comment().map(|c| Misc::Comment(c.to_string())));
    let doc = xmltree::Document {
        declaration: if document.without_declaration {
            None
        } else {
            Some(document.declaration.clone().unwrap_or_default())
        },
        before,
        root,
        after: document.after.clone(),
        warnings: Vec::new(),
    };
    Ok(xmltree::write(&doc, pretty))
}

/// Writes nodes as elements. The type of a node, when the schema has one,
/// says whether a string is markup, written as the elements it holds, and the
/// namespace of each field says which prefix it is written with.
struct Writer<'a> {
    spec: &'a TypeSpec,
    /// The schema's namespace.
    target: Option<String>,
    /// The prefix the root is written with, which is the target namespace's.
    root_prefix: Option<String>,
    /// The prefix the elements below the root are written with when they are
    /// in the target namespace and the schema qualifies them.
    prefix: Option<String>,
    /// Declarations the root needs for prefixes the document did not declare.
    added: RefCell<Vec<(Option<String>, String)>>,
}

/// The namespace declarations in scope: the root's, then those of each
/// element on the way down.
type Scope = [(Option<String>, String)];

impl Writer<'_> {
    /// The field of a record of type `ty`.
    fn field(&self, ty: Option<TypeId>, name: &str) -> Option<&Field> {
        self.spec
            .get(ty?)
            .kind
            .fields()?
            .iter()
            .find(|f| f.name == name)
    }

    /// The type of a field of a record of type `ty`.
    fn field_type(&self, ty: Option<TypeId>, name: &str) -> Option<TypeId> {
        self.field(ty, name).map(|f| f.ty)
    }

    fn markup(&self, ty: Option<TypeId>) -> bool {
        ty.is_some_and(|ty| is_markup(&self.spec.get(ty).kind))
    }

    /// The text of a scalar node: as it was written in the file while it
    /// still reads as the same value, and as it is written now otherwise.
    fn text_of(&self, node: &Node, ty: Option<TypeId>) -> Option<String> {
        if let (Some(lexical), Some(ty)) = (node.lexical(), ty)
            && scalar_of(&self.spec.get(ty).kind, lexical) == *node.value()
        {
            return Some(lexical.to_string());
        }
        scalar_text(node.value())
    }

    /// Makes sure `prefix` stands for `uri` where it is used: if nothing in
    /// scope binds it, the root declares it.
    fn declare(&self, prefix: &str, uri: &str, scope: &Scope) {
        if prefix == "xml" {
            return;
        }
        let bound = scope
            .iter()
            .rev()
            .find(|(p, _)| p.as_deref() == Some(prefix))
            .is_some_and(|(_, u)| u == uri);
        let added = self
            .added
            .borrow()
            .iter()
            .any(|(p, _)| p.as_deref() == Some(prefix));
        if !bound && !added {
            self.added
                .borrow_mut()
                .push((Some(prefix.to_string()), uri.to_string()));
        }
    }

    /// The prefix for something in the target namespace that must have one
    /// (a qualified attribute): the root's, or `tns`, declared.
    fn target_prefix(&self, scope: &Scope) -> Option<String> {
        let target = self.target.as_deref()?;
        let prefix = self
            .root_prefix
            .clone()
            .unwrap_or_else(|| "tns".to_string());
        self.declare(&prefix, target, scope);
        Some(prefix)
    }

    /// The prefix and local name for a field `key`: from its namespace when
    /// the schema defines it, from the key itself otherwise.
    fn qualify(
        &self,
        key: &str,
        field: Option<&Field>,
        attribute: bool,
        scope: &Scope,
    ) -> (Option<String>, String) {
        let bare = key.strip_prefix('@').unwrap_or(key);
        let (written, local) = match bare.split_once(':') {
            Some((p, l)) => (Some(p.to_string()), l.to_string()),
            None => (None, bare.to_string()),
        };
        let Some(field) = field else {
            // Undefined content keeps the prefix it was read with.
            return match written {
                Some(p) => (Some(p), local),
                None if attribute => (None, local),
                None => (self.prefix.clone(), local),
            };
        };
        match field.namespace.as_deref() {
            None => (None, local),
            Some(uri) if Some(uri) == self.target.as_deref() => {
                if attribute {
                    (self.target_prefix(scope), local)
                } else {
                    (self.root_prefix.clone(), local)
                }
            }
            Some(uri) => {
                let prefix = written.unwrap_or_else(|| "ns".to_string());
                self.declare(&prefix, uri, scope);
                (Some(prefix), local)
            }
        }
    }

    /// The element for a node, named `name` (a field name, or the root's),
    /// with the comments inside it: the one at the start before its text,
    /// and the one at the end after the rest. With no text to stand before,
    /// the one at the start goes with the end.
    fn emit(&self, name: &str, node: &Node, ty: Option<TypeId>, scope: &Scope) -> Element {
        let mut element = Element::new(name);
        element.declarations = node.xml_declarations().to_vec();
        let mut inner: Vec<(Option<String>, String)> = scope.to_vec();
        inner.extend(element.declarations.iter().cloned());
        self.emit_value(&mut element, node, ty, &inner);
        let start = node.comment_at(CommentPlace::Start);
        let end = node.comment_at(CommentPlace::End);
        let text_first = matches!(element.children.first(), Some(Child::Text(_)));
        if let Some(start) = start.filter(|_| text_first) {
            element
                .children
                .insert(0, Child::Comment(start.to_string()));
        }
        if let Some(start) = start.filter(|_| !text_first) {
            element.children.push(Child::Comment(start.to_string()));
        }
        if let Some(end) = end {
            element.children.push(Child::Comment(end.to_string()));
        }
        element
    }

    fn emit_value(&self, element: &mut Element, node: &Node, ty: Option<TypeId>, scope: &Scope) {
        match node.value() {
            Value::Object(fields) => {
                for (key, child) in fields {
                    let field = self.field(ty, key);
                    if key.starts_with('@') {
                        if let Some(value) = self.text_of(child, field.map(|f| f.ty)) {
                            let (prefix, name) = self.qualify(key, field, true, scope);
                            element.attributes.push(Attribute {
                                prefix,
                                name,
                                value,
                                namespace: None,
                            });
                        }
                    } else if key == "#children" {
                        self.push_list(element, child, field.map(|f| f.ty), scope);
                    } else if key == "#text" {
                        let field = self.field_type(ty, key);
                        if let Some(text) = self.text_of(child, field)
                            && !text.is_empty()
                        {
                            push_text(element, text, self.markup(field));
                        }
                    } else {
                        self.push_children(element, key, child, field, scope);
                    }
                }
            }
            _ => {
                if let Some(text) = self.text_of(node, ty)
                    && !text.is_empty()
                {
                    push_text(element, text, self.markup(ty));
                }
            }
        }
    }

    /// Adds the elements of an ordered list `#children`, in its order: each
    /// item is `{name: value}`, and its type the list's choice.
    fn push_list(&self, parent: &mut Element, node: &Node, ty: Option<TypeId>, scope: &Scope) {
        let Value::Array(items) = node.value() else {
            return;
        };
        let choice = ty.and_then(|ty| match &self.spec.get(ty).kind {
            Kind::Array { items, .. } => Some(*items),
            _ => None,
        });
        for item in items {
            let Value::Object(fields) = item.value() else {
                continue;
            };
            for (key, child) in fields {
                self.push_children(parent, key, child, self.field(choice, key), scope);
            }
        }
    }

    /// Adds the element or elements a field stands for. A node that holds an
    /// error value, or null, is absent.
    fn push_children(
        &self,
        parent: &mut Element,
        key: &str,
        node: &Node,
        field: Option<&Field>,
        scope: &Scope,
    ) {
        match node.value() {
            Value::Error(_) | Value::Null => {}
            Value::Array(items) => {
                for child in items {
                    self.push_children(parent, key, child, field, scope);
                }
            }
            _ => {
                if let Some(comment) = node.comment() {
                    parent.children.push(Child::Comment(comment.to_string()));
                }
                // The type of one element: the field's, or its items'.
                let ty = field.map(|f| match &self.spec.get(f.ty).kind {
                    Kind::Array { items, .. } => *items,
                    _ => f.ty,
                });
                let (prefix, local) = self.qualify(key, field, false, scope);
                let mut element = self.emit(key, node, ty, scope);
                element.name = local;
                element.prefix = prefix;
                // An unqualified element where the default namespace is the
                // target's would fall into it.
                let unqualified = field.is_some_and(|f| f.namespace.is_none());
                let default_is_target = self.target.is_some()
                    && scope
                        .iter()
                        .rev()
                        .find(|(p, _)| p.is_none())
                        .is_some_and(|(_, u)| Some(u) == self.target.as_ref());
                if unqualified
                    && default_is_target
                    && !element.declarations.iter().any(|(p, _)| p.is_none())
                {
                    element.declarations.push((None, String::new()));
                }
                parent.children.push(Child::Element(element));
            }
        }
    }
}

/// Adds text to an element: as it is, or, for markup, as the content it
/// holds. Markup that is not well-formed is written as text, so that nothing
/// is lost; validation says what is wrong with it.
fn push_text(element: &mut Element, text: String, markup: bool) {
    if markup && let Ok(children) = xmltree::parse_fragment(&text) {
        element.children.extend(children);
    } else {
        element.children.push(Child::Text(text));
    }
}

#[cfg(test)]
#[path = "xml_tests.rs"]
mod tests;
