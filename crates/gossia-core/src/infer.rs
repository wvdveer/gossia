//! Inferring a schema from data, for importing a standalone file (DESIGN.md,
//! "Import and export", "Schema inference").
//!
//! What is inferred is always inside the v1 subset: closed objects, arrays and
//! simple types, with no unions and no nulls. It is a starting point that the
//! user edits on the schema tab afterwards, so where the data disagrees with
//! itself the most common shape wins and the rest is kept in the data, flagged
//! as invalid, never converted or dropped. Every such decision is a warning
//! for the import report.

use chrono::{DateTime, NaiveDate, NaiveDateTime};
use gossia_i18n::tr;
use indexmap::IndexMap;

use crate::value::{Node, Value};
use crate::xmltree::{self, Document, Element, XML_NS, XSI_NS};

/// A JSON Schema inferred from JSON data, and the data as it is imported.
#[derive(Debug, Clone)]
pub struct JsonInference {
    /// The schema document, as the schema tab holds it.
    pub schema: Node,
    /// The data without its nulls, which a schema of version 1 cannot type.
    /// A null is taken to mean that the value is absent, whatever schema the
    /// data is then given.
    pub data: Node,
    /// Where nulls were left out, for the report.
    pub dropped: Vec<String>,
    /// What the report says about the schema, in the order it was found.
    pub warnings: Vec<String>,
}

/// An XSD inferred from an XML document, as XSD text.
#[derive(Debug, Clone)]
pub struct XsdInference {
    pub text: String,
    pub warnings: Vec<String>,
}

// ------------------------------------------------------------------ JSON

/// Infers a JSON Schema from data read from a JSON file.
pub fn infer_json(original: &Node) -> JsonInference {
    let mut nulls: IndexMap<(String, bool), usize> = IndexMap::new();
    let data = without_nulls(original, "", &mut nulls);
    let dropped: Vec<String> = nulls
        .into_iter()
        .map(|((path, item), n)| {
            let place = place(&path);
            match (item, n) {
                (true, 1) => tr!(
                    "{place}: 1 null item left out, since version 1 has no null.",
                    place = place
                ),
                (true, n) => tr!(
                    "{place}: {n} null items left out, since version 1 has no null.",
                    place = place,
                    n = n
                ),
                (false, 1) => tr!(
                    "{place}: 1 null left out, since version 1 has no null, so it is optional.",
                    place = place
                ),
                (false, n) => tr!(
                    "{place}: {n} nulls left out, since version 1 has no null, so it is optional.",
                    place = place,
                    n = n
                ),
            }
        })
        .collect();
    // The data with its nulls, so that a field that is only ever null is in
    // the schema all the same, as an optional field.
    let mut shape = Shape::default();
    shape.observe(original);
    let mut warnings = Vec::new();
    let schema = shape.schema("", &mut warnings);
    JsonInference {
        schema,
        data,
        dropped,
        warnings,
    }
}

/// Plain text as data: an array of its lines (DESIGN.md, "Import and
/// export"). Line endings are normalised, a final newline adds no empty
/// line, and blank lines in the middle are kept as empty strings.
pub fn text_lines(text: &str) -> JsonInference {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let body = text.strip_suffix('\n').unwrap_or(&text);
    let lines: Vec<Node> = if text.is_empty() {
        Vec::new()
    } else {
        body.split('\n')
            .map(|line| Node::new(Value::Str(line.to_owned())))
            .collect()
    };
    let schema = object(vec![
        ("type", text_node("array")),
        ("items", object(vec![("type", text_node("string"))])),
    ]);
    JsonInference {
        schema,
        data: Node::new(Value::Array(lines)),
        dropped: Vec::new(),
        warnings: Vec::new(),
    }
}

/// The data with its null fields and null items taken out, counting them by
/// where they were (`true` for items).
fn without_nulls(node: &Node, path: &str, nulls: &mut IndexMap<(String, bool), usize>) -> Node {
    let value = match node.value() {
        Value::Array(items) => {
            let item_path = format!("{path}[]");
            let mut kept = Vec::with_capacity(items.len());
            for item in items {
                if matches!(item.value(), Value::Null) {
                    *nulls.entry((path.to_owned(), true)).or_default() += 1;
                } else {
                    kept.push(without_nulls(item, &item_path, nulls));
                }
            }
            Value::Array(kept)
        }
        Value::Object(fields) => {
            let mut kept = IndexMap::with_capacity(fields.len());
            for (name, child) in fields {
                let child_path = field_path(path, name);
                if matches!(child.value(), Value::Null) {
                    *nulls.entry((child_path, false)).or_default() += 1;
                } else {
                    kept.insert(name.clone(), without_nulls(child, &child_path, nulls));
                }
            }
            Value::Object(kept)
        }
        _ => return node.clone(),
    };
    Node::new(value).with_comments_of(node)
}

