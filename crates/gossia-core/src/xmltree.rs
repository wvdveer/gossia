//! Reading and writing XML text as a plain tree of elements (DESIGN.md, "Core
//! model": "XML details preserved" and "Not preserved").
//!
//! This is the layer under both the XSD lowering and the XML data adapter. It
//! knows nothing about schemas. It keeps what the design says is kept: the
//! declaration, prefixes and namespace declarations, attribute order, element
//! order, comments, and the document type declaration and processing
//! instructions outside the root, as written (entities declared in a document
//! type declaration are never expanded and nothing external is ever fetched).
//! It drops what the design says is dropped: processing instructions inside
//! the root (with a warning). CDATA sections become ordinary text.
//!
//! Reading is bounded: nesting deeper than [`MAX_DEPTH`] is refused, and the
//! tree is built without recursion so that depth cannot overflow the stack.

use gossia_i18n::tr;
use std::fmt;

use quick_xml::XmlVersion;
use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;

/// How deeply elements may nest.
pub const MAX_DEPTH: usize = 128;

/// An attribute, other than a namespace declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attribute {
    pub prefix: Option<String>,
    pub name: String,
    pub value: String,
}

/// What an element holds, in document order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Child {
    Element(Element),
    Text(String),
    Comment(String),
}

/// An element, with its name split into prefix and local name and resolved to
/// a namespace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Element {
    pub prefix: Option<String>,
    /// The name without its prefix.
    pub name: String,
    /// The namespace the name belongs to, if any.
    pub namespace: Option<String>,
    /// The namespaces declared on this element: `(prefix, uri)`, with `None`
    /// for the default namespace.
    pub declarations: Vec<(Option<String>, String)>,
    pub attributes: Vec<Attribute>,
    pub children: Vec<Child>,
}

impl Element {
    pub fn new(name: &str) -> Element {
        Element {
            prefix: None,
            name: name.to_string(),
            namespace: None,
            declarations: Vec::new(),
            attributes: Vec::new(),
            children: Vec::new(),
        }
    }

    /// The value of the attribute with this local name and no prefix.
    pub fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|a| a.prefix.is_none() && a.name == name)
            .map(|a| a.value.as_str())
    }

    /// The child elements, in order.
    pub fn elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|c| match c {
            Child::Element(e) => Some(e),
            _ => None,
        })
    }

    /// Whether any child is text that is not only whitespace.
    pub fn has_text(&self) -> bool {
        self.children
            .iter()
            .any(|c| matches!(c, Child::Text(t) if !t.trim().is_empty()))
    }

    /// All the text the element holds directly, joined.
    pub fn text(&self) -> String {
        self.children
            .iter()
            .filter_map(|c| match c {
                Child::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect()
    }
}

/// The XML declaration, `<?xml version="1.0" encoding="UTF-8"?>`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Declaration {
    pub version: String,
    pub encoding: Option<String>,
    pub standalone: Option<String>,
}

impl Default for Declaration {
    fn default() -> Declaration {
        Declaration {
            version: "1.0".to_string(),
            encoding: Some("UTF-8".to_string()),
            standalone: None,
        }
    }
}

/// What may stand outside the root element besides the XML declaration.
/// Saved untagged, so a plain string is a comment, as in workbooks that kept
/// only comments there.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
pub enum Misc {
    /// A comment: the text between `<!--` and `-->`.
    Comment(String),
    /// A processing instruction: the text between `<?` and `?>`, its target
    /// first.
    Pi { pi: String },
    /// The document type declaration: the text between `<!DOCTYPE ` and the
    /// closing `>`, as written. It is never expanded.
    DocType { doctype: String },
}

impl Misc {
    /// The text of a comment, or `None` for anything else.
    pub fn comment(&self) -> Option<&str> {
        match self {
            Misc::Comment(c) => Some(c),
            _ => None,
        }
    }
}

/// A document: its root and what sits around it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    pub declaration: Option<Declaration>,
    /// Comments, processing instructions and the document type declaration
    /// before the root element, in order.
    pub before: Vec<Misc>,
    pub root: Element,
    /// Comments and processing instructions after the root element.
    pub after: Vec<Misc>,
    /// What was dropped or changed on reading, for the import report.
    pub warnings: Vec<String>,
}

/// The text is not well-formed XML, or is outside what is accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XmlError {
    /// Byte offset into the text, when known.
    pub position: Option<usize>,
    pub message: String,
}

