//! Lowering XSD (the v1 subset) into a [`TypeSpec`].
//!
//! See DESIGN.md, "Schema subsets (v1)" and "Notes on the XSD side". The
//! subset is closed record types (`xs:complexType` with one `xs:sequence` of
//! `xs:element`, and `xs:attribute`), arrays (`maxOccurs` above 1), and simple
//! types with a few restrictions. Anything outside it fails here with an error
//! that names the construct and where it was written. Nothing is silently
//! ignored, and unknown attributes and elements are errors too, so typos show.
//!
//! How XML maps onto the tree (DESIGN.md, "Data tree"): an attribute is a field
//! named `@name`, the text of an element that also has attributes is the field
//! `#text`, and a child element is a field named after it, an array if it may
//! repeat. An element of a simple type with no attributes is just its value.
//!
//! Decisions that refine the design (recorded in DESIGN.md under "Decisions
//! made while building step 6"):
//! - There is exactly one global `xs:element`, the root; every other global
//!   declaration is a named type. `ref` may name that element (recursion).
//! - One target namespace at most. Local elements are matched by their local
//!   name; `elementFormDefault` says whether they are in the namespace when
//!   the document is written. `attributeFormDefault` must be `unqualified`.
//! - The built-in types are `string`-like, `boolean`, integer-like,
//!   `decimal`-like, `date` and `dateTime`; the sign-restricted integers
//!   (`nonNegativeInteger`, `positiveInteger`, ...) carry their bound.
//! - A pattern is anchored at both ends, as XSD requires, and several
//!   `xs:pattern` facets in one restriction are alternatives.
//! - The text of an element with attributes is `#text` and is never required,
//!   since an empty element is a valid way to write an empty string.
//! - Mixed content (0.5.0) is `mixed="true"` with a `sequence` of one
//!   `xs:any processContents="skip"`: the inside is a string of markup
//!   ([`StringFormat::Markup`]), the element's value or, beside attributes,
//!   its `#text`.

use gossia_i18n::tr;
use std::collections::{HashMap, HashSet};
use std::fmt;

use crate::num::Num;
use crate::typespec::{
    Field, Kind, Pattern, Range, StringConstraints, StringFormat, TypeDef, TypeId, TypeSpec,
    XmlInfo,
};
use crate::value::{Node, Value};
use crate::xmltree::{self, Attribute, Child, Element, XmlError};

/// The namespace every XSD construct is in.
pub const XSD_NS: &str = "http://www.w3.org/2001/XMLSchema";

/// How deeply constructs may nest before lowering gives up.
const MAX_DEPTH: usize = 128;

/// A schema that the subset cannot take, with where and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaError {
    /// The element or attribute the error is about.
    pub keyword: String,
    /// Where it was written, such as `schema/element[order]/complexType`.
    pub location: String,
    pub message: String,
}

impl fmt::Display for SchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: `{}`: {}", self.location, self.keyword, self.message)
    }
}

impl std::error::Error for SchemaError {}

/// Why [`lower_str`] failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadError {
    NotXml(XmlError),
    Schema(SchemaError),
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::NotXml(e) => {
                f.write_str(&tr!("the schema is not well-formed XML: {e}", e = e))
            }
            LoadError::Schema(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for LoadError {}

/// The document a new XSD starts with: the prefix `xs` for the XML Schema
/// namespace, which the `type="xs:string"` of its declarations rely on.
pub fn default_document() -> crate::xml::XmlDocument {
    crate::xml::XmlDocument {
        declaration: Some(xmltree::Declaration::default()),
        root_prefix: Some("xs".to_string()),
        declarations: vec![(Some("xs".to_string()), XSD_NS.to_string())],
        prolog: Vec::new(),
        after: Vec::new(),
        without_declaration: false,
    }
}

/// Reads XSD text and lowers it.
pub fn lower_str(text: &str) -> Result<TypeSpec, LoadError> {
    let doc = xmltree::parse(text).map_err(LoadError::NotXml)?;
    lower(&doc.root).map_err(LoadError::Schema)
}

/// Lowers a schema document held as a data tree (a schema sheet's content, typed
/// by the XSD meta-schema). `namespaces` are the declarations on the root
/// element, which say what a prefix in `type="xs:string"` means.
pub fn lower_node(
    root: &Node,
    namespaces: &[(Option<String>, String)],
) -> Result<TypeSpec, SchemaError> {
    let mut element = element_from_node("schema", root);
    element.declarations = namespaces.to_vec();
    lower(&element)
}

/// The elements of an XSD, from the tree that the meta-schema types: keys that
/// start with `@` are attributes, `#text` is text, the rest are child elements,
/// repeated for an array.
fn element_from_node(name: &str, node: &Node) -> Element {
    let mut element = Element::new(name);
    element.namespace = Some(XSD_NS.to_string());
    match node.value() {
        Value::Object(fields) => {
            for (key, child) in fields {
                if let Some(attribute) = key.strip_prefix('@') {
                    if let Some(value) = crate::xml::scalar_text(child.value()) {
                        element.attributes.push(Attribute {
                            prefix: None,
                            name: attribute.to_string(),
                            value,
                        });
                    }
                } else if key == "#text" {
                    if let Some(text) = crate::xml::scalar_text(child.value()) {
                        element.children.push(Child::Text(text));
                    }
                } else if let Value::Array(items) = child.value() {
                    for item in items {
                        element
                            .children
                            .push(Child::Element(element_from_node(key, item)));
                    }
                } else {
                    element
                        .children
                        .push(Child::Element(element_from_node(key, child)));
                }
            }
        }
        other => {
            if let Some(text) = crate::xml::scalar_text(other) {
                element.children.push(Child::Text(text));
            }
        }
    }
    element
}

type Res<T> = Result<T, SchemaError>;

fn err<T>(keyword: &str, location: &str, message: impl Into<String>) -> Res<T> {
    Err(SchemaError {
        keyword: keyword.to_string(),
        location: location.to_string(),
        message: message.into(),
    })
}

/// Why a construct that this project knows is refused.
fn excluded_element(name: &str) -> Option<&'static str> {
    Some(match name {
        "choice" | "union" | "all" => {
            tr!("unions and alternatives are not supported in version 1")
        }
        "any" => tr!("`xs:any` is supported only as the content of a mixed type"),
        "group" | "attributeGroup" | "anyAttribute" => {
            tr!("groups and open content are not supported in version 1")
        }
        "complexContent" | "extension" | "restriction" => {
            tr!("type extension and derivation of complex types are not supported in version 1")
        }
        "import" | "include" | "redefine" | "override" => {
            tr!("imports, includes and non-local references are not supported in version 1")
        }
        "key" | "keyref" | "unique" | "selector" | "field" => {
            tr!("identity constraints are not supported in version 1")
        }
        "list" => tr!("list types are not supported in version 1"),
        "appinfo" | "notation" => tr!("only `documentation` is accepted as an annotation"),
        _ => return None,
    })
}