/// The kinds of value a position can hold, in the order that settles a tie:
/// the more general first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JsonKind {
    Text,
    Number,
    Integer,
    Boolean,
    Object,
    Array,
}

impl JsonKind {
    fn label(self) -> &'static str {
        match self {
            JsonKind::Text => tr!("text"),
            JsonKind::Number => tr!("number"),
            JsonKind::Integer => tr!("whole number"),
            JsonKind::Boolean => tr!("yes or no"),
            JsonKind::Object => tr!("object"),
            JsonKind::Array => tr!("array"),
        }
    }
}

/// Everything seen at one position of the data, across every instance.
#[derive(Debug, Default)]
struct Shape {
    strings: usize,
    ints: usize,
    floats: usize,
    bools: usize,
    objects: usize,
    arrays: usize,
    /// The fields of the objects, in the order they were first seen, with the
    /// number of objects that had each.
    fields: IndexMap<String, (usize, Shape)>,
    /// The items of all the arrays together.
    items: Option<Box<Shape>>,
}

impl Shape {
    fn observe(&mut self, node: &Node) {
        match node.value() {
            Value::Null => {}
            Value::Bool(_) => self.bools += 1,
            Value::Int(_) => self.ints += 1,
            Value::Float(_) => self.floats += 1,
            Value::Str(_) | Value::Date(_) | Value::DateTime(_) | Value::Error(_) => {
                self.strings += 1;
            }
            Value::Array(items) => {
                self.arrays += 1;
                let shape = self.items.get_or_insert_with(Box::default);
                for item in items {
                    shape.observe(item);
                }
            }
            Value::Object(fields) => {
                self.objects += 1;
                for (name, child) in fields {
                    let entry = self.fields.entry(name.clone()).or_default();
                    if !matches!(child.value(), Value::Null) {
                        entry.0 += 1;
                        entry.1.observe(child);
                    }
                }
            }
        }
    }

    /// How many values of each kind, leaving out the kinds not seen. A mix of
    /// whole numbers and others is all numbers.
    fn counts(&self) -> Vec<(JsonKind, usize)> {
        let number = if self.floats > 0 {
            (JsonKind::Number, self.ints + self.floats)
        } else {
            (JsonKind::Integer, self.ints)
        };
        [
            (JsonKind::Text, self.strings),
            number,
            (JsonKind::Boolean, self.bools),
            (JsonKind::Object, self.objects),
            (JsonKind::Array, self.arrays),
        ]
        .into_iter()
        .filter(|(_, n)| *n > 0)
        .collect()
    }