impl fmt::Display for XmlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.position {
            Some(at) => f.write_str(&tr!(
                "at byte {at}: {message}",
                at = at,
                message = self.message
            )),
            None => f.write_str(&self.message),
        }
    }
}

impl std::error::Error for XmlError {}

fn fail<T>(position: usize, message: impl Into<String>) -> Result<T, XmlError> {
    Err(XmlError {
        position: Some(position),
        message: message.into(),
    })
}

/// Splits `p:name` into the prefix and the local name.
fn split_name(qualified: &str) -> (Option<String>, String) {
    match qualified.split_once(':') {
        Some((prefix, local)) if !prefix.is_empty() && !local.is_empty() => {
            (Some(prefix.to_string()), local.to_string())
        }
        _ => (None, qualified.to_string()),
    }
}

/// A namespace in scope while reading.
struct Scope {
    declarations: Vec<(Option<String>, String)>,
}

fn lookup(scopes: &[Scope], prefix: Option<&str>) -> Option<String> {
    for scope in scopes.iter().rev() {
        for (p, uri) in scope.declarations.iter().rev() {
            if p.as_deref() == prefix {
                // `xmlns=""` takes the default namespace away.
                return (!uri.is_empty()).then(|| uri.clone());
            }
        }
    }
    match prefix {
        Some("xml") => Some("http://www.w3.org/XML/1998/namespace".to_string()),
        _ => None,
    }
}

/// Reads an XML document.
pub fn parse(text: &str) -> Result<Document, XmlError> {
    parse_with(text, false)
}

/// The element a fragment is read inside.
const FRAGMENT: &str = "gossia-fragment";

/// Reads a piece of XML content with no root of its own: text, elements and
/// comments, as the inside of an element of mixed content holds them.
/// Prefixes need not be declared, because the piece is read away from the
/// document it belongs in, where they are.
pub fn parse_fragment(text: &str) -> Result<Vec<Child>, XmlError> {
    let start = format!("<{FRAGMENT}>");
    let wrapped = format!("{start}{text}</{FRAGMENT}>");
    let doc = parse_with(&wrapped, true).map_err(|mut e| {
        e.position = e
            .position
            .map(|at| at.saturating_sub(start.len()).min(text.len()));
        e
    })?;
    if doc.root.name != FRAGMENT || doc.declaration.is_some() || !doc.after.is_empty() {
        return Err(XmlError {
            position: None,
            message: tr!("this is not a piece of XML content").to_string(),
        });
    }
    Ok(doc.root.children)
}

/// Writes a piece of XML content as [`parse_fragment`] reads it, on one line.
pub fn write_fragment(children: &[Child]) -> String {
    let mut out = String::new();
    for child in children {
        match child {
            Child::Text(t) => out.push_str(&escape_text(t)),
            Child::Comment(c) => out.push_str(&format!("<!--{c}-->")),
            Child::Element(e) => write_element(e, false, 0, &mut out),
        }
    }
    out
}