fn excluded_attribute(name: &str) -> Option<&'static str> {
    Some(match name {
        "nillable" => tr!("nullable elements are not supported in version 1"),
        "default" | "fixed" => tr!("default and fixed values are not supported in version 1"),
        "substitutionGroup" | "abstract" | "block" | "final" | "blockDefault" | "finalDefault" => {
            tr!("substitution and derivation control are not supported in version 1")
        }
        "form" | "attributeFormDefault" => {
            tr!("per-declaration forms are not supported in version 1")
        }
        "minExclusive" | "maxExclusive" => tr!("exclusive ranges are not supported in version 1"),
        _ => return None,
    })
}

/// Resolves the prefixes in the values of `type`, `base` and `ref` against the
/// namespaces in scope, so that the rest of lowering sees `{uri}local` (or just
/// `local` when there is no namespace) and never has to think about prefixes.
fn resolve_names(
    element: &mut Element,
    scopes: &mut Vec<Vec<(Option<String>, String)>>,
    depth: usize,
) -> Res<()> {
    if depth >= MAX_DEPTH {
        return err(
            &element.name,
            "schema",
            tr!(
                "constructs nested more than {MAX_DEPTH} levels deep are not supported",
                MAX_DEPTH = MAX_DEPTH
            ),
        );
    }
    scopes.push(element.declarations.clone());
    let lookup = |scopes: &[Vec<(Option<String>, String)>], prefix: Option<&str>| {
        scopes
            .iter()
            .rev()
            .flatten()
            .find(|(p, _)| p.as_deref() == prefix)
            .map(|(_, uri)| uri.clone())
            .filter(|uri| !uri.is_empty())
    };
    for attribute in &mut element.attributes {
        if attribute.prefix.is_some() || !matches!(attribute.name.as_str(), "type" | "base" | "ref")
        {
            continue;
        }
        let (prefix, local) = match attribute.value.split_once(':') {
            Some((p, l)) => (Some(p), l),
            None => (None, attribute.value.as_str()),
        };
        let namespace = lookup(scopes, prefix);
        if prefix.is_some() && namespace.is_none() {
            return err(
                &attribute.name,
                &element.name,
                tr!(
                    "the prefix `{prefix}` is not declared",
                    prefix = prefix.unwrap_or_default()
                ),
            );
        }
        attribute.value = match namespace {
            Some(uri) => format!("{{{uri}}}{local}"),
            None => local.to_string(),
        };
    }
    for child in &mut element.children {
        if let Child::Element(e) = child {
            resolve_names(e, scopes, depth + 1)?;
        }
    }
    scopes.pop();
    Ok(())
}

/// Splits `{uri}local`.
fn clark(name: &str) -> (Option<&str>, &str) {
    match name.strip_prefix('{').and_then(|r| r.split_once('}')) {
        Some((uri, local)) => (Some(uri), local),
        None => (None, name),
    }
}

/// Lowers a schema document.
pub fn lower(root: &Element) -> Res<TypeSpec> {
    let mut root = root.clone();
    resolve_names(&mut root, &mut Vec::new(), 0)?;
    let mut lowerer = Lowerer {
        types: Vec::new(),
        complex: HashMap::new(),
        simple: HashMap::new(),
        done: HashMap::new(),
        target: None,
        qualified: false,
        root_name: String::new(),
        root_id: TypeId(0),
        depth: 0,
    };
    lowerer.run(&root)
}

struct Lowerer<'a> {
    types: Vec<TypeDef>,
    complex: HashMap<String, &'a Element>,
    simple: HashMap<String, &'a Element>,
    /// Named types already lowered or being lowered.
    done: HashMap<String, TypeId>,
    target: Option<String>,
    qualified: bool,
    root_name: String,
    root_id: TypeId,
    depth: usize,
}

