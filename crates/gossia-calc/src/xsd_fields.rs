//! The field commands on a tab whose schema is an XSD (DESIGN.md, "Editing the
//! schema from a data tab", "Attributes and elements"). The XSD tab holds the
//! document as a tree described by the XSD meta-schema: a record type is a
//! `complexType` node, its elements are the `element` items of its `sequence`,
//! and its attributes are its `attribute` items (or those of its
//! `simpleContent/extension`, when the element has text). A command changes
//! that node; the data is changed as it is for JSON Schema.

use gossia_core::xsd::{self, XSD_NS};
use gossia_core::{Node, Path, Value};
use indexmap::IndexMap;

use super::*;

/// What a command does to the `complexType` of the record.
pub(super) enum XsdChange<'a> {
    Add {
        /// `@name` for an attribute.
        name: &'a str,
        kind: FieldKind,
        required: bool,
    },
    Rename {
        from: &'a str,
        to: &'a str,
    },
    Delete {
        name: &'a str,
    },
    /// Every field, in its new order.
    Reorder {
        order: &'a [String],
    },
}

/// The order of what a `complexType` holds, as the meta-schema declares it,
/// so that something added is written in its place.
const COMPLEX_ORDER: &[&str] = &[
    "@name",
    "annotation",
    "sequence",
    "simpleContent",
    "attribute",
];
const EXTENSION_ORDER: &[&str] = &["@base", "attribute"];
const SEQUENCE_ORDER: &[&str] = &["annotation", "element"];

impl FieldKind {
    /// The built-in XSD type of a field of this type, without a prefix.
    fn xsd_type(self) -> &'static str {
        match self {
            FieldKind::Text => "string",
            FieldKind::Integer => "integer",
            FieldKind::Number => "decimal",
            FieldKind::Boolean => "boolean",
            FieldKind::Date => "date",
            FieldKind::DateTime => "dateTime",
            // Written as a complexType of their own, not a built-in type.
            FieldKind::Object | FieldKind::Array => "anyType",
        }
    }
}

/// Where a type whose lowering `source` is given is written in the XSD tab's
/// tree: `schema/element[order]/complexType` or `schema/type[Party]`.
pub(super) fn xsd_type_path(document: &Node, source: &str) -> Option<Path> {
    let mut parts = source.split('/');
    if parts.next()? != "schema" {
        return None;
    }
    let mut path = Path::root();
    let mut at_root = true;
    for part in parts {
        let (key, name) = match part.split_once('[') {
            Some((key, rest)) => (key, Some(rest.strip_suffix(']')?)),
            None => (part, None),
        };
        // A named type is one of the `complexType` items of the schema.
        let key = if key == "type" && at_root {
            "complexType"
        } else {
            key
        };
        at_root = false;
        path = path.key(key);
        let node = document.get(&path)?;
        if let Value::Array(items) = node.value() {
            let name = name?;
            let index = items
                .iter()
                .position(|item| declared_name(item) == Some(name))?;
            path = path.index(index);
        } else if let Some(name) = name
            && declared_name(node) != Some(name)
        {
            return None;
        }
    }
    Some(path)
}

/// The `name` of a declaration, or the local part of its `ref`.
fn declared_name(node: &Node) -> Option<&str> {
    let fields = node.value().as_object()?;
    if let Some(name) = fields.get("@name").and_then(|n| n.value().as_str()) {
        return Some(name);
    }
    let reference = fields.get("@ref")?.value().as_str()?;
    Some(reference.rsplit(':').next().unwrap_or(reference))
}

fn attribute_value(node: &Node, key: &str) -> Option<String> {
    node.value()
        .as_object()?
        .get(key)?
        .value()
        .as_str()
        .map(str::to_owned)
}

fn text(s: &str) -> Node {
    Node::new(Value::Str(s.to_owned()))
}

fn declaration(fields: &[(&str, String)]) -> Node {
    Node::new(Value::Object(
        fields
            .iter()
            .map(|(k, v)| ((*k).to_owned(), text(v)))
            .collect(),
    ))
}

/// Puts `key` in `map` before the first key that comes after it in `order`,
/// or at the end.
fn insert_ordered(map: &mut IndexMap<String, Node>, key: &str, node: Node, order: &[&str]) {
    let rank = |k: &str| order.iter().position(|o| *o == k).unwrap_or(usize::MAX);
    let mine = rank(key);
    let at = map.keys().position(|k| rank(k) > mine).unwrap_or(map.len());
    map.shift_insert(at, key.to_owned(), node);
}

/// The items of a list held at `key` of `map` (empty if there is none).
fn items_at(map: &IndexMap<String, Node>, key: &str) -> Vec<Node> {
    match map.get(key).map(Node::value) {
        Some(Value::Array(items)) => items.clone(),
        Some(Value::Object(_)) => vec![map[key].clone()],
        _ => Vec::new(),
    }
}