    fn schema(&self, path: &str, warnings: &mut Vec<String>) -> Node {
        let counts = self.counts();
        // The most common kind; on a tie, the first in the order above.
        let Some(kind) = counts
            .iter()
            .fold(None::<(JsonKind, usize)>, |best, &(kind, n)| match best {
                Some((_, m)) if m >= n => best,
                _ => Some((kind, n)),
            })
            .map(|(kind, _)| kind)
        else {
            warnings.push(tr!(
                "{place}: inferred from no data, so it is text.",
                place = place(path)
            ));
            return object(vec![("type", text_node("string"))]);
        };
        if counts.len() > 1 {
            let mut by_count = counts.clone();
            by_count.sort_by_key(|c| std::cmp::Reverse(c.1));
            let seen: Vec<String> = by_count
                .iter()
                .map(|(k, n)| tr!("{n} {kind}", n = n, kind = k.label()))
                .collect();
            warnings.push(tr!(
                "{place}: the values are of different kinds ({seen}); the schema takes the most \
                 common, {kind}, and the others are kept and flagged as invalid.",
                place = place(path),
                seen = seen.join(", "),
                kind = kind.label()
            ));
        }
        match kind {
            JsonKind::Text => object(vec![("type", text_node("string"))]),
            JsonKind::Number => object(vec![("type", text_node("number"))]),
            JsonKind::Integer => object(vec![("type", text_node("integer"))]),
            JsonKind::Boolean => object(vec![("type", text_node("boolean"))]),
            JsonKind::Array => {
                let item_path = format!("{path}[]");
                let items = match &self.items {
                    Some(shape) if !shape.counts().is_empty() => shape.schema(&item_path, warnings),
                    _ => {
                        warnings.push(tr!(
                            "{place}: inferred from no data (the arrays are empty), so the items \
                             are text.",
                            place = place(&item_path)
                        ));
                        object(vec![("type", text_node("string"))])
                    }
                };
                object(vec![("type", text_node("array")), ("items", items)])
            }
            JsonKind::Object => {
                let mut properties = IndexMap::new();
                let mut required = Vec::new();
                for (name, (present, shape)) in &self.fields {
                    properties.insert(
                        name.clone(),
                        shape.schema(&field_path(path, name), warnings),
                    );
                    if *present == self.objects {
                        required.push(text_node(name));
                    }
                }
                let mut fields = vec![
                    ("type", text_node("object")),
                    ("properties", Node::new(Value::Object(properties))),
                ];
                if !required.is_empty() {
                    fields.push(("required", Node::new(Value::Array(required))));
                }
                object(fields)
            }
        }
    }
}

fn field_path(path: &str, name: &str) -> String {
    if path.is_empty() {
        name.to_owned()
    } else {
        format!("{path}.{name}")
    }
}

/// Where a warning is about, as the report says it.
fn place(path: &str) -> String {
    if path.is_empty() {
        tr!("The root").to_owned()
    } else {
        path.to_owned()
    }
}

fn text_node(text: &str) -> Node {
    Node::new(Value::Str(text.to_owned()))
}

fn object(fields: Vec<(&str, Node)>) -> Node {
    Node::new(Value::Object(
        fields.into_iter().map(|(k, v)| (k.to_owned(), v)).collect(),
    ))
}

// ------------------------------------------------------------------- XML

/// The XSD of one namespace of a document, inferred: what one XSD tab holds.
#[derive(Debug, Clone)]
pub struct XsdTab {
    /// The namespace, or none for the elements in no namespace.
    pub namespace: Option<String>,
    /// The prefix the document first used for it (none for the main tab, or
    /// for a namespace only ever the default one).
    pub prefix: Option<String>,
    pub text: String,
}

/// The XSDs inferred from a document, one per namespace: the first is the
/// main one, whose global element is the root, and it imports the others,
/// which may import each other.
#[derive(Debug, Clone)]
pub struct XsdInferences {
    pub tabs: Vec<XsdTab>,
    pub warnings: Vec<String>,
}

/// Infers an XSD from an XML document that needs only one: elements in one
/// namespace, with the built-in `xml:` and `xsi:` attributes at most (which
/// the schema names with `ref`). A document that needs more is refused with
/// a message; [`infer_xsds`] infers it.
pub fn infer_xsd(doc: &Document) -> Result<XsdInference, String> {
    let inferred = infer_xsds(doc, &|_, _| String::new())?;
    let mut tabs = inferred.tabs;
    if tabs.len() > 1 {
        return Err(tr!(
            "the document uses more than one namespace, which version 1 does not support"
        )
        .to_owned());
    }
    Ok(XsdInference {
        text: tabs.remove(0).text,
        warnings: inferred.warnings,
    })
}

/// Infers the XSDs of an XML document, one for each namespace it uses (the
/// elements in no namespace count as one). An element of another namespace
/// than its parent's is a global element of its own namespace's schema,
/// which the parent's refers to with `ref`; so is an attribute of another
/// namespace. `location` gives the `schemaLocation` by which one schema
/// imports another, from that one's prefix and namespace. Mixed content is
/// inferred as a mixed type, whose inside is kept as markup, and children in
/// a varying order as a repeating choice, the ordered list `#children`.
pub fn infer_xsds(
    doc: &Document,
    location: &dyn Fn(Option<&str>, Option<&str>) -> String,
) -> Result<XsdInferences, String> {
    let mut registry = Registry::default();
    let root = &doc.root;
    registry.space(root.namespace.as_deref(), root.prefix.as_deref());
    registry.observe_global(root, &root.name)?;
    registry.unique_prefixes();
    let mut warnings = doc.warnings.clone();
    let mut tabs = Vec::new();
    for (namespace, space) in &registry.spaces {
        let text = registry.write(namespace.as_deref(), space, location, &mut warnings);
        tabs.push(XsdTab {
            namespace: namespace.clone(),
            prefix: space.prefix.clone(),
            text,
        });
    }
    Ok(XsdInferences { tabs, warnings })
}

