//! The internal type description that both schema languages are lowered into.
//!
//! See DESIGN.md, "Schema model". Types are held by id in an arena, not inlined,
//! so a type can refer to itself (a tree of folders that contain folders).

use regex::Regex;

use crate::num::Num;

/// The id of a type within one [`TypeSpec`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TypeId(pub(crate) u32);

/// A lowered schema: an arena of types and the id of the root type.
#[derive(Debug, Clone)]
pub struct TypeSpec {
    types: Vec<TypeDef>,
    root: TypeId,
}

impl TypeSpec {
    pub(crate) fn new(types: Vec<TypeDef>, root: TypeId) -> TypeSpec {
        TypeSpec { types, root }
    }

    pub fn root(&self) -> TypeId {
        self.root
    }

    pub fn get(&self, id: TypeId) -> &TypeDef {
        &self.types[id.0 as usize]
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
#[derive(Debug, Clone)]
pub struct TypeDef {
    pub kind: Kind,
    /// Where in the schema document this type was written, for error messages.
    /// For JSON Schema this is a JSON pointer such as `#/$defs/Order`.
    pub source: String,
    /// The name of a reusable type, if it was declared with one.
    pub name: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
}

/// What a type is.
#[derive(Debug, Clone)]
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
}

/// A field of an object type.
#[derive(Debug, Clone)]
pub struct Field {
    pub name: String,
    pub ty: TypeId,
    pub required: bool,
}

/// Inclusive bounds. The bounds are kept as written, integer or float.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Range {
    pub min: Option<Num>,
    pub max: Option<Num>,
}

/// Constraints on a string. Lengths count Unicode code points.
#[derive(Debug, Clone, Default)]
pub struct StringConstraints {
    pub pattern: Option<Pattern>,
    pub min_length: Option<usize>,
    pub max_length: Option<usize>,
    pub format: Option<StringFormat>,
}

/// A string constraint that a string must be a valid date or date and time.
/// The node stays a string either way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StringFormat {
    /// `format: date`, an RFC 3339 full date (`2026-10-23`).
    Date,
    /// `format: date-time`, an RFC 3339 date and time (`2026-10-23T14:30:00Z`).
    DateTime,
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

impl PartialEq for Pattern {
    fn eq(&self, other: &Pattern) -> bool {
        self.source == other.source
    }
}