/// Sets the list at `key`: an empty list takes the key away.
fn set_items(map: &mut IndexMap<String, Node>, key: &str, items: Vec<Node>, order: &[&str]) {
    if items.is_empty() {
        map.shift_remove(key);
        return;
    }
    match map.get_mut(key) {
        Some(node) => *node.value_mut() = Value::Array(items),
        None => insert_ordered(map, key, Node::new(Value::Array(items)), order),
    }
}

fn object_of(node: &Node) -> Result<IndexMap<String, Node>, EditError> {
    match node.value() {
        Value::Object(map) => Ok(map.clone()),
        _ => Err(EditError::Path(
            "the type is not written as a complexType".to_owned(),
        )),
    }
}

/// A name XML allows for an element or an attribute, without a prefix.
fn check_xml_name(name: &str) -> Result<(), EditError> {
    if name.is_empty() {
        return Err(EditError::Path("a field needs a name".to_owned()));
    }
    let mut chars = name.chars();
    let first_ok = chars.next().is_some_and(|c| c.is_alphabetic() || c == '_');
    let rest_ok = chars.all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.'));
    let reserved = name.len() >= 3 && name[..3].eq_ignore_ascii_case("xml");
    if !first_ok || !rest_ok || reserved {
        return Err(EditError::Path(format!(
            "{name} is not a name XML allows: it starts with a letter or _, holds only \
             letters, digits, _, - and ., and does not start with xml"
        )));
    }
    Ok(())
}

impl Calc {
    /// The prefix the XSD tab binds to the XML Schema namespace, with its
    /// colon (`xs:`), for the `type` of a new declaration.
    fn xsd_prefix(&self, schema_sheet: &str) -> String {
        let declarations = self
            .sheets
            .get(schema_sheet)
            .and_then(|s| s.xml.as_ref())
            .map(|d| d.declarations.clone())
            .unwrap_or_else(|| xsd::default_document().declarations);
        match declarations.iter().find(|(_, uri)| uri == XSD_NS) {
            Some((Some(prefix), _)) => format!("{prefix}:"),
            _ => String::new(),
        }
    }