/// Whether an attribute is one of the built-in `xml:` and `xsi:` ones.
fn is_builtin(attribute: &xmltree::Attribute) -> bool {
    matches!(attribute.namespace.as_deref(), Some(XML_NS | XSI_NS))
}

/// Everything seen of one namespace: what its schema declares globally.
#[derive(Debug, Default)]
struct Space {
    /// The prefix its schema and the others name it by.
    prefix: Option<String>,
    /// Whether the local elements of its elements are in it (qualified) or
    /// in none, once seen.
    qualified: Option<bool>,
    /// The global elements, by local name, in the order first seen.
    elements: IndexMap<String, ElementShape>,
    /// The global attributes, by local name.
    attributes: IndexMap<String, TextShape>,
    /// The other namespaces its declarations refer to, in the order first
    /// seen.
    refers: Vec<String>,
    /// The built-in attributes its elements have, by prefix and namespace.
    builtin: Vec<(String, String)>,
    /// How many attributes of its elements are in it, and how many in none.
    attribute_forms: (usize, usize),
}

#[derive(Debug, Default)]
struct Registry {
    /// The namespaces, the root's first.
    spaces: IndexMap<Option<String>, Space>,
}

impl Registry {
    /// The space of a namespace, made when first seen with `prefix`.
    fn space(&mut self, namespace: Option<&str>, prefix: Option<&str>) -> &mut Space {
        let space = self.spaces.entry(namespace.map(str::to_owned)).or_default();
        if space.prefix.is_none() {
            space.prefix = prefix.map(str::to_owned);
        }
        space
    }

    /// Observes an element that is a global element of its namespace.
    fn observe_global(&mut self, element: &Element, path: &str) -> Result<(), String> {
        let namespace = element.namespace.clone();
        let space = self.space(namespace.as_deref(), element.prefix.as_deref());
        // Taken out while it is observed, since what is inside it may add to
        // the registry.
        let mut shape = space
            .elements
            .shift_remove(&element.name)
            .unwrap_or_default();
        let result = shape.observe(element, path, namespace.as_deref(), self);
        let space = self.space(namespace.as_deref(), None);
        match space.elements.get_mut(&element.name) {
            // The same element met again inside itself: kept as first seen.
            Some(_) => {}
            None => {
                space.elements.insert(element.name.clone(), shape);
            }
        }
        result
    }

    /// The prefix by which the schemas name a namespace.
    fn prefix_of(&self, namespace: &str) -> String {
        self.spaces
            .get(&Some(namespace.to_owned()))
            .and_then(|s| s.prefix.clone())
            .unwrap_or_else(|| "ns".to_owned())
    }

    /// Gives every namespace but the main one a prefix of its own: the one
    /// the document used, unless another namespace has it already, or
    /// `ns1`, `ns2`, ... for one that only ever was the default namespace.
    fn unique_prefixes(&mut self) {
        let mut taken: Vec<String> = vec!["xs".to_owned(), "xml".to_owned()];
        let mut n = 0;
        for (i, space) in self.spaces.values_mut().enumerate() {
            for (prefix, _) in &space.builtin {
                if !taken.contains(prefix) {
                    taken.push(prefix.clone());
                }
            }
            if i == 0 && space.prefix.is_none() {
                continue;
            }
            let wanted = space.prefix.clone().filter(|p| !taken.contains(p));
            let prefix = wanted.unwrap_or_else(|| {
                loop {
                    n += 1;
                    let candidate = format!("ns{n}");
                    if !taken.contains(&candidate) {
                        break candidate;
                    }
                }
            });
            taken.push(prefix.clone());
            space.prefix = Some(prefix);
        }
    }