/// The local name of an XSD element, or an error if it is not one.
fn xs_name<'e>(element: &'e Element, location: &str) -> Res<&'e str> {
    if element.namespace.as_deref() != Some(XSD_NS) {
        return err(
            &element.name,
            location,
            match &element.namespace {
                Some(n) => tr!(
                    "is not in the XML Schema namespace ({namespace}), but in {n}",
                    namespace = XSD_NS,
                    n = n
                ),
                None => tr!(
                    "is not in the XML Schema namespace ({namespace})",
                    namespace = XSD_NS
                ),
            },
        );
    }
    Ok(&element.name)
}

/// Checks that the attributes are all among `allowed`.
fn check_attributes(element: &Element, allowed: &[&str], location: &str) -> Res<()> {
    for a in &element.attributes {
        if a.prefix.is_some() {
            return err(
                &format!("{}:{}", a.prefix.as_deref().unwrap_or_default(), a.name),
                location,
                tr!("attributes from other namespaces are not supported in version 1"),
            );
        }
        if !allowed.contains(&a.name.as_str()) {
            return match excluded_attribute(&a.name) {
                Some(reason) => err(&a.name, location, reason),
                None => err(
                    &a.name,
                    location,
                    tr!(
                        "is not an attribute of `{name}` that this version recognises",
                        name = element.name
                    ),
                ),
            };
        }
    }
    Ok(())
}

/// Checks that the children are XSD elements among `allowed`, and that there
/// is no text but whitespace.
fn check_children(element: &Element, allowed: &[&str], location: &str) -> Res<()> {
    for child in &element.children {
        match child {
            Child::Comment(_) => {}
            Child::Text(t) if t.trim().is_empty() => {}
            Child::Text(_) => {
                return err(&element.name, location, tr!("text is not allowed here"));
            }
            Child::Element(e) => {
                let name = xs_name(e, location)?;
                if !allowed.contains(&name) {
                    return match excluded_element(name) {
                        Some(reason) => err(name, location, reason),
                        None => err(
                            name,
                            location,
                            tr!(
                                "is not allowed inside `{name}` in this version",
                                name = element.name
                            ),
                        ),
                    };
                }
            }
        }
    }
    Ok(())
}

fn named_children<'e>(element: &'e Element, name: &str) -> Vec<&'e Element> {
    element
        .elements()
        .filter(|e| e.name == name)
        .collect::<Vec<_>>()
}

/// The first `xs:documentation` of the `xs:annotation` of an element.
fn documentation(element: &Element, location: &str) -> Res<Option<String>> {
    let annotations = named_children(element, "annotation");
    if annotations.len() > 1 {
        return err(
            "annotation",
            location,
            tr!("only one `annotation` is allowed"),
        );
    }
    let Some(annotation) = annotations.first() else {
        return Ok(None);
    };
    let here = format!("{location}/annotation");
    check_attributes(annotation, &[], &here)?;
    check_children(annotation, &["documentation"], &here)?;
    let first = named_children(annotation, "documentation");
    Ok(first
        .first()
        .map(|d| d.text().trim().to_string())
        .filter(|t| !t.is_empty()))
}

/// The occurrence bounds of an element.
fn occurs(element: &Element, location: &str) -> Res<(usize, Option<usize>)> {
    let count = |name: &str, default: usize| -> Res<usize> {
        match element.attribute(name) {
            None => Ok(default),
            Some(v) => v.trim().parse::<usize>().or_else(|_| {
                err(
                    name,
                    location,
                    tr!("`{name}` must be a non-negative whole number", name = name),
                )
            }),
        }
    };
    let min = count("minOccurs", 1)?;
    let max = match element.attribute("maxOccurs") {
        None => Some(1),
        Some("unbounded") => None,
        Some(v) => match v.trim().parse::<usize>() {
            Ok(n) => Some(n),
            Err(_) => {
                return err(
                    "maxOccurs",
                    location,
                    tr!("`maxOccurs` must be a whole number or `unbounded`"),
                );
            }
        },
    };
    if max == Some(0) {
        return err(
            "maxOccurs",
            location,
            tr!("an element that may occur 0 times is not supported"),
        );
    }
    if let Some(max) = max
        && min > max
    {
        return err("minOccurs", location, tr!("is more than `maxOccurs`"));
    }
    Ok((min, max))
}

impl<'a> Lowerer<'a> {
    fn push(&mut self, kind: Kind, location: &str) -> TypeId {
        self.types.push(TypeDef {
            kind,
            source: location.to_string(),
            name: None,
            title: None,
            description: None,
            choices: Vec::new(),
        });
        TypeId(u32::try_from(self.types.len() - 1).expect("fewer than 2^32 types"))
    }