/// Reads a document; with `lenient`, a prefix that is not declared leaves its
/// element in no namespace instead of being refused.
fn parse_with(text: &str, lenient: bool) -> Result<Document, XmlError> {
    let mut reader = Reader::from_str(text);
    reader.config_mut().expand_empty_elements = true;
    let mut doc = Document {
        declaration: None,
        before: Vec::new(),
        root: Element::new(""),
        after: Vec::new(),
        warnings: Vec::new(),
    };
    // The elements that are open, and the namespaces they declare.
    let mut open: Vec<Element> = Vec::new();
    let mut scopes: Vec<Scope> = Vec::new();
    let mut root: Option<Element> = None;
    // Text being gathered: it is cut by markup, not by entity references.
    let mut text_run = String::new();

    loop {
        let at = usize::try_from(reader.buffer_position()).unwrap_or(0);
        let event = reader.read_event().map_err(|e| XmlError {
            position: Some(at),
            message: e.to_string(),
        })?;
        // Anything but text ends the run of text.
        if !matches!(
            event,
            Event::Text(_) | Event::CData(_) | Event::GeneralRef(_)
        ) && !text_run.is_empty()
        {
            let run = std::mem::take(&mut text_run);
            match open.last_mut() {
                Some(parent) => push_text(parent, run),
                None if run.trim().is_empty() => {}
                None => return fail(at, tr!("text outside the root element")),
            }
        }
        match event {
            Event::Decl(decl) => {
                if doc.declaration.is_some() || root.is_some() || !open.is_empty() {
                    return fail(at, tr!("the XML declaration must be first"));
                }
                let version = decl
                    .version()
                    .map_err(|e| XmlError {
                        position: Some(at),
                        message: e.to_string(),
                    })?
                    .into_owned();
                let attribute = |value: Option<Result<std::borrow::Cow<'_, str>, _>>| {
                    value
                        .and_then(|v: Result<std::borrow::Cow<'_, str>, _>| v.ok())
                        .map(|v| v.into_owned())
                };
                doc.declaration = Some(Declaration {
                    version,
                    encoding: attribute(decl.encoding()),
                    standalone: attribute(decl.standalone()),
                });
            }
            Event::DocType(d) => {
                let doctype = d.into_inner().trim().to_string();
                if doctype.contains('[') {
                    doc.warnings.push(
                        tr!(
                            "the document type declaration is kept as written, but entities \
                         declared in it are not expanded"
                        )
                        .to_string(),
                    );
                }
                doc.before.push(Misc::DocType { doctype });
            }
            Event::PI(pi) => {
                let pi = pi.into_inner().into_owned();
                match open.last() {
                    Some(_) => doc.warnings.push(
                        tr!("a processing instruction inside the root element was dropped")
                            .to_string(),
                    ),
                    None if root.is_none() => doc.before.push(Misc::Pi { pi }),
                    None => doc.after.push(Misc::Pi { pi }),
                }
            }
            Event::Comment(c) => {
                let comment = c.xml_content(XmlVersion::Explicit1_0).into_owned();
                match open.last_mut() {
                    Some(parent) => parent.children.push(Child::Comment(comment)),
                    None if root.is_none() => doc.before.push(Misc::Comment(comment)),
                    None => doc.after.push(Misc::Comment(comment)),
                }
            }
            Event::Start(start) => {
                if root.is_some() && open.is_empty() {
                    return fail(at, tr!("more than one root element"));
                }
                if open.len() >= MAX_DEPTH {
                    return fail(
                        at,
                        tr!(
                            "elements nested more than {MAX_DEPTH} levels deep are not supported",
                            MAX_DEPTH = MAX_DEPTH
                        ),
                    );
                }
                let mut element = start_element(&start, at)?;
                scopes.push(Scope {
                    declarations: element.declarations.clone(),
                });
                element.namespace = lookup(&scopes, element.prefix.as_deref());
                if element.prefix.is_some() && element.namespace.is_none() && !lenient {
                    return fail(
                        at,
                        tr!(
                            "the prefix `{prefix}` is not declared",
                            prefix = element.prefix.as_deref().unwrap_or_default()
                        ),
                    );
                }
                open.push(element);
            }
            Event::End(_) => {
                let Some(element) = open.pop() else {
                    return fail(at, tr!("a closing tag with nothing open"));
                };
                scopes.pop();
                match open.last_mut() {
                    Some(parent) => parent.children.push(Child::Element(element)),
                    None => root = Some(element),
                }
            }
            Event::Text(t) => {
                let content = t.xml_content(XmlVersion::Explicit1_0);
                text_run.push_str(&content);
            }
            Event::CData(c) => {
                let content = c.xml_content(XmlVersion::Explicit1_0);
                text_run.push_str(&content);
            }
            Event::GeneralRef(r) => {
                if let Some(ch) = r.resolve_char_ref().map_err(|e| XmlError {
                    position: Some(at),
                    message: e.to_string(),
                })? {
                    text_run.push(ch);
                } else {
                    text_run.push(match &*r {
                        "amp" => '&',
                        "lt" => '<',
                        "gt" => '>',
                        "quot" => '"',
                        "apos" => '\'',
                        other => {
                            return fail(
                                at,
                                tr!("the entity `&{other};` is not defined; only the five predefined entities and character references are", other = other),
                            );
                        }
                    });
                }
            }
            Event::Empty(_) => unreachable!("empty elements are expanded"),
            Event::Eof => break,
        }
    }
    if !open.is_empty() {
        return fail(text.len(), tr!("the document ends inside an element"));
    }
    let Some(root) = root else {
        return fail(text.len(), tr!("there is no root element"));
    };
    doc.root = root;
    Ok(doc)
}