    /// The schema of one namespace, as XSD text.
    fn write(
        &self,
        namespace: Option<&str>,
        space: &Space,
        location: &dyn Fn(Option<&str>, Option<&str>) -> String,
        warnings: &mut Vec<String>,
    ) -> String {
        let (qualified_attributes, plain_attributes) = space.attribute_forms;
        let attributes_qualified = qualified_attributes > 0 && plain_attributes == 0;
        let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        out.push_str("<xs:schema xmlns:xs=\"http://www.w3.org/2001/XMLSchema\"");
        let mut declared: Vec<(String, String)> = Vec::new();
        for (prefix, uri) in space.builtin.iter().filter(|(p, _)| p != "xml") {
            declared.push((prefix.clone(), uri.clone()));
        }
        for uri in &space.refers {
            declared.push((self.prefix_of(uri), uri.clone()));
        }
        for (prefix, uri) in &declared {
            out.push_str(&format!(
                " xmlns:{prefix}=\"{}\"",
                xmltree::escape_attribute(uri)
            ));
        }
        if let Some(ns) = namespace {
            out.push_str(&format!(
                " targetNamespace=\"{}\" elementFormDefault=\"{}\"",
                xmltree::escape_attribute(ns),
                if space.qualified.unwrap_or(true) {
                    "qualified"
                } else {
                    "unqualified"
                }
            ));
            if attributes_qualified {
                out.push_str(" attributeFormDefault=\"qualified\"");
            }
        }
        out.push_str(">\n");
        if space.builtin.iter().any(|(_, uri)| uri == XML_NS) {
            out.push_str(&format!(
                "  <xs:import namespace=\"{XML_NS}\" schemaLocation=\"http://www.w3.org/2001/xml.xsd\"/>\n"
            ));
        }
        for uri in &space.refers {
            let prefix = self
                .spaces
                .get(&Some(uri.clone()))
                .and_then(|s| s.prefix.as_deref());
            out.push_str(&format!(
                "  <xs:import namespace=\"{}\" schemaLocation=\"{}\"/>\n",
                xmltree::escape_attribute(uri),
                xmltree::escape_attribute(&location(prefix, Some(uri)))
            ));
        }
        let main = self
            .spaces
            .first()
            .is_some_and(|(ns, _)| ns.as_deref() == namespace);
        for (name, shape) in &space.elements {
            let path = match &space.prefix {
                Some(p) if !main => format!("{p}:{name}"),
                _ => name.clone(),
            };
            shape.write(
                name,
                &path,
                "",
                1,
                attributes_qualified,
                self,
                &mut out,
                warnings,
            );
        }
        for (name, text) in &space.attributes {
            out.push_str(&format!(
                "  <xs:attribute name=\"{}\" type=\"xs:{}\"/>\n",
                xmltree::escape_attribute(name),
                text.xsd_type()
            ));
        }
        out.push_str("</xs:schema>\n");
        out
    }
}

/// What the text of an element or an attribute could be typed as, from all
/// its values: each flag stays on while every value fits.
#[derive(Debug, Clone, Copy)]
struct TextShape {
    count: usize,
    integer: bool,
    decimal: bool,
    boolean: bool,
    date: bool,
    date_time: bool,
    /// How many values were not empty.
    filled: usize,
}

impl Default for TextShape {
    fn default() -> TextShape {
        TextShape {
            count: 0,
            integer: true,
            decimal: true,
            boolean: true,
            date: true,
            date_time: true,
            filled: 0,
        }
    }
}

impl TextShape {
    fn observe(&mut self, text: &str) {
        let t = text.trim();
        self.count += 1;
        if !t.is_empty() {
            self.filled += 1;
        }
        let integer = is_integer(t);
        self.integer &= integer;
        self.decimal &= integer || is_decimal(t);
        self.boolean &= t == "true" || t == "false";
        self.date &= t.len() == 10 && NaiveDate::parse_from_str(t, "%Y-%m-%d").is_ok();
        self.date_time &= t.contains('T')
            && (DateTime::parse_from_rfc3339(t).is_ok()
                || NaiveDateTime::parse_from_str(t, "%Y-%m-%dT%H:%M:%S%.f").is_ok());
    }

    /// The XSD type, without its prefix. Text with no values is a string.
    fn xsd_type(&self) -> &'static str {
        if self.count == 0 {
            "string"
        } else if self.integer {
            "integer"
        } else if self.decimal {
            "decimal"
        } else if self.boolean {
            "boolean"
        } else if self.date {
            "date"
        } else if self.date_time {
            "dateTime"
        } else {
            "string"
        }
    }
}