    fn run(&mut self, root: &'a Element) -> Res<TypeSpec> {
        let location = "schema";
        if xs_name(root, location)? != "schema" {
            return err(
                &root.name,
                location,
                tr!("a schema document has `xs:schema` as its root"),
            );
        }
        check_attributes(
            root,
            &[
                "targetNamespace",
                "elementFormDefault",
                "attributeFormDefault",
                "version",
                "id",
            ],
            location,
        )?;
        for banned in ["version", "id"] {
            if root.attribute(banned).is_some() {
                return err(banned, location, tr!("is not supported in version 1"));
            }
        }
        self.target = root.attribute("targetNamespace").map(str::to_string);
        self.qualified = match root.attribute("elementFormDefault") {
            None | Some("unqualified") => false,
            Some("qualified") => true,
            Some(_) => {
                return err(
                    "elementFormDefault",
                    location,
                    tr!("must be `qualified` or `unqualified`"),
                );
            }
        };
        if let Some(v) = root.attribute("attributeFormDefault")
            && v != "unqualified"
        {
            return err(
                "attributeFormDefault",
                location,
                tr!("only `unqualified` is supported in version 1"),
            );
        }
        check_children(
            root,
            &["element", "complexType", "simpleType", "annotation"],
            location,
        )?;
        documentation(root, location)?;
        let mut globals: Vec<&'a Element> = Vec::new();
        for child in root.elements() {
            match child.name.as_str() {
                "element" => globals.push(child),
                kind @ ("complexType" | "simpleType") => {
                    let Some(name) = child.attribute("name") else {
                        return err(kind, location, tr!("a global type needs a `name`"));
                    };
                    let table = if kind == "complexType" {
                        &mut self.complex
                    } else {
                        &mut self.simple
                    };
                    let clash = table.contains_key(name)
                        || self.complex.contains_key(name)
                        || self.simple.contains_key(name);
                    if clash {
                        return err(
                            kind,
                            location,
                            tr!("there are two types named `{name}`", name = name),
                        );
                    }
                    if kind == "complexType" {
                        self.complex.insert(name.to_string(), child);
                    } else {
                        self.simple.insert(name.to_string(), child);
                    }
                }
                _ => {}
            }
        }
        let root_element = match globals.as_slice() {
            [one] => *one,
            [] => {
                return err(
                    "element",
                    location,
                    tr!("a schema needs one global `xs:element`, the root of its documents"),
                );
            }
            _ => {
                return err(
                    "element",
                    location,
                    tr!(
                        "a schema has one global `xs:element`; the other global declarations must be types"
                    ),
                );
            }
        };
        let Some(root_name) = root_element.attribute("name") else {
            return err(
                "element",
                location,
                tr!("the global element needs a `name`"),
            );
        };
        self.root_name = root_name.to_string();
        let here = format!("{location}/element[{root_name}]");
        check_attributes(root_element, &["name", "type"], &here)?;
        self.root_id = self.push(Kind::Bool, &here);
        let ty = self.element_type(root_element, &here)?;
        self.types[self.root_id.0 as usize] = self.types[ty.0 as usize].clone();
        // Lower every named type, used or not, so that mistakes in them surface.
        let mut names: Vec<String> = self
            .complex
            .keys()
            .chain(self.simple.keys())
            .cloned()
            .collect();
        names.sort();
        for name in names {
            self.named_type(&name, &format!("{location}/type[{name}]"))?;
        }
        Ok(
            TypeSpec::new(std::mem::take(&mut self.types), self.root_id).with_xml(XmlInfo {
                root: self.root_name.clone(),
                target_namespace: self.target.clone(),
                qualified: self.qualified,
            }),
        )
    }

    /// The type of an element: from `type=`, or from the type written inside it.
    fn element_type(&mut self, element: &'a Element, location: &str) -> Res<TypeId> {
        if self.depth >= MAX_DEPTH {
            return err(
                "element",
                location,
                tr!(
                    "constructs nested more than {MAX_DEPTH} levels deep are not supported",
                    MAX_DEPTH = MAX_DEPTH
                ),
            );
        }
        self.depth += 1;
        let result = self.element_type_inner(element, location);
        self.depth -= 1;
        result
    }

    fn element_type_inner(&mut self, element: &'a Element, location: &str) -> Res<TypeId> {
        check_children(
            element,
            &["annotation", "complexType", "simpleType"],
            location,
        )?;
        documentation(element, location)?;
        let complex = named_children(element, "complexType");
        let simple = named_children(element, "simpleType");
        let written = complex.len() + simple.len();
        match (element.attribute("type"), written) {
            (Some(_), 1) => err(
                "type",
                location,
                tr!("an element has `type` or a type written inside it, not both"),
            ),
            (Some(name), _) => self.resolve_type(name, location),
            (None, 0) => err(
                "type",
                location,
                tr!("an element needs a `type`, or a `complexType` or `simpleType` inside it"),
            ),
            (None, 1) => {
                if let Some(ct) = complex.first() {
                    let here = format!("{location}/complexType");
                    if ct.attribute("name").is_some() {
                        return err(
                            "name",
                            &here,
                            tr!("a type written inside an element has no name"),
                        );
                    }
                    let (kind, description) = self.complex_kind(ct, &here)?;
                    let id = self.push(kind, &here);
                    self.types[id.0 as usize].description = description;
                    Ok(id)
                } else {
                    let st = simple[0];
                    let here = format!("{location}/simpleType");
                    if st.attribute("name").is_some() {
                        return err(
                            "name",
                            &here,
                            tr!("a type written inside an element has no name"),
                        );
                    }
                    let (kind, description) = self.simple_kind(st, &here)?;
                    let id = self.push(kind, &here);
                    self.types[id.0 as usize].description = description;
                    Ok(id)
                }
            }
            (None, _) => err("complexType", location, tr!("an element has one type")),
        }
    }

    /// Resolves a type name (`{uri}local`) to a type.
    fn resolve_type(&mut self, name: &str, location: &str) -> Res<TypeId> {
        let (uri, local) = clark(name);
        // A schema that describes XSD itself (the meta-schema) has the XML Schema
        // namespace as its target, so a name there may be one of its own types;
        // its own types come first.
        let own = uri == self.target.as_deref()
            && (self.complex.contains_key(local) || self.simple.contains_key(local));
        if uri == Some(XSD_NS) && !own {
            let kind = builtin(local, location)?;
            return Ok(self.push(kind, location));
        }
        if uri != self.target.as_deref() {
            return err(
                "type",
                location,
                tr!(
                    "`{name}` is in another namespace; only one namespace is supported in version 1",
                    name = name
                ),
            );
        }
        self.named_type(local, location)
    }

