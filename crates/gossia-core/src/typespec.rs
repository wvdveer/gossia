//! The internal type description that both schema languages are lowered into.
//!
//! See DESIGN.md, "Schema model". Types are held by id in an arena, not inlined,
//! so a type can refer to itself (a tree of folders that contain folders).

use std::sync::Arc;

use regex::Regex;

use crate::copybook::Layout;

use crate::num::Num;
use crate::path::{Path, Segment};

/// Where a path leads in a schema: see [`TypeSpec::resolve`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resolved {
    pub ty: TypeId,
    pub required: bool,
}

/// The id of a type within one [`TypeSpec`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct TypeId(pub(crate) u32);

/// A lowered schema: an arena of types and the id of the root type.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TypeSpec {
    types: Vec<TypeDef>,
    root: TypeId,
    xml: Option<XmlInfo>,
    /// For a schema lowered from a COBOL copybook, the bytes of a record.
    copybook: Option<Arc<Layout>>,
}

/// What a schema lowered from an XSD says about the XML document itself.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct XmlInfo {
    /// The name of the root element: the one global element.
    pub root: String,
    /// The target namespace, if the schema has one. The root element is in it.
    pub target_namespace: Option<String>,
    /// Whether local elements are in the target namespace too
    /// (`elementFormDefault="qualified"`).
    pub qualified: bool,
    /// The prefixes of the fields outside the target namespace, with their
    /// namespaces, so that writing can declare them.
    #[serde(default)]
    pub prefixes: Vec<(String, String)>,
}

impl TypeSpec {
    pub(crate) fn new(types: Vec<TypeDef>, root: TypeId) -> TypeSpec {
        TypeSpec {
            types,
            root,
            xml: None,
            copybook: None,
        }
    }

    pub(crate) fn with_xml(mut self, xml: XmlInfo) -> TypeSpec {
        self.xml = Some(xml);
        self
    }

    pub(crate) fn with_copybook(mut self, layout: Layout) -> TypeSpec {
        self.copybook = Some(Arc::new(layout));
        self
    }

    /// The same types with `ty` as the root: the schema of one node of this
    /// one's data. For an XSD, `element` names the node's element, which is
    /// then the root element; a local element is in the target namespace
    /// only when the schema qualifies local elements.
    pub fn rooted_at(&self, ty: TypeId, element: Option<&str>) -> TypeSpec {
        let mut spec = self.clone();
        spec.root = ty;
        if let (Some(xml), Some(name)) = (&mut spec.xml, element) {
            xml.root = name.to_owned();
            if !xml.qualified {
                xml.target_namespace = None;
            }
        }
        spec
    }

    /// For a schema lowered from a copybook, how its records are laid out.
    pub fn copybook(&self) -> Option<&Layout> {
        self.copybook.as_deref()
    }

    /// For a schema lowered from an XSD, how its documents are put together.
    pub fn xml(&self) -> Option<&XmlInfo> {
        self.xml.as_ref()
    }

    pub fn root(&self) -> TypeId {
        self.root
    }

    pub fn get(&self, id: TypeId) -> &TypeDef {
        &self.types[id.0 as usize]
    }

    /// The type of the node at `path`, and whether the schema requires that
    /// node to exist. `None` if the schema defines no node there (an unknown
    /// field, or an index into something that is not an array). The root is
    /// required. An array item is required, since it exists once it is added.
    pub fn resolve(&self, path: &Path) -> Option<Resolved> {
        let mut here = Resolved {
            ty: self.root,
            required: true,
        };
        for segment in path.segments() {
            here = match (&self.get(here.ty).kind, segment) {
                (Kind::Object { fields }, Segment::Key(name)) => {
                    let field = fields.iter().find(|f| &f.name == name)?;
                    Resolved {
                        ty: field.ty,
                        required: field.required,
                    }
                }
                (Kind::Map { value }, Segment::Key(_)) => Resolved {
                    ty: *value,
                    required: true,
                },
                (Kind::Array { items, .. }, Segment::Index(_)) => Resolved {
                    ty: *items,
                    required: true,
                },
                // Inside an `any`, everything is `any`.
                (Kind::Any, segment) => Resolved {
                    ty: here.ty,
                    required: matches!(segment, Segment::Index(_)),
                },
                _ => return None,
            };
        }
        Some(here)
    }

    /// Whether `path` is a field of an object that the schema declares and
    /// does not require. A node missing at such a path is simply absent, which
    /// is different from a node the schema does not define.
    pub fn is_optional_field(&self, path: &Path) -> bool {
        let Some((Segment::Key(name), parent)) = path.segments().split_last() else {
            return false;
        };
        let parent = Path::from(parent.to_vec());
        let Some(here) = self.resolve(&parent) else {
            return false;
        };
        match &self.get(here.ty).kind {
            Kind::Object { fields } => fields.iter().any(|f| &f.name == name && !f.required),
            _ => false,
        }
    }