/// Digits with an optional minus, and no leading zero unless it is `0`, so
/// that identifiers like `007` stay text; and small enough to be a whole
/// number.
fn is_integer(t: &str) -> bool {
    let digits = t.strip_prefix('-').unwrap_or(t);
    plain_digits(digits) && t.parse::<i64>().is_ok()
}

fn is_decimal(t: &str) -> bool {
    let unsigned = t.strip_prefix('-').unwrap_or(t);
    match unsigned.split_once('.') {
        Some((whole, fraction)) => {
            plain_digits(whole)
                && !fraction.is_empty()
                && fraction.bytes().all(|b| b.is_ascii_digit())
        }
        None => false,
    }
}

fn plain_digits(digits: &str) -> bool {
    !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit())
        && (digits == "0" || !digits.starts_with('0'))
}

/// Everything seen of the elements at one position of the document.
#[derive(Debug, Default)]
struct ElementShape {
    instances: usize,
    /// The attributes, in the order merged from every instance (as the
    /// children are). The key of one of another namespace is `{uri}name`, of
    /// a built-in one `prefix:name`.
    attributes: IndexMap<String, AttributeShape>,
    /// The child elements, in the order merged from every instance; the key
    /// of one of another namespace, a global element there, is `{uri}name`.
    children: IndexMap<String, ChildShape>,
    /// The instances that held child elements.
    with_elements: usize,
    /// The text of the instances that held no child elements.
    text: TextShape,
    /// The instances disagreed about the order of the children.
    order_differs: bool,
    /// An instance held text next to child elements: the position is mixed
    /// content, kept as markup, and what is inside it is not looked at.
    mixed: bool,
}

#[derive(Debug, Default)]
struct AttributeShape {
    /// How many instances of the element had it.
    present: usize,
    text: TextShape,
    /// It is in the namespace of its element's schema (it has a prefix).
    qualified: bool,
}

#[derive(Debug, Default)]
struct ChildShape {
    /// How many instances of the parent had this child.
    present: usize,
    /// The most times it appeared in one instance of the parent.
    most: usize,
    shape: ElementShape,
}

/// For the key of something in another namespace, `{uri}name`, the name a
/// schema refers to it by: `prefix:name`.
fn reference(registry: &Registry, key: &str) -> Option<String> {
    let (uri, local) = key.strip_prefix('{')?.split_once('}')?;
    Some(format!("{}:{local}", registry.prefix_of(uri)))
}