    /// A type declared at the top level, lowered the first time it is used.
    fn named_type(&mut self, name: &str, location: &str) -> Res<TypeId> {
        if let Some(&id) = self.done.get(name) {
            return Ok(id);
        }
        let here = format!("schema/type[{name}]");
        let (element, is_complex) = if let Some(e) = self.complex.get(name) {
            (*e, true)
        } else if let Some(e) = self.simple.get(name) {
            (*e, false)
        } else {
            return err(
                "type",
                location,
                if self.target.is_some() {
                    tr!(
                        "there is no type named `{name}` in the target namespace",
                        name = name
                    )
                } else {
                    tr!("there is no type named `{name}`", name = name)
                },
            );
        };
        let id = self.push(Kind::Bool, &here);
        self.done.insert(name.to_string(), id);
        let (kind, description) = if is_complex {
            self.complex_kind(element, &here)?
        } else {
            self.simple_kind(element, &here)?
        };
        let def = &mut self.types[id.0 as usize];
        def.kind = kind;
        def.name = Some(name.to_string());
        def.description = description;
        Ok(id)
    }

    // ------------------------------------------------------------ complex

    fn complex_kind(&mut self, ct: &'a Element, location: &str) -> Res<(Kind, Option<String>)> {
        check_attributes(ct, &["name", "mixed"], location)?;
        check_children(
            ct,
            &["annotation", "sequence", "simpleContent", "attribute"],
            location,
        )?;
        let description = documentation(ct, location)?;
        match ct.attribute("mixed").map(str::trim) {
            None | Some("false" | "0") => {}
            Some("true" | "1") => return self.mixed_kind(ct, location, description),
            Some(_) => return err("mixed", location, tr!("must be `true` or `false`")),
        }
        let sequences = named_children(ct, "sequence");
        let simple_content = named_children(ct, "simpleContent");
        if sequences.len() + simple_content.len() > 1 {
            return err(
                "sequence",
                location,
                tr!("a type has one `sequence` or one `simpleContent`"),
            );
        }
        // Attributes first, then the content.
        let mut fields: Vec<Field> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let attribute_parent: &Element = if let Some(sc) = simple_content.first() {
            if !named_children(ct, "attribute").is_empty() {
                return err(
                    "attribute",
                    location,
                    tr!("attributes of a `simpleContent` go inside its `extension`"),
                );
            }
            sc
        } else {
            ct
        };
        let mut text_type: Option<TypeId> = None;
        let attribute_owner: &Element = if let Some(sc) = simple_content.first() {
            let here = format!("{location}/simpleContent");
            check_attributes(sc, &[], &here)?;
            check_children(sc, &["annotation", "extension"], &here)?;
            let Some(ext) = named_children(sc, "extension").first().copied() else {
                return err(
                    "simpleContent",
                    &here,
                    tr!(
                        "needs an `extension` of a simple type (a `restriction` is not supported in version 1)"
                    ),
                );
            };
            let ext_here = format!("{here}/extension");
            check_attributes(ext, &["base"], &ext_here)?;
            check_children(ext, &["annotation", "attribute"], &ext_here)?;
            let Some(base) = ext.attribute("base") else {
                return err("base", &ext_here, tr!("an `extension` needs a `base`"));
            };
            let base_id = self.resolve_type(base, &ext_here)?;
            if matches!(self.types[base_id.0 as usize].kind, Kind::Object { .. }) {
                return err(
                    "base",
                    &ext_here,
                    tr!("the base of a `simpleContent` must be a simple type"),
                );
            }
            text_type = Some(base_id);
            ext
        } else {
            attribute_parent
        };
        let owner_here = if simple_content.is_empty() {
            location.to_string()
        } else {
            format!("{location}/simpleContent/extension")
        };
        for attribute in named_children(attribute_owner, "attribute") {
            let (name, field) = self.attribute_field(attribute, &owner_here)?;
            if !seen.insert(name.clone()) {
                return err(
                    "attribute",
                    &owner_here,
                    tr!("there are two attributes named `{name}`", name = name),
                );
            }
            fields.push(field);
        }
        if let Some(ty) = text_type {
            fields.push(Field {
                name: "#text".to_string(),
                ty,
                required: false,
            });
        }
        if let Some(sequence) = sequences.first() {
            let here = format!("{location}/sequence");
            check_attributes(sequence, &["minOccurs", "maxOccurs"], &here)?;
            if occurs(sequence, &here)? != (1, Some(1)) {
                return err(
                    "maxOccurs",
                    &here,
                    tr!("a `sequence` that repeats is not supported in version 1"),
                );
            }
            check_children(sequence, &["annotation", "element"], &here)?;
            documentation(sequence, &here)?;
            let mut elements = HashSet::new();
            for element in named_children(sequence, "element") {
                let (name, field) = self.element_field(element, &here)?;
                if !elements.insert(name.clone()) {
                    return err(
                        "element",
                        &here,
                        tr!("there are two elements named `{name}`", name = name),
                    );
                }
                fields.push(field);
            }
        }
        Ok((Kind::Object { fields }, description))
    }

