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

/// Infers an XSD from an XML document. More than one namespace is outside the
/// subset, and refused with a message, except for the built-in `xml:` and
/// `xsi:` attributes, which the schema names with `ref`. Mixed content is
/// inferred as a mixed type, whose inside is kept as markup, and children
/// in a varying order as a repeating choice, the ordered list `#children`.
pub fn infer_xsd(doc: &Document) -> Result<XsdInference, String> {
    let namespace = doc.root.namespace.clone();
    let mut qualified = None;
    check_namespaces(&doc.root, namespace.as_deref(), true, &mut qualified)?;
    let mut shape = ElementShape::default();
    shape.observe(&doc.root, &doc.root.name)?;
    let mut warnings = doc.warnings.clone();
    // The prefixes of the built-in attributes, as the document writes them.
    let mut builtin: Vec<(String, String)> = Vec::new();
    builtin_prefixes(&doc.root, &mut builtin);
    // Attributes in the document's namespace (`w:id` in Word) are qualified:
    // by default when they all are, one by one otherwise.
    let (qualified_attributes, plain_attributes) = shape.attribute_forms();
    let attributes_qualified = qualified_attributes > 0 && plain_attributes == 0;
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str("<xs:schema xmlns:xs=\"http://www.w3.org/2001/XMLSchema\"");
    for (prefix, uri) in builtin.iter().filter(|(p, _)| p != "xml") {
        out.push_str(&format!(
            " xmlns:{prefix}=\"{}\"",
            xmltree::escape_attribute(uri)
        ));
    }
    if let Some(ns) = &namespace {
        out.push_str(&format!(
            " targetNamespace=\"{}\" elementFormDefault=\"{}\"",
            xmltree::escape_attribute(ns),
            if qualified.unwrap_or(true) {
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
    if builtin.iter().any(|(_, uri)| uri == XML_NS) {
        out.push_str(&format!(
            "  <xs:import namespace=\"{XML_NS}\" schemaLocation=\"http://www.w3.org/2001/xml.xsd\"/>\n"
        ));
    }
    shape.write(
        &doc.root.name,
        &doc.root.name,
        "",
        1,
        attributes_qualified,
        &mut out,
        &mut warnings,
    );
    out.push_str("</xs:schema>\n");
    Ok(XsdInference {
        text: out,
        warnings,
    })
}

/// Whether an attribute is one of the built-in `xml:` and `xsi:` ones.
fn is_builtin(attribute: &xmltree::Attribute) -> bool {
    matches!(attribute.namespace.as_deref(), Some(XML_NS | XSI_NS))
}

/// The prefixes the built-in attributes are written with, each namespace
/// once, in the order first seen.
fn builtin_prefixes(element: &Element, out: &mut Vec<(String, String)>) {
    for a in element.attributes.iter().filter(|a| is_builtin(a)) {
        let (Some(prefix), Some(uri)) = (&a.prefix, &a.namespace) else {
            continue;
        };
        if !out.iter().any(|(_, u)| u == uri) {
            out.push((prefix.clone(), uri.clone()));
        }
    }
    for child in element.elements() {
        builtin_prefixes(child, out);
    }
}

/// The key of an attribute in a shape: its name, with the prefix for a
/// built-in one.
fn attribute_key(attribute: &xmltree::Attribute) -> String {
    match &attribute.prefix {
        Some(p) if is_builtin(attribute) => format!("{p}:{}", attribute.name),
        _ => attribute.name.clone(),
    }
}

/// One namespace at most: the root's. The other elements are all in it
/// (`qualified`) or all in none (`unqualified`), and an attribute with a
/// prefix is in the root's namespace or is a built-in one.
fn check_namespaces(
    element: &Element,
    namespace: Option<&str>,
    root: bool,
    qualified: &mut Option<bool>,
) -> Result<(), String> {
    let more = || {
        tr!("the document uses more than one namespace, which version 1 does not support")
            .to_owned()
    };
    if !root {
        match (element.namespace.as_deref(), namespace) {
            (None, None) => {}
            (Some(ns), Some(root_ns)) if ns == root_ns => {
                if !*qualified.get_or_insert(true) {
                    return Err(more());
                }
            }
            (None, Some(_)) => {
                if *qualified.get_or_insert(false) {
                    return Err(more());
                }
            }
            _ => return Err(more()),
        }
    }
    if let Some(attribute) = element
        .attributes
        .iter()
        .find(|a| a.prefix.is_some() && !is_builtin(a) && a.namespace.as_deref() != namespace)
    {
        return Err(tr!(
            "the attribute {prefix}:{name} of <{name2}> is in a namespace, which version 1 does not support",
            prefix = attribute.prefix.as_deref().unwrap_or_default(),
            name = attribute.name,
            name2 = element.name
        ));
    }
    // What is inside mixed content is kept as markup, whatever it holds.
    if element.has_text() {
        return Ok(());
    }
    for child in element.elements() {
        check_namespaces(child, namespace, false, qualified)?;
    }
    Ok(())
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
    /// children are).
    attributes: IndexMap<String, AttributeShape>,
    /// The child elements, in the order merged from every instance.
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
    /// It is in the document's namespace (it has its prefix).
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

impl ElementShape {
    fn observe(&mut self, element: &Element, path: &str) -> Result<(), String> {
        self.instances += 1;
        // A new attribute goes after the one before it in this instance, so
        // that writing keeps the order of documents whose instances leave out
        // different attributes.
        let mut after: Option<usize> = None;
        for attribute in &element.attributes {
            let key = attribute_key(attribute);
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
            let qualified = attribute.namespace.is_some() && !is_builtin(attribute);
            let entry = &mut self.attributes[at];
            if entry.present > 0 && entry.qualified != qualified {
                return Err(tr!(
                    "<{path}> has the attribute {name} both with and without a prefix, \
                     which version 1 does not support",
                    path = path,
                    name = key
                ));
            }
            entry.present += 1;
            entry.qualified = qualified;
            entry.text.observe(&attribute.value);
        }
        let mut here: IndexMap<&str, usize> = IndexMap::new();
        let mut last_name: Option<&str> = None;
        for child in element.elements() {
            if last_name != Some(child.name.as_str()) && here.contains_key(child.name.as_str()) {
                // The elements of one name are apart: they are written together.
                self.order_differs = true;
            }
            *here.entry(&child.name).or_default() += 1;
            last_name = Some(&child.name);
        }
        if here.is_empty() {
            self.text.observe(&element.text());
        } else {
            self.mixed |= element.has_text();
            self.with_elements += 1;
        }
        if self.mixed {
            return Ok(());
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
        for child in element.elements() {
            let child_path = format!("{path}/{}", child.name);
            self.children
                .get_mut(child.name.as_str())
                .expect("every child was counted")
                .shape
                .observe(child, &child_path)?;
        }
        Ok(())
    }

    /// How many attributes, here and below, are in the document's namespace
    /// and how many in none (the built-in ones are neither).
    fn attribute_forms(&self) -> (usize, usize) {
        let mut counts = (0, 0);
        for (name, a) in &self.attributes {
            if a.qualified {
                counts.0 += 1;
            } else if !name.contains(':') {
                counts.1 += 1;
            }
        }
        if !self.mixed {
            for child in self.children.values() {
                let (q, p) = child.shape.attribute_forms();
                counts.0 += q;
                counts.1 += p;
            }
        }
        counts
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
            self.write_attributes(&format!("{pad}    "), qualified, out);
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
                child.shape.write(
                    child_name,
                    &format!("{path}/{child_name}"),
                    &occurs,
                    depth + 3,
                    qualified,
                    out,
                    warnings,
                );
            }
            out.push_str(&format!("{inner}</xs:{group}>\n"));
            self.write_attributes(&inner, qualified, out);
        } else if self.text.filled > 0 {
            out.push_str(&format!(
                "{inner}<xs:simpleContent>\n{inner}  <xs:extension base=\"xs:{}\">\n",
                self.text.xsd_type()
            ));
            self.write_attributes(&format!("{inner}    "), qualified, out);
            out.push_str(&format!(
                "{inner}  </xs:extension>\n{inner}</xs:simpleContent>\n"
            ));
        } else {
            self.write_attributes(&inner, qualified, out);
        }
        out.push_str(&format!("{pad}  </xs:complexType>\n{pad}</xs:element>\n"));
    }

    fn write_attributes(&self, pad: &str, qualified: bool, out: &mut String) {
        for (name, a) in &self.attributes {
            let required = if a.present == self.instances {
                " use=\"required\""
            } else {
                ""
            };
            if name.contains(':') {
                // A built-in attribute: the schema refers to it.
                out.push_str(&format!(
                    "{pad}<xs:attribute ref=\"{}\"{required}/>\n",
                    xmltree::escape_attribute(name)
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