impl ElementShape {
    /// Observes an instance whose declaration is in the schema of `tab`.
    fn observe(
        &mut self,
        element: &Element,
        path: &str,
        tab: Option<&str>,
        registry: &mut Registry,
    ) -> Result<(), String> {
        self.instances += 1;
        // A new attribute goes after the one before it in this instance, so
        // that writing keeps the order of documents whose instances leave out
        // different attributes.
        let mut after: Option<usize> = None;
        for attribute in &element.attributes {
            let mine = attribute.namespace.is_some() && attribute.namespace.as_deref() == tab;
            let key = match (&attribute.prefix, attribute.namespace.as_deref()) {
                (Some(prefix), Some(uri)) if is_builtin(attribute) => {
                    let space = registry.space(tab, None);
                    if !space.builtin.iter().any(|(_, u)| u == uri) {
                        space.builtin.push((prefix.clone(), uri.to_owned()));
                    }
                    format!("{prefix}:{}", attribute.name)
                }
                (_, Some(uri)) if !mine => {
                    // A global attribute of its own namespace.
                    registry
                        .space(Some(uri), attribute.prefix.as_deref())
                        .attributes
                        .entry(attribute.name.clone())
                        .or_default()
                        .observe(&attribute.value);
                    let space = registry.space(tab, None);
                    if !space.refers.iter().any(|u| u == uri) {
                        space.refers.push(uri.to_owned());
                    }
                    format!("{{{uri}}}{}", attribute.name)
                }
                _ => {
                    let forms = &mut registry.space(tab, None).attribute_forms;
                    if mine {
                        forms.0 += 1;
                    } else {
                        forms.1 += 1;
                    }
                    attribute.name.clone()
                }
            };
            let at = match self.attributes.get_index_of(&key) {
                Some(i) => i,
                None => {
                    let at = after.map_or(0, |a| a + 1);
                    self.attributes
                        .shift_insert(at, key.clone(), Default::default());
                    at
                }
            };
            after = Some(after.map_or(at, |a| a.max(at)));
            let entry = &mut self.attributes[at];
            if entry.present > 0 && entry.qualified != mine {
                return Err(tr!(
                    "<{path}> has the attribute {name} both with and without a prefix, \
                     which version 1 does not support",
                    path = path,
                    name = key
                ));
            }
            entry.present += 1;
            entry.qualified = mine;
            entry.text.observe(&attribute.value);
        }
        if element.elements().next().is_none() {
            self.text.observe(&element.text());
        } else {
            self.mixed |= element.has_text();
            self.with_elements += 1;
        }
        if self.mixed {
            return Ok(());
        }
        let keys = element
            .elements()
            .map(|child| child_key(child, tab, path, registry))
            .collect::<Result<Vec<_>, String>>()?;
        let mut here: IndexMap<&str, usize> = IndexMap::new();
        let mut last: Option<&str> = None;
        for (key, _) in &keys {
            if last != Some(key.as_str()) && here.contains_key(key.as_str()) {
                // The elements of one name are apart: they are written together.
                self.order_differs = true;
            }
            *here.entry(key).or_default() += 1;
            last = Some(key);
        }
        // Merges the order of this instance into the order so far: a new name
        // goes after the name before it.
        let mut after: Option<usize> = None;
        for (&name, &count) in &here {
            let at = match self.children.get_index_of(name) {
                Some(i) => {
                    if after.is_some_and(|a| i < a) {
                        self.order_differs = true;
                    }
                    i
                }
                None => {
                    let at = after.map_or(0, |a| a + 1);
                    self.children
                        .shift_insert(at, name.to_owned(), ChildShape::default());
                    at
                }
            };
            after = Some(after.map_or(at, |a| a.max(at)));
            let child = &mut self.children[at];
            child.present += 1;
            child.most = child.most.max(count);
        }
        for (child, (key, global)) in element.elements().zip(&keys) {
            let child_path = format!("{path}/{}", child.name);
            if *global {
                registry.observe_global(child, &child_path)?;
            } else {
                self.children
                    .get_mut(key.as_str())
                    .expect("every child was counted")
                    .shape
                    .observe(child, &child_path, tab, registry)?;
            }
        }
        Ok(())
    }