    /// A type of mixed content: a string of markup, with the attributes beside
    /// it as fields when there are any (the markup is then `#text`). Its
    /// content must be the one form that says "any markup": a `sequence` of
    /// one `xs:any` that is skipped, optional and unbounded.
    fn mixed_kind(
        &mut self,
        ct: &'a Element,
        location: &str,
        description: Option<String>,
    ) -> Res<(Kind, Option<String>)> {
        let form = || {
            tr!(
                "mixed content is written as a `sequence` holding one `xs:any` with \
                 processContents=\"skip\", minOccurs=\"0\" and maxOccurs=\"unbounded\""
            )
        };
        if !named_children(ct, "simpleContent").is_empty() {
            return err("simpleContent", location, form());
        }
        let sequences = named_children(ct, "sequence");
        let [sequence] = sequences.as_slice() else {
            return err("mixed", location, form());
        };
        let here = format!("{location}/sequence");
        check_attributes(sequence, &[], &here)?;
        if sequence
            .elements()
            .any(|e| e.name != "annotation" && e.name != "any")
        {
            return err("sequence", &here, form());
        }
        check_children(sequence, &["annotation", "any"], &here)?;
        documentation(sequence, &here)?;
        let anys = named_children(sequence, "any");
        let [any] = anys.as_slice() else {
            return err("sequence", &here, form());
        };
        let any_here = format!("{here}/any");
        check_attributes(
            any,
            &["processContents", "minOccurs", "maxOccurs", "namespace"],
            &any_here,
        )?;
        check_children(any, &["annotation"], &any_here)?;
        let skipped = any.attribute("processContents") == Some("skip");
        let any_namespace = matches!(any.attribute("namespace"), None | Some("##any"));
        if !skipped || !any_namespace || occurs(any, &any_here)? != (0, None) {
            return err("any", &any_here, form());
        }
        let markup = self.push(
            Kind::String(StringConstraints {
                format: Some(StringFormat::Markup),
                ..StringConstraints::default()
            }),
            &any_here,
        );
        let mut fields: Vec<Field> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for attribute in named_children(ct, "attribute") {
            let (name, field) = self.attribute_field(attribute, location)?;
            if !seen.insert(name.clone()) {
                return err(
                    "attribute",
                    location,
                    tr!("there are two attributes named `{name}`", name = name),
                );
            }
            fields.push(field);
        }
        if fields.is_empty() {
            let kind = self.types[markup.0 as usize].kind.clone();
            return Ok((kind, description));
        }
        fields.push(Field {
            name: "#text".to_string(),
            ty: markup,
            required: false,
        });
        Ok((Kind::Object { fields }, description))
    }

    /// A field for an `xs:attribute`: named `@name`, a simple type.
    fn attribute_field(&mut self, attribute: &'a Element, parent: &str) -> Res<(String, Field)> {
        let Some(name) = attribute.attribute("name") else {
            return err(
                "attribute",
                parent,
                tr!("an attribute needs a `name` (`ref` is not supported in version 1)"),
            );
        };
        let here = format!("{parent}/attribute[{name}]");
        check_attributes(attribute, &["name", "type", "use"], &here)?;
        check_children(attribute, &["annotation", "simpleType"], &here)?;
        documentation(attribute, &here)?;
        let required = match attribute.attribute("use") {
            None | Some("optional") => false,
            Some("required") => true,
            Some("prohibited") => {
                return err(
                    "use",
                    &here,
                    tr!("`prohibited` is not supported in version 1"),
                );
            }
            Some(_) => return err("use", &here, tr!("must be `optional` or `required`")),
        };
        let inner = named_children(attribute, "simpleType");
        let ty = match (attribute.attribute("type"), inner.first()) {
            (Some(_), Some(_)) => {
                return err(
                    "type",
                    &here,
                    tr!("an attribute has `type` or a `simpleType` inside it, not both"),
                );
            }
            (Some(type_name), None) => self.resolve_type(type_name, &here)?,
            (None, Some(st)) => {
                let there = format!("{here}/simpleType");
                let (kind, description) = self.simple_kind(st, &there)?;
                let id = self.push(kind, &there);
                self.types[id.0 as usize].description = description;
                id
            }
            (None, None) => {
                return err("type", &here, tr!("an attribute needs a `type`"));
            }
        };
        if matches!(
            self.types[ty.0 as usize].kind,
            Kind::Object { .. } | Kind::Array { .. }
        ) {
            return err("type", &here, tr!("an attribute has a simple type"));
        }
        if name.contains(':') || name.is_empty() {
            return err("name", &here, tr!("is not a valid attribute name"));
        }
        Ok((
            name.to_string(),
            Field {
                name: format!("@{name}"),
                ty,
                required,
            },
        ))
    }

    /// A field for an `xs:element` in a sequence.
    fn element_field(&mut self, element: &'a Element, parent: &str) -> Res<(String, Field)> {
        let (min, max) = occurs(element, parent)?;
        let (name, ty, here) = if let Some(reference) = element.attribute("ref") {
            let (uri, local) = clark(reference);
            let here = format!("{parent}/element[{local}]");
            check_attributes(element, &["ref", "minOccurs", "maxOccurs"], &here)?;
            check_children(element, &["annotation"], &here)?;
            if uri != self.target.as_deref() || local != self.root_name {
                return err(
                    "ref",
                    &here,
                    tr!(
                        "only the global element `{root_name}` can be referred to",
                        root_name = self.root_name
                    ),
                );
            }
            (local.to_string(), self.root_id, here)
        } else {
            let Some(name) = element.attribute("name") else {
                return err(
                    "element",
                    parent,
                    tr!("an element needs a `name` (or a `ref`)"),
                );
            };
            if name.contains(':')
                || name.is_empty()
                || name.starts_with('@')
                || name.starts_with('#')
            {
                return err(
                    "name",
                    parent,
                    tr!("`{name}` is not a valid element name", name = name),
                );
            }
            let here = format!("{parent}/element[{name}]");
            check_attributes(element, &["name", "type", "minOccurs", "maxOccurs"], &here)?;
            let ty = self.element_type(element, &here)?;
            (name.to_string(), ty, here)
        };
        let _ = here;
        let (ty, required) = if max == Some(1) {
            (ty, min >= 1)
        } else {
            let array = self.push(
                Kind::Array {
                    items: ty,
                    min_items: (min >= 1).then_some(min),
                    max_items: max,
                },
                &format!("{parent}/element[{name}]"),
            );
            (array, min >= 1)
        };
        Ok((name.clone(), Field { name, ty, required }))
    }