    /// The value of the record's `complexType` with `change` made to it.
    pub(super) fn xsd_edited_type(
        &self,
        places: &FieldPlaces,
        change: &XsdChange<'_>,
    ) -> Result<Value, EditError> {
        let node = self
            .sheets
            .get(&places.schema_sheet)
            .and_then(|d| d.data.get(&places.document_path))
            .ok_or_else(|| EditError::Path("the type is not in the schema any more".to_owned()))?;
        let mut complex = object_of(node)?;
        let simple_content = complex.contains_key("simpleContent");
        // Where the attributes are, and their list.
        let mut extension = match complex.get("simpleContent") {
            Some(sc) => {
                let sc = object_of(sc)?;
                Some(object_of(sc.get("extension").ok_or_else(|| {
                    EditError::Path("the simpleContent has no extension".to_owned())
                })?)?)
            }
            None => None,
        };
        let mut attributes = match &extension {
            Some(ext) => items_at(ext, "attribute"),
            None => items_at(&complex, "attribute"),
        };
        let mut sequence = match complex.get("sequence") {
            Some(seq) => object_of(seq)?,
            None => IndexMap::new(),
        };
        let mut elements = items_at(&sequence, "element");
        let find = |list: &[Node], name: &str| {
            list.iter()
                .position(|item| declared_name(item) == Some(name))
                .ok_or_else(|| {
                    EditError::Path(format!("cannot find {name} in the schema's complexType"))
                })
        };

        match change {
            XsdChange::Add {
                name,
                kind,
                required,
            } => {
                let ty = format!(
                    "{}{}",
                    self.xsd_prefix(&places.schema_sheet),
                    kind.xsd_type()
                );
                if let Some(attribute) = name.strip_prefix('@') {
                    check_xml_name(attribute)?;
                    if kind.is_structured() {
                        return Err(EditError::Path(format!(
                            "an attribute holds one value, so it cannot be an {}; add an                              element instead",
                            kind.label()
                        )));
                    }
                    let mut fields = vec![("@name", attribute.to_owned()), ("@type", ty)];
                    if *required {
                        fields.push(("@use", "required".to_owned()));
                    }
                    attributes.push(declaration(&fields));
                } else {
                    check_xml_name(name)?;
                    if simple_content {
                        return Err(EditError::Path(
                            "this element holds text, so it cannot have child elements; add an \
                             attribute instead"
                                .to_owned(),
                        ));
                    }
                    if kind.is_structured() {
                        // An element with a complexType of its own, with no
                        // fields yet; an array is one that repeats.
                        let mut fields = vec![("@name", (*name).to_owned())];
                        if !required {
                            fields.push(("@minOccurs", "0".to_owned()));
                        }
                        if *kind == FieldKind::Array {
                            fields.push(("@maxOccurs", "unbounded".to_owned()));
                        }
                        let mut element = declaration(&fields);
                        if let Value::Object(map) = element.value_mut() {
                            map.insert(
                                "complexType".to_owned(),
                                Node::new(Value::Object(IndexMap::new())),
                            );
                        }
                        elements.push(element);
                    } else {
                        let mut fields = vec![("@name", (*name).to_owned()), ("@type", ty)];
                        if !required {
                            fields.push(("@minOccurs", "0".to_owned()));
                        }
                        elements.push(declaration(&fields));
                    }
                }
            }
            XsdChange::Rename { from, to } => {
                if *from == "#text" {
                    return Err(EditError::Path(
                        "the text of an element has no name to change".to_owned(),
                    ));
                }
                match (from.strip_prefix('@'), to.strip_prefix('@')) {
                    (Some(old), Some(new)) => {
                        check_xml_name(new)?;
                        let i = find(&attributes, old)?;
                        set_name(&mut attributes[i], new);
                    }
                    (None, None) => {
                        check_xml_name(to)?;
                        let i = find(&elements, from)?;
                        if attribute_value(&elements[i], "@ref").is_some() {
                            return Err(EditError::Path(format!(
                                "{from} refers to the global element, so it takes that \
                                 element's name; rename it on {}",
                                places.schema_sheet
                            )));
                        }
                        set_name(&mut elements[i], to);
                    }
                    _ => {
                        return Err(EditError::Path(
                            "an attribute stays an attribute and an element an element: an \
                             attribute's name starts with @"
                                .to_owned(),
                        ));
                    }
                }
            }
            XsdChange::Delete { name } => {
                if *name == "#text" {
                    return Err(EditError::Path(format!(
                        "the text of an element is not a field that can be deleted; change \
                         its type on {}",
                        places.schema_sheet
                    )));
                }
                match name.strip_prefix('@') {
                    Some(attribute) => {
                        let i = find(&attributes, attribute)?;
                        attributes.remove(i);
                    }
                    None => {
                        let i = find(&elements, name)?;
                        elements.remove(i);
                    }
                }
            }
            XsdChange::Reorder { order } => {
                // Attributes stay among the attributes, the text where it is.
                let kind = |f: &str| {
                    if f.starts_with('@') {
                        0
                    } else if f == "#text" {
                        1
                    } else {
                        2
                    }
                };
                let kept = order.len() == places.fields.len()
                    && order
                        .iter()
                        .zip(&places.fields)
                        .all(|(new, old)| kind(new) == kind(old));
                if !kept {
                    return Err(EditError::Path(
                        "XSD keeps attributes apart from elements, so an attribute moves only \
                         among the attributes and an element among the elements"
                            .to_owned(),
                    ));
                }
                let mut reordered_attributes = Vec::new();
                let mut reordered_elements = Vec::new();
                for field in order.iter() {
                    match field.strip_prefix('@') {
                        Some(attribute) => {
                            reordered_attributes
                                .push(attributes[find(&attributes, attribute)?].clone());
                        }
                        None if field == "#text" => {}
                        None => reordered_elements.push(elements[find(&elements, field)?].clone()),
                    }
                }
                attributes = reordered_attributes;
                elements = reordered_elements;
            }
        }

        if !elements.is_empty() || complex.contains_key("sequence") {
            set_items(&mut sequence, "element", elements, SEQUENCE_ORDER);
            match complex.get_mut("sequence") {
                Some(node) => *node.value_mut() = Value::Object(sequence),
                None => insert_ordered(
                    &mut complex,
                    "sequence",
                    Node::new(Value::Object(sequence)),
                    COMPLEX_ORDER,
                ),
            }
        }
        match extension.as_mut() {
            Some(ext) => {
                set_items(ext, "attribute", attributes, EXTENSION_ORDER);
                let mut sc = object_of(&complex["simpleContent"])?;
                if let Some(node) = sc.get_mut("extension") {
                    *node.value_mut() = Value::Object(ext.clone());
                }
                if let Some(node) = complex.get_mut("simpleContent") {
                    *node.value_mut() = Value::Object(sc);
                }
            }
            None => set_items(&mut complex, "attribute", attributes, COMPLEX_ORDER),
        }
        Ok(Value::Object(complex))
    }
}

fn set_name(node: &mut Node, name: &str) {
    if let Value::Object(fields) = node.value_mut()
        && let Some(slot) = fields.get_mut("@name")
    {
        *slot.value_mut() = Value::Str(name.to_owned());
    }
}