    /// A short name for a type, for display: `integer`, `array of object`,
    /// `map of number`, or the name a type was declared with.
    pub fn describe(&self, ty: TypeId) -> String {
        let def = self.get(ty);
        if let Some(name) = &def.name {
            return name.clone();
        }
        match &def.kind {
            Kind::String(_) => "string".to_owned(),
            Kind::Integer(_) => "integer".to_owned(),
            Kind::Number(_) => "number".to_owned(),
            Kind::Bool => "boolean".to_owned(),
            Kind::Date => "date".to_owned(),
            Kind::DateTime => "dateTime".to_owned(),
            Kind::Enum { .. } => "string (choice)".to_owned(),
            Kind::Object { .. } => "object".to_owned(),
            Kind::Array { items, .. } => format!("array of {}", self.describe(*items)),
            Kind::Map { value } => format!("map of {}", self.describe(*value)),
            Kind::Any => "any".to_owned(),
        }
    }

    /// The number of types in the arena, including any that are not reachable.
    pub fn len(&self) -> usize {
        self.types.len()
    }

    pub fn is_empty(&self) -> bool {
        self.types.is_empty()
    }
}

/// One type, with where it came from.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TypeDef {
    pub kind: Kind,
    /// Where in the schema document this type was written, for error messages.
    /// For JSON Schema this is a JSON pointer such as `#/$defs/Order`.
    pub source: String,
    /// The name of a reusable type, if it was declared with one.
    pub name: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    /// Values the source names, offered as choices without limiting the
    /// type: the condition names (level 88) of a COBOL item, with the value
    /// each stands for.
    pub choices: Vec<(crate::value::Value, String)>,
}

/// What a type is.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum Kind {
    String(StringConstraints),
    Integer(Range),
    Number(Range),
    Bool,
    /// An XML `date`. Only XSD produces it.
    Date,
    /// An XML `dateTime`. Only XSD produces it.
    DateTime,
    Array {
        items: TypeId,
        min_items: Option<usize>,
        max_items: Option<usize>,
    },
    /// A closed record: exactly these fields, in this order.
    Object {
        fields: Vec<Field>,
    },
    /// A string-keyed dictionary with one value type. JSON Schema only.
    Map {
        value: TypeId,
    },
    /// A string with a fixed set of allowed values (and optionally more
    /// constraints on the string).
    Enum {
        values: Vec<String>,
        strings: StringConstraints,
    },
    /// Any value at all: text, a number, a boolean, a record or a list, with
    /// anything inside. A JSON Schema with no `type` (owner, 2026-11-06); the
    /// Type column calls it `any`. What is inside an `any` is `any` too.
    Any,
}

impl Kind {
    /// Whether values of this kind are markup: the inside of an XML element
    /// of mixed content, kept as written ([`StringFormat::Markup`]).
    pub fn is_markup(&self) -> bool {
        matches!(self, Kind::String(c) if c.format == Some(StringFormat::Markup))
    }
}

/// A field of an object type.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Field {
    pub name: String,
    pub ty: TypeId,
    pub required: bool,
    /// For a field of an XSD, the namespace of its element or attribute
    /// (none for an unqualified one). A field outside the schema's own
    /// namespace has the prefix in its name: `dc:title`, `@xml:lang`.
    #[serde(default)]
    pub namespace: Option<String>,
}

/// Inclusive bounds. The bounds are kept as written, integer or float.
#[derive(Debug, Clone, Copy, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Range {
    pub min: Option<Num>,
    pub max: Option<Num>,
}

/// Constraints on a string. Lengths count Unicode code points.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct StringConstraints {
    pub pattern: Option<Pattern>,
    pub min_length: Option<usize>,
    pub max_length: Option<usize>,
    pub format: Option<StringFormat>,
}

/// A string constraint that a string must be a valid date or date and time.
/// The node stays a string either way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum StringFormat {
    /// `format: date`, an RFC 3339 full date (`2026-10-23`).
    Date,
    /// `format: date-time`, an RFC 3339 date and time (`2026-10-23T14:30:00Z`).
    DateTime,
    /// A piece of XML content written as it is: the inside of an XSD element
    /// of mixed content (`mixed="true"` with an `xs:any` that is skipped).
    /// Only XSD produces it. The string holds text, elements and comments
    /// and must be well-formed; prefixes in it are those of the document.
    Markup,
}

/// A regular expression, kept with its source text. It matches anywhere in the
/// string unless it is anchored, as JSON Schema and XSD (after the implicit
/// anchoring that XSD lowering applies) require.
#[derive(Debug, Clone)]
pub struct Pattern {
    source: String,
    regex: Regex,
}