    // ------------------------------------------------------------- simple

    fn simple_kind(&mut self, st: &'a Element, location: &str) -> Res<(Kind, Option<String>)> {
        check_attributes(st, &["name"], location)?;
        check_children(st, &["annotation", "restriction"], location)?;
        let description = documentation(st, location)?;
        let restrictions = named_children(st, "restriction");
        let Some(restriction) = restrictions.first() else {
            return err(
                "simpleType",
                location,
                tr!("needs a `restriction` (`list` and `union` are not supported in version 1)"),
            );
        };
        let here = format!("{location}/restriction");
        check_attributes(restriction, &["base"], &here)?;
        check_children(
            restriction,
            &[
                "annotation",
                "enumeration",
                "pattern",
                "minInclusive",
                "maxInclusive",
                "minLength",
                "maxLength",
            ],
            &here,
        )?;
        let Some(base) = restriction.attribute("base") else {
            return err("base", &here, "a `restriction` needs a `base`");
        };
        let base_id = self.resolve_type(base, &here)?;
        let base_kind = self.types[base_id.0 as usize].kind.clone();
        let kind = apply_facets(base_kind, restriction, &here)?;
        Ok((kind, description))
    }
}

/// The kind of a built-in type.
fn builtin(local: &str, location: &str) -> Res<Kind> {
    let whole = |min: Option<i64>, max: Option<i64>| {
        Kind::Integer(Range {
            min: min.map(Num::Int),
            max: max.map(Num::Int),
        })
    };
    Ok(match local {
        "string" | "normalizedString" | "token" | "anyURI" | "NCName" | "Name" | "ID"
        | "language" | "NMTOKEN" => Kind::String(StringConstraints::default()),
        "boolean" => Kind::Bool,
        "integer" | "int" | "long" | "short" | "byte" | "unsignedInt" | "unsignedLong"
        | "unsignedShort" | "unsignedByte" => whole(None, None),
        "nonNegativeInteger" => whole(Some(0), None),
        "positiveInteger" => whole(Some(1), None),
        "nonPositiveInteger" => whole(None, Some(0)),
        "negativeInteger" => whole(None, Some(-1)),
        "decimal" | "double" | "float" => Kind::Number(Range::default()),
        "date" => Kind::Date,
        "dateTime" => Kind::DateTime,
        "anyType" | "anySimpleType" => {
            return err(
                "type",
                location,
                tr!(
                    "`{local}` allows any content, which is not supported in version 1",
                    local = local
                ),
            );
        }
        other => {
            return err(
                "type",
                location,
                tr!(
                    "the built-in type `{other}` is not supported in version 1",
                    other = other
                ),
            );
        }
    })
}