    /// Writes the declaration of the element, with `occurs` already made.
    /// `qualified` says whether the schema qualifies attributes by default.
    #[allow(clippy::too_many_arguments)]
    fn write(
        &self,
        name: &str,
        path: &str,
        occurs: &str,
        depth: usize,
        qualified: bool,
        registry: &Registry,
        out: &mut String,
        warnings: &mut Vec<String>,
    ) {
        let pad = "  ".repeat(depth);
        let name_attr = xmltree::escape_attribute(name);
        if self.mixed {
            warnings.push(tr!(
                "<{path}> holds text next to elements (mixed content); what is inside it is \
                 kept as XML, which is edited in the Raw view.",
                path = path
            ));
            out.push_str(&format!(
                "{pad}<xs:element name=\"{name_attr}\"{occurs}>\n\
                 {pad}  <xs:complexType mixed=\"true\">\n\
                 {pad}    <xs:sequence>\n\
                 {pad}      <xs:any processContents=\"skip\" minOccurs=\"0\" maxOccurs=\"unbounded\"/>\n\
                 {pad}    </xs:sequence>\n"
            ));
            self.write_attributes(&format!("{pad}    "), qualified, registry, out);
            out.push_str(&format!("{pad}  </xs:complexType>\n{pad}</xs:element>\n"));
            return;
        }
        if self.with_elements == 0 && self.attributes.is_empty() {
            out.push_str(&format!(
                "{pad}<xs:element name=\"{name_attr}\" type=\"xs:{}\"{occurs}/>\n",
                self.text.xsd_type()
            ));
            return;
        }
        out.push_str(&format!(
            "{pad}<xs:element name=\"{name_attr}\"{occurs}>\n{pad}  <xs:complexType>\n"
        ));
        let inner = format!("{pad}    ");
        if self.with_elements > 0 {
            if self.text.filled > 0 {
                warnings.push(tr!(
                    "<{path}>: some elements hold text and others hold elements; the elements \
                     are the schema, and the text is kept and flagged.",
                    path = path
                ));
            }
            // Children in a varying order are one ordered list, of which
            // each item is one of them.
            let group = if self.order_differs {
                warnings.push(tr!(
                    "<{path}>: the child elements are not always in the same order, so they \
                     are kept in their order as one list, #children.",
                    path = path
                ));
                out.push_str(&format!(
                    "{inner}<xs:choice minOccurs=\"0\" maxOccurs=\"unbounded\">\n"
                ));
                "choice"
            } else {
                out.push_str(&format!("{inner}<xs:sequence>\n"));
                "sequence"
            };
            for (child_name, child) in &self.children {
                let mut occurs = String::new();
                if child.present < self.instances && !self.order_differs {
                    occurs.push_str(" minOccurs=\"0\"");
                }
                if child.most > 1 && !self.order_differs {
                    occurs.push_str(" maxOccurs=\"unbounded\"");
                }
                if let Some(name) = reference(registry, child_name) {
                    // A global element of another namespace.
                    out.push_str(&format!(
                        "{inner}  <xs:element ref=\"{}\"{occurs}/>\n",
                        xmltree::escape_attribute(&name)
                    ));
                    continue;
                }
                child.shape.write(
                    child_name,
                    &format!("{path}/{child_name}"),
                    &occurs,
                    depth + 3,
                    qualified,
                    registry,
                    out,
                    warnings,
                );
            }
            out.push_str(&format!("{inner}</xs:{group}>\n"));
            self.write_attributes(&inner, qualified, registry, out);
        } else if self.text.filled > 0 {
            out.push_str(&format!(
                "{inner}<xs:simpleContent>\n{inner}  <xs:extension base=\"xs:{}\">\n",
                self.text.xsd_type()
            ));
            self.write_attributes(&format!("{inner}    "), qualified, registry, out);
            out.push_str(&format!(
                "{inner}  </xs:extension>\n{inner}</xs:simpleContent>\n"
            ));
        } else {
            self.write_attributes(&inner, qualified, registry, out);
        }
        out.push_str(&format!("{pad}  </xs:complexType>\n{pad}</xs:element>\n"));
    }

    fn write_attributes(&self, pad: &str, qualified: bool, registry: &Registry, out: &mut String) {
        for (name, a) in &self.attributes {
            let required = if a.present == self.instances {
                " use=\"required\""
            } else {
                ""
            };
            // A built-in attribute, or a global one of another namespace:
            // the schema refers to it.
            let referred =
                reference(registry, name).or_else(|| name.contains(':').then(|| name.clone()));
            if let Some(name) = referred {
                out.push_str(&format!(
                    "{pad}<xs:attribute ref=\"{}\"{required}/>\n",
                    xmltree::escape_attribute(&name)
                ));
                continue;
            }
            let form = if a.qualified && !qualified {
                " form=\"qualified\""
            } else {
                ""
            };
            out.push_str(&format!(
                "{pad}<xs:attribute name=\"{}\" type=\"xs:{}\"{required}{form}/>\n",
                xmltree::escape_attribute(name),
                a.text.xsd_type()
            ));
        }
    }
}

/// The key of a child element below an element of `tab`'s schema, and
/// whether it is a global element of another namespace. A child in the same
/// namespace, or in none below a namespace, is local; the schema's local
/// elements are all in its namespace (qualified) or all in none.
fn child_key(
    child: &Element,
    tab: Option<&str>,
    path: &str,
    registry: &mut Registry,
) -> Result<(String, bool), String> {
    let namespace = child.namespace.as_deref();
    if namespace.is_some() && namespace != tab {
        let uri = namespace.unwrap_or_default();
        registry.space(namespace, child.prefix.as_deref());
        let space = registry.space(tab, None);
        if !space.refers.iter().any(|u| u == uri) {
            space.refers.push(uri.to_owned());
        }
        return Ok((format!("{{{uri}}}{}", child.name), true));
    }
    if tab.is_some() {
        let qualified = namespace.is_some();
        let space = registry.space(tab, None);
        if *space.qualified.get_or_insert(qualified) != qualified {
            return Err(tr!(
                "the elements inside <{path}> are partly in its namespace and partly in none, \
                 which version 1 does not support",
                path = path
            ));
        }
    }
    Ok((child.name.clone(), false))
}