impl Pattern {
    pub fn new(source: &str) -> Result<Pattern, regex::Error> {
        Ok(Pattern {
            source: source.to_string(),
            regex: Regex::new(source)?,
        })
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn is_match(&self, text: &str) -> bool {
        self.regex.is_match(text)
    }
}

/// A pattern travels as its source text and is compiled again on arrival.
impl serde::Serialize for Pattern {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.source)
    }
}

impl<'de> serde::Deserialize<'de> for Pattern {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Pattern, D::Error> {
        let source = String::deserialize(deserializer)?;
        Pattern::new(&source).map_err(serde::de::Error::custom)
    }
}

impl PartialEq for Pattern {
    fn eq(&self, other: &Pattern) -> bool {
        self.source == other.source
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jsonschema::lower_str;

    const SCHEMA: &str = r##"{
      "type": "object",
      "properties": {
        "name": {"type": "string"},
        "tags": {"type": "array", "items": {"type": "string"}},
        "prices": {"type": "object", "additionalProperties": {"type": "number"}},
        "address": {"type": "object", "properties": {"zip": {"type": "string"}}}
      },
      "required": ["name"]
    }"##;

    fn resolved(spec: &TypeSpec, path: &str) -> Option<(String, bool)> {
        let path: Path = if path.is_empty() {
            Path::root()
        } else {
            path.parse().unwrap()
        };
        spec.resolve(&path).map(|r| {
            let kind = match &spec.get(r.ty).kind {
                Kind::String(_) => "string",
                Kind::Number(_) => "number",
                Kind::Array { .. } => "array",
                Kind::Object { .. } => "object",
                Kind::Map { .. } => "map",
                _ => "other",
            };
            (kind.to_string(), r.required)
        })
    }

    #[test]
    fn resolve_follows_a_path_through_the_schema() {
        let spec = lower_str(SCHEMA).unwrap();
        assert_eq!(resolved(&spec, ""), Some(("object".into(), true)));
        assert_eq!(resolved(&spec, "name"), Some(("string".into(), true)));
        assert_eq!(resolved(&spec, "tags"), Some(("array".into(), false)));
        assert_eq!(resolved(&spec, "tags[7]"), Some(("string".into(), true)));
        assert_eq!(
            resolved(&spec, "prices.rice"),
            Some(("number".into(), true))
        );
        assert_eq!(
            resolved(&spec, "address.zip"),
            Some(("string".into(), false))
        );
    }

    #[test]
    fn an_optional_field_is_told_apart_from_an_undefined_one_and_from_a_map_key() {
        let spec = lower_str(SCHEMA).unwrap();
        let optional = |text: &str| spec.is_optional_field(&text.parse().unwrap());
        assert!(optional("tags"));
        assert!(optional("address.zip"));
        assert!(!optional("name"), "required");
        assert!(!optional("nothing"), "not defined");
        assert!(
            !optional("prices.rice"),
            "a missing map key is a missing key"
        );
        assert!(!optional("tags[0]"));
        assert!(!spec.is_optional_field(&Path::root()));
    }

    #[test]
    fn describe_gives_a_short_name_for_display() {
        let spec = lower_str(SCHEMA).unwrap();
        let at = |text: &str| -> String {
            let path: Path = if text.is_empty() {
                Path::root()
            } else {
                text.parse().unwrap()
            };
            spec.describe(spec.resolve(&path).unwrap().ty)
        };
        assert_eq!(at(""), "object");
        assert_eq!(at("name"), "string");
        assert_eq!(at("tags"), "array of string");
        assert_eq!(at("prices"), "map of number");
    }

    #[test]
    fn resolve_finds_nothing_where_the_schema_defines_no_node() {
        let spec = lower_str(SCHEMA).unwrap();
        assert_eq!(resolved(&spec, "nothing"), None);
        assert_eq!(resolved(&spec, "name.first"), None);
        assert_eq!(resolved(&spec, "name[0]"), None);
        assert_eq!(resolved(&spec, "tags.first"), None);
        assert_eq!(resolved(&spec, "address[0]"), None);
    }

    /// The worker of the wasm version receives schemas encoded (DESIGN.md,
    /// Web port, 0.3.0): a pattern arrives as its text and is compiled again.
    #[test]
    fn a_pattern_is_encoded_as_its_text_and_compiled_again() {
        let pattern = Pattern::new("^[A-Z]{3}$").unwrap();
        let text = serde_json::to_string(&pattern).unwrap();
        assert_eq!(text, r#""^[A-Z]{3}$""#);
        let back: Pattern = serde_json::from_str(&text).unwrap();
        assert!(back.is_match("EUR"));
        assert!(!back.is_match("euro"));
        assert!(serde_json::from_str::<Pattern>(r#""(?=x)""#).is_err());
    }
}