/// Adds text to an element, joining it to the text before it.
fn push_text(parent: &mut Element, run: String) {
    if let Some(Child::Text(last)) = parent.children.last_mut() {
        last.push_str(&run);
    } else {
        parent.children.push(Child::Text(run));
    }
}

fn start_element(start: &BytesStart<'_>, at: usize) -> Result<Element, XmlError> {
    let name = start.name().as_ref().to_string();
    let (prefix, local) = split_name(&name);
    let mut element = Element {
        prefix,
        name: local,
        namespace: None,
        declarations: Vec::new(),
        attributes: Vec::new(),
        children: Vec::new(),
    };
    for attribute in start.attributes() {
        let attribute = attribute.map_err(|e| XmlError {
            position: Some(at),
            message: e.to_string(),
        })?;
        let key = attribute.key.as_ref().to_string();
        let value = attribute
            .normalized_value(XmlVersion::Explicit1_0)
            .map_err(|e| XmlError {
                position: Some(at),
                message: e.to_string(),
            })?
            .into_owned();
        if key == "xmlns" {
            element.declarations.push((None, value));
        } else if let Some(prefix) = key.strip_prefix("xmlns:") {
            element.declarations.push((Some(prefix.to_string()), value));
        } else {
            let (prefix, name) = split_name(&key);
            element.attributes.push(Attribute {
                prefix,
                name,
                value,
            });
        }
    }
    Ok(element)
}

// ----------------------------------------------------------------- writing

/// Escapes text content.
pub fn escape_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            // A carriage return would be read back as a line feed.
            '\r' => out.push_str("&#13;"),
            c => out.push(c),
        }
    }
    out
}

/// Escapes an attribute value, which is written in double quotes.
pub fn escape_attribute(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '"' => out.push_str("&quot;"),
            // Whitespace other than a space would be normalised away on reading.
            '\t' => out.push_str("&#9;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            c => out.push(c),
        }
    }
    out
}

fn qualified(prefix: &Option<String>, name: &str) -> String {
    match prefix {
        Some(p) => format!("{p}:{name}"),
        None => name.to_string(),
    }
}

/// Writes a document. With `pretty`, elements that hold only elements and
/// comments are indented by two spaces; an element that holds text is written
/// on one line with its text exactly as it is, and everything inside it too.
pub fn write(doc: &Document, pretty: bool) -> String {
    let mut out = String::new();
    if let Some(d) = &doc.declaration {
        out.push_str(&format!(
            "<?xml version=\"{}\"",
            escape_attribute(&d.version)
        ));
        if let Some(e) = &d.encoding {
            out.push_str(&format!(" encoding=\"{}\"", escape_attribute(e)));
        }
        if let Some(s) = &d.standalone {
            out.push_str(&format!(" standalone=\"{}\"", escape_attribute(s)));
        }
        out.push_str("?>");
        if pretty {
            out.push('\n');
        }
    }
    for misc in &doc.before {
        write_misc(misc, &mut out);
        if pretty {
            out.push('\n');
        }
    }
    write_element(&doc.root, pretty, 0, &mut out);
    for misc in &doc.after {
        if pretty {
            out.push('\n');
        }
        write_misc(misc, &mut out);
    }
    if pretty {
        out.push('\n');
    }
    out
}

fn write_misc(misc: &Misc, out: &mut String) {
    match misc {
        Misc::Comment(c) => out.push_str(&format!("<!--{c}-->")),
        Misc::Pi { pi } => out.push_str(&format!("<?{pi}?>")),
        Misc::DocType { doctype } => out.push_str(&format!("<!DOCTYPE {doctype}>")),
    }
}

fn write_element(element: &Element, pretty: bool, depth: usize, out: &mut String) {
    let name = qualified(&element.prefix, &element.name);
    out.push('<');
    out.push_str(&name);
    for (prefix, uri) in &element.declarations {
        match prefix {
            Some(p) => out.push_str(&format!(" xmlns:{p}=\"{}\"", escape_attribute(uri))),
            None => out.push_str(&format!(" xmlns=\"{}\"", escape_attribute(uri))),
        }
    }
    for a in &element.attributes {
        out.push_str(&format!(
            " {}=\"{}\"",
            qualified(&a.prefix, &a.name),
            escape_attribute(&a.value)
        ));
    }
    if element.children.is_empty() {
        out.push_str("/>");
        return;
    }
    out.push('>');
    let holds_text = element.children.iter().any(|c| matches!(c, Child::Text(_)));
    let indent = pretty && !holds_text;
    for child in &element.children {
        match child {
            Child::Text(t) => out.push_str(&escape_text(t)),
            Child::Comment(c) => {
                if indent {
                    newline(out, depth + 1);
                }
                out.push_str(&format!("<!--{c}-->"));
            }
            Child::Element(e) => {
                if indent {
                    newline(out, depth + 1);
                }
                // Inside text, any layout would change the text.
                write_element(e, indent, depth + 1, out);
            }
        }
    }
    if indent {
        newline(out, depth);
    }
    out.push_str(&format!("</{name}>"));
}