/// Applies the facets of a restriction to the kind of its base.
fn apply_facets(base: Kind, restriction: &Element, location: &str) -> Res<Kind> {
    let mut enumerations: Vec<String> = Vec::new();
    let mut patterns: Vec<String> = Vec::new();
    let (mut min_inc, mut max_inc, mut min_len, mut max_len) = (None, None, None, None);
    let mut any = false;
    for facet in restriction.elements() {
        if facet.name == "annotation" {
            continue;
        }
        any = true;
        let here = format!("{location}/{}", facet.name);
        check_attributes(facet, &["value"], &here)?;
        check_children(facet, &["annotation"], &here)?;
        let Some(value) = facet.attribute("value") else {
            return err(&facet.name, &here, tr!("a facet needs a `value`"));
        };
        let once = |slot: &mut Option<String>| -> Res<()> {
            if slot.replace(value.to_string()).is_some() {
                return err(&facet.name, &here, tr!("is given twice"));
            }
            Ok(())
        };
        match facet.name.as_str() {
            "enumeration" => {
                if enumerations.iter().any(|v| v == value) {
                    return err(
                        "enumeration",
                        &here,
                        tr!("`{value}` is listed twice", value = value),
                    );
                }
                enumerations.push(value.to_string());
            }
            "pattern" => patterns.push(value.to_string()),
            "minInclusive" => once(&mut min_inc)?,
            "maxInclusive" => once(&mut max_inc)?,
            "minLength" => once(&mut min_len)?,
            "maxLength" => once(&mut max_len)?,
            _ => unreachable!("checked by check_children"),
        }
    }
    let _ = any;
    let length = |text: Option<String>, name: &str| -> Res<Option<usize>> {
        match text {
            None => Ok(None),
            Some(t) => t.trim().parse::<usize>().map(Some).or_else(|_| {
                err(
                    name,
                    location,
                    tr!("`{name}` must be a non-negative whole number", name = name),
                )
            }),
        }
    };
    let (min_len, max_len) = (length(min_len, "minLength")?, length(max_len, "maxLength")?);
    let pattern = pattern_of(&patterns, location)?;
    let string_facets = pattern.is_some() || min_len.is_some() || max_len.is_some();
    let range_facets = min_inc.is_some() || max_inc.is_some();
    let refuse = |what: &str, facet: &str| -> Res<Kind> {
        err(
            "restriction",
            location,
            tr!(
                "{what} cannot be restricted by {facet} in version 1",
                what = what,
                facet = facet
            ),
        )
    };
    match base {
        Kind::String(strings) => {
            if range_facets {
                return refuse("a string", tr!("`minInclusive` or `maxInclusive`"));
            }
            let strings = merge_strings(strings, pattern, min_len, max_len, location)?;
            if enumerations.is_empty() {
                Ok(Kind::String(strings))
            } else {
                Ok(Kind::Enum {
                    values: enumerations,
                    strings,
                })
            }
        }
        Kind::Enum { values, strings } => {
            if range_facets {
                return refuse("a string", tr!("`minInclusive` or `maxInclusive`"));
            }
            let strings = merge_strings(strings, pattern, min_len, max_len, location)?;
            let values = if enumerations.is_empty() {
                values
            } else {
                for v in &enumerations {
                    if !values.contains(v) {
                        return err(
                            "enumeration",
                            location,
                            tr!("`{v}` is not one of the values of the base type", v = v),
                        );
                    }
                }
                enumerations
            };
            Ok(Kind::Enum { values, strings })
        }
        Kind::Integer(range) => {
            if !enumerations.is_empty() || string_facets {
                return refuse(
                    tr!("an integer"),
                    tr!("an enumeration, a pattern or a length"),
                );
            }
            Ok(Kind::Integer(merge_range(
                range, min_inc, max_inc, true, location,
            )?))
        }
        Kind::Number(range) => {
            if !enumerations.is_empty() || string_facets {
                return refuse("a number", tr!("an enumeration, a pattern or a length"));
            }
            Ok(Kind::Number(merge_range(
                range, min_inc, max_inc, false, location,
            )?))
        }
        other => {
            if !enumerations.is_empty() || string_facets || range_facets {
                return refuse(tr!("this type"), tr!("any facet"));
            }
            Ok(other)
        }
    }
}

/// Adds a pattern and lengths to the constraints of a base string type. A
/// pattern the base already has cannot be replaced; lengths can only tighten.
fn merge_strings(
    mut strings: StringConstraints,
    pattern: Option<Pattern>,
    min_len: Option<usize>,
    max_len: Option<usize>,
    location: &str,
) -> Res<StringConstraints> {
    if pattern.is_some() {
        if strings.pattern.is_some() {
            return err(
                "pattern",
                location,
                tr!(
                    "the base type already has a pattern; restricting it further is not supported in version 1"
                ),
            );
        }
        strings.pattern = pattern;
    }
    strings.min_length = match (strings.min_length, min_len) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (a, b) => a.or(b),
    };
    strings.max_length = match (strings.max_length, max_len) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    };
    if let (Some(lo), Some(hi)) = (strings.min_length, strings.max_length)
        && lo > hi
    {
        return err("minLength", location, tr!("is more than `maxLength`"));
    }
    Ok(strings)
}

/// Adds bounds to the range of a base numeric type; bounds can only tighten.
fn merge_range(
    mut range: Range,
    min: Option<String>,
    max: Option<String>,
    whole: bool,
    location: &str,
) -> Res<Range> {
    let bound = |text: &str, name: &str| -> Res<Num> {
        let text = text.trim();
        if let Ok(i) = text.parse::<i64>() {
            return Ok(Num::Int(i));
        }
        match text.parse::<f64>() {
            Ok(x) if x.is_finite() && !whole => Ok(Num::Float(x)),
            _ => err(
                name,
                location,
                if whole {
                    tr!("`{name}` must be a whole number here", name = name)
                } else {
                    tr!("`{name}` must be a number", name = name)
                },
            ),
        }
    };
    if let Some(text) = min {
        let new = bound(&text, "minInclusive")?;
        range.min = Some(match range.min {
            Some(old) if old.compare(new) == Some(std::cmp::Ordering::Greater) => old,
            _ => new,
        });
    }
    if let Some(text) = max {
        let new = bound(&text, "maxInclusive")?;
        range.max = Some(match range.max {
            Some(old) if old.compare(new) == Some(std::cmp::Ordering::Less) => old,
            _ => new,
        });
    }
    if let (Some(lo), Some(hi)) = (range.min, range.max)
        && lo.compare(hi) == Some(std::cmp::Ordering::Greater)
    {
        return err("minInclusive", location, tr!("is more than `maxInclusive`"));
    }
    Ok(range)
}

fn pattern_of(patterns: &[String], location: &str) -> Res<Option<Pattern>> {
    if patterns.is_empty() {
        return Ok(None);
    }
    let source = if patterns.len() == 1 {
        format!("^(?:{})$", patterns[0])
    } else {
        let alternatives: Vec<String> = patterns.iter().map(|p| format!("(?:{p})")).collect();
        format!("^(?:{})$", alternatives.join("|"))
    };
    match Pattern::new(&source) {
        Ok(p) => Ok(Some(p)),
        Err(e) => err(
            "pattern",
            location,
            tr!("this regular expression is not supported: {e}", e = e),
        ),
    }
}

#[cfg(test)]
#[path = "xsd_tests.rs"]
pub(crate) mod tests;