fn newline(out: &mut String, depth: usize) {
    out.push('\n');
    for _ in 0..depth {
        out.push_str("  ");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(text: &str) -> Element {
        parse(text).unwrap().root
    }

    #[test]
    fn elements_attributes_and_text_are_read_in_order() {
        let e = root("<a x=\"1\" y=\"2\"><b>one</b><c/>tail</a>");
        assert_eq!(e.name, "a");
        assert_eq!(e.attribute("x"), Some("1"));
        assert_eq!(e.attributes[1].name, "y");
        let names: Vec<_> = e.elements().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["b", "c"]);
        assert_eq!(e.text(), "tail");
    }

    #[test]
    fn the_declaration_and_comments_around_the_root_are_kept() {
        let d = parse(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><!--first--><r/><!--last-->",
        )
        .unwrap();
        let decl = d.declaration.unwrap();
        assert_eq!(decl.version, "1.0");
        assert_eq!(decl.encoding.as_deref(), Some("UTF-8"));
        assert_eq!(decl.standalone.as_deref(), Some("yes"));
        assert_eq!(d.before, [Misc::Comment("first".to_string())]);
        assert_eq!(d.after, [Misc::Comment("last".to_string())]);
    }

    #[test]
    fn predefined_entities_and_character_references_are_resolved_and_others_refused() {
        assert_eq!(
            root("<a>&lt;&amp;&#65;&#x42;&apos;&quot;&gt;</a>").text(),
            "<&AB'\">"
        );
        let e = parse("<a>&nbsp;</a>").unwrap_err();
        assert!(e.message.contains("nbsp"), "{e}");
    }

    #[test]
    fn cdata_becomes_ordinary_text_joined_to_the_text_around_it() {
        assert_eq!(root("<a>x<![CDATA[<y>]]>z</a>").text(), "x<y>z");
    }

    #[test]
    fn a_doctype_and_processing_instructions_outside_the_root_are_kept_and_never_expanded() {
        let d = parse("<!DOCTYPE a [<!ENTITY e \"boom\">]><?pi data?><a>&e;</a>");
        // The entity is not defined, because the declaration is not read.
        assert!(d.is_err());
        let text = "<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\"><?xml-stylesheet type=\"text/xsl\" href=\"s.xsl\"?><a><?inner x?></a><?tail y?>";
        let d = parse(text).unwrap();
        assert_eq!(
            d.before,
            [
                Misc::DocType {
                    doctype: "plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\"".to_string()
                },
                Misc::Pi {
                    pi: "xml-stylesheet type=\"text/xsl\" href=\"s.xsl\"".to_string()
                },
            ]
        );
        assert_eq!(
            d.after,
            [Misc::Pi {
                pi: "tail y".to_string()
            }]
        );
        // Only the one inside the root is dropped.
        assert_eq!(d.warnings.len(), 1, "{:?}", d.warnings);
        assert_eq!(write(&d, false), text.replace("<a><?inner x?></a>", "<a/>"));
        // An internal subset is kept, with a warning that it is not used.
        let d = parse("<!DOCTYPE a [<!ELEMENT a EMPTY>]><a/>").unwrap();
        assert_eq!(d.warnings.len(), 1);
        assert_eq!(write(&d, false), "<!DOCTYPE a [<!ELEMENT a EMPTY>]><a/>");
    }

    #[test]
    fn a_fragment_reads_and_writes_back_with_undeclared_prefixes() {
        let text = "Some <b>bold</b> and <x:y a=\"1\">odd</x:y><!--c--> &amp; more";
        let children = parse_fragment(text).unwrap();
        assert_eq!(children.len(), 6);
        assert_eq!(write_fragment(&children), text);
        assert!(parse_fragment("").unwrap().is_empty());
        assert_eq!(
            write_fragment(&parse_fragment("just text").unwrap()),
            "just text"
        );
        for bad in [
            "<b>open",
            "a < b",
            "</gossia-fragment><x>",
            "<?xml version=\"1.0\"?>",
        ] {
            assert!(parse_fragment(bad).is_err(), "{bad:?}");
        }
        let e = parse_fragment("ab<c>").unwrap_err();
        assert!(e.position.is_some_and(|at| at <= 5), "{e:?}");
    }

    #[test]
    fn namespaces_are_resolved_through_prefixes_and_the_default() {
        let e = root("<p:a xmlns:p=\"urn:one\" xmlns=\"urn:two\"><b/><p:c/></p:a>");
        assert_eq!(e.namespace.as_deref(), Some("urn:one"));
        assert_eq!(e.prefix.as_deref(), Some("p"));
        assert_eq!(e.declarations.len(), 2);
        let kids: Vec<_> = e.elements().collect();
        assert_eq!(kids[0].namespace.as_deref(), Some("urn:two"));
        assert_eq!(kids[1].namespace.as_deref(), Some("urn:one"));
        assert!(parse("<p:a/>").is_err(), "an undeclared prefix");
        let e = root("<a xmlns=\"urn:x\"><b xmlns=\"\"/></a>");
        assert_eq!(e.elements().next().unwrap().namespace, None);
    }

    #[test]
    fn what_is_not_well_formed_is_refused() {
        for bad in [
            "", "<a>", "<a></b>", "<a/><b/>", "text<a/>", "<a x=1/>", "</a>",
        ] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn deep_nesting_is_refused_without_overflowing_the_stack() {
        let deep = format!("{}{}", "<a>".repeat(100_000), "</a>".repeat(100_000));
        let e = parse(&deep).unwrap_err();
        assert!(e.message.contains("nested"), "{e}");
        let ok = format!("{}{}", "<a>".repeat(100), "</a>".repeat(100));
        assert!(parse(&ok).is_ok());
    }

    #[test]
    fn writing_reads_back_the_same_tree() {
        let text = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><!--c--><r xmlns=\"urn:x\" a=\"1 &amp; &quot;2&quot;\"><!--k--><b>t &lt; u</b><c/><d><e/></d></r><!--z-->";
        let d = parse(text).unwrap();
        for pretty in [false, true] {
            let again = parse(&write(&d, pretty)).unwrap();
            assert_eq!(again.declaration, d.declaration);
            assert_eq!(again.before, d.before);
            assert_eq!(again.after, d.after);
            // Indentation is only ever added between elements, so the trees agree
            // once whitespace-only text is ignored.
            assert_eq!(strip(again.root), strip(d.root.clone()));
        }
    }

    fn strip(mut e: Element) -> Element {
        e.children = e
            .children
            .into_iter()
            .filter_map(|c| match c {
                Child::Text(t) if t.trim().is_empty() => None,
                Child::Element(e) => Some(Child::Element(strip(e))),
                other => Some(other),
            })
            .collect();
        e
    }

    #[test]
    fn text_is_written_exactly_and_special_characters_survive() {
        let mut e = Element::new("a");
        e.attributes.push(Attribute {
            prefix: None,
            name: "v".into(),
            value: "tab\there\nline \"q\" <&>".into(),
        });
        e.children
            .push(Child::Text("  keep \r\n spaces & <tags>  ".into()));
        let doc = Document {
            declaration: None,
            before: vec![],
            root: e.clone(),
            after: vec![],
            warnings: vec![],
        };
        for pretty in [false, true] {
            let back = parse(&write(&doc, pretty)).unwrap().root;
            assert_eq!(back.attribute("v"), e.attribute("v"));
            // A carriage return is written as a reference, so it survives.
            assert_eq!(back.text(), e.text());
        }
    }

    #[test]
    fn pretty_output_indents_elements_and_keeps_text_on_one_line() {
        let d = parse("<a><b>x</b><c><d/></c></a>").unwrap();
        assert_eq!(
            write(&d, true),
            "<a>\n  <b>x</b>\n  <c>\n    <d/>\n  </c>\n</a>\n"
        );
        assert_eq!(write(&d, false), "<a><b>x</b><c><d/></c></a>");
    }
}
