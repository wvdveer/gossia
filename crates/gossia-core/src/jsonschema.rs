//! Lowering JSON Schema (the v1 subset) into a [`TypeSpec`].
//!
//! See DESIGN.md, "Schema subsets (v1)". The subset is closed record types,
//! arrays, and simple types with a few restrictions. Anything outside it fails
//! here with an error that names the keyword and where it was written. Nothing
//! is silently ignored, and unknown keywords are errors too, so typos show.
//!
//! Decisions that refine the design (recorded in DESIGN.md under "Decisions
//! made while building step 1"):
//! - `type` is required, except on a schema that is only a `$ref` or only an
//!   `enum` (which is a string enumeration).
//! - A keyword that does not apply to the schema's type (`minimum` on a string)
//!   is an error, not ignored.
//! - `additionalProperties: false` is accepted as the explicit form of what
//!   objects already are (closed); `true` is open content and is refused.
//! - `$ref` is `#` or `#/$defs/Name`, and `$defs` is read at the root only. A
//!   `$ref` may carry `title` and `description` beside it and nothing else.
//! - The root `$schema` may be given if it names draft 2020-12.
//! - A pattern must be accepted by the `regex` crate, which lacks look-around
//!   and back-references.

use gossia_i18n::tr;
use std::collections::{HashMap, HashSet};
use std::fmt;

use indexmap::IndexMap;

use crate::json;
use crate::num::Num;
use crate::typespec::{
    Field, Kind, Pattern, Range, StringConstraints, StringFormat, TypeDef, TypeId, TypeSpec,
};
use crate::value::{Node, Value};

/// The meta-schema URI that a root `$schema` may name.
const SCHEMA_URI: &str = "https://json-schema.org/draft/2020-12/schema";

/// How deeply schemas may nest before lowering gives up.
const MAX_DEPTH: usize = 128;

/// Keywords the subset accepts.
const SUPPORTED: &[&str] = &[
    "type",
    "properties",
    "required",
    "items",
    "minItems",
    "maxItems",
    "minimum",
    "maximum",
    "minLength",
    "maxLength",
    "pattern",
    "enum",
    "format",
    "additionalProperties",
    "$ref",
    "$defs",
    "$schema",
    "title",
    "description",
];

/// Why a known JSON Schema keyword is refused, or `None` if the keyword is not
/// one that this project knows.
fn excluded_reason(keyword: &str) -> Option<&'static str> {
    Some(match keyword {
        "oneOf" | "anyOf" | "allOf" | "not" => {
            tr!("unions and combinators are not supported in version 1")
        }
        "if" | "then" | "else" | "dependentSchemas" | "dependentRequired" | "dependencies" => {
            tr!("conditional schemas are not supported in version 1")
        }
        "patternProperties" | "unevaluatedProperties" | "unevaluatedItems" | "propertyNames" => {
            tr!("open content is not supported in version 1")
        }
        "$id" | "$anchor" | "$dynamicRef" | "$dynamicAnchor" | "$recursiveRef"
        | "$recursiveAnchor" | "$vocabulary" => {
            tr!("identifiers, anchors, and non-local references are not supported in version 1")
        }
        "const" | "exclusiveMinimum" | "exclusiveMaximum" | "multipleOf" | "uniqueItems"
        | "prefixItems" | "contains" | "minContains" | "maxContains" | "minProperties"
        | "maxProperties" => tr!("this keyword is not supported in version 1"),
        "$comment" | "default" | "examples" | "deprecated" | "readOnly" | "writeOnly"
        | "contentEncoding" | "contentMediaType" | "contentSchema" => {
            tr!("only `title` and `description` are accepted as annotations")
        }
        _ => return None,
    })
}

/// A schema that the subset cannot take, with where and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaError {
    /// The keyword the error is about.
    pub keyword: String,
    /// Where the schema was written, as a JSON pointer such as `#/properties/a`.
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
    NotJson(json::ParseError),
    Schema(SchemaError),
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::NotJson(e) => f.write_str(&tr!("the schema is not valid JSON: {e}", e = e)),
            LoadError::Schema(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for LoadError {}

/// Reads JSON Schema text and lowers it.
pub fn lower_str(text: &str) -> Result<TypeSpec, LoadError> {
    let parsed = json::parse(text).map_err(LoadError::NotJson)?;
    lower(&parsed.node).map_err(LoadError::Schema)
}

/// Lowers a schema document held as a data tree (a schema sheet's content).
pub fn lower(schema: &Node) -> Result<TypeSpec, SchemaError> {
    let mut lowerer = Lowerer {
        root: schema.value(),
        types: Vec::new(),
        refs: HashMap::new(),
        aliasing: HashSet::new(),
        depth: 0,
    };
    lowerer.run()
}

type Res<T> = Result<T, SchemaError>;

fn err<T>(keyword: &str, location: &str, message: impl Into<String>) -> Res<T> {
    Err(SchemaError {
        keyword: keyword.to_string(),
        location: location.to_string(),
        message: message.into(),
    })
}

/// Escapes a key for use in a JSON pointer.
fn escape(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}

fn unescape(segment: &str) -> String {
    segment.replace("~1", "/").replace("~0", "~")
}

fn child_ptr(ptr: &str, keys: &[&str]) -> String {
    let mut out = ptr.to_string();
    for key in keys {
        out.push('/');
        out.push_str(&escape(key));
    }
    out
}

/// Checks every schema object reachable through `properties`, `$defs`,
/// `items`, and `additionalProperties` for keywords outside the subset, and
/// for `$defs` and `$schema` anywhere but the root. Reports the first found.
fn scan(schema: &Value, ptr: &str, depth: usize) -> Res<()> {
    let Some(obj) = schema.as_object() else {
        // Not an object: lowering reports it, with better context.
        return Ok(());
    };
    if depth >= MAX_DEPTH {
        return err(
            "type",
            ptr,
            tr!(
                "schemas nested more than {MAX_DEPTH} levels deep are not supported",
                MAX_DEPTH = MAX_DEPTH
            ),
        );
    }
    for key in obj.keys() {
        if !SUPPORTED.contains(&key.as_str()) {
            return match excluded_reason(key) {
                Some(reason) => err(key, ptr, reason),
                None => err(
                    key,
                    ptr,
                    tr!("is not a keyword that this version recognises"),
                ),
            };
        }
        if ptr != "#" && (key == "$defs" || key == "$schema") {
            return err(key, ptr, tr!("is only allowed at the root of the schema"));
        }
    }
    for (key, node) in obj {
        match key.as_str() {
            "properties" | "$defs" => {
                if let Some(children) = node.value().as_object() {
                    for (name, child) in children {
                        scan(child.value(), &child_ptr(ptr, &[key, name]), depth + 1)?;
                    }
                }
            }
            "items" | "additionalProperties" => {
                scan(node.value(), &child_ptr(ptr, &[key]), depth + 1)?;
            }
            _ => {}
        }
    }
    Ok(())
}

/// The type a schema declares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ty {
    String,
    Integer,
    Number,
    Boolean,
    Array,
    Object,
    /// No `type`: any value.
    Any,
}

impl Ty {
    fn name(self) -> &'static str {
        match self {
            Ty::String => "string",
            Ty::Integer => "integer",
            Ty::Number => "number",
            Ty::Boolean => "boolean",
            Ty::Array => "array",
            Ty::Object => "object",
            Ty::Any => "any",
        }
    }

    /// The keywords that mean something for this type, besides the common ones.
    fn keywords(self) -> &'static [&'static str] {
        match self {
            Ty::String => &["pattern", "minLength", "maxLength", "format", "enum"],
            Ty::Integer | Ty::Number => &["minimum", "maximum"],
            Ty::Boolean => &[],
            Ty::Array => &["items", "minItems", "maxItems"],
            Ty::Object => &["properties", "required", "additionalProperties"],
            Ty::Any => &[],
        }
    }
}

/// Keywords that are valid on any schema.
const COMMON: &[&str] = &["type", "title", "description", "$ref", "$defs", "$schema"];

/// The keywords of a type other than `ty` (a `type` name) that a schema of
/// type `ty` cannot have: `properties` once a schema is a `string`, say.
/// None for a name that is not a type.
pub fn keywords_not_for(ty: &str) -> Option<Vec<&'static str>> {
    let all = [
        Ty::String,
        Ty::Integer,
        Ty::Number,
        Ty::Boolean,
        Ty::Array,
        Ty::Object,
    ];
    let own = all.iter().find(|t| t.name() == ty)?.keywords();
    let mut others: Vec<&'static str> = Vec::new();
    for keyword in all.iter().flat_map(|t| t.keywords()) {
        if !own.contains(keyword) && !others.contains(keyword) {
            others.push(keyword);
        }
    }
    Some(others)
}

struct Lowerer<'a> {
    root: &'a Value,
    types: Vec<TypeDef>,
    /// Targets (`#`, `#/$defs/Name`) already resolved, or being resolved.
    refs: HashMap<String, TypeId>,
    /// Targets that are plain aliases of another target, being followed. A
    /// target met again while it is in this set is a cycle with no structure.
    aliasing: HashSet<String>,
    depth: usize,
}

impl<'a> Lowerer<'a> {
    fn run(&mut self) -> Res<TypeSpec> {
        let root = self.root;
        let Some(obj) = root.as_object() else {
            return err(
                "$schema",
                "#",
                tr!(
                    "a schema must be an object, not {kind}",
                    kind = root.kind_name()
                ),
            );
        };
        // Unsupported keywords anywhere in the document come first, so that
        // `oneOf` is reported as `oneOf` and not as a missing `type` beside it.
        scan(root, "#", 0)?;
        if let Some(node) = obj.get("$schema") {
            match node.value().as_str() {
                Some(uri) if uri.trim_end_matches('#') == SCHEMA_URI => {}
                _ => {
                    return err(
                        "$schema",
                        "#",
                        tr!(
                            "only draft 2020-12 ({SCHEMA_URI}) is supported",
                            SCHEMA_URI = SCHEMA_URI
                        ),
                    );
                }
            }
        }
        let mut def_names = Vec::new();
        if let Some(node) = obj.get("$defs") {
            let Some(defs) = node.value().as_object() else {
                return err("$defs", "#", tr!("`$defs` must be an object"));
            };
            for (name, def) in defs {
                if def.value().as_object().is_none() {
                    return err(
                        "$defs",
                        &child_ptr("#", &["$defs", name]),
                        tr!("a definition must be a schema object"),
                    );
                }
                def_names.push(name.clone());
            }
        }
        let root_id = self.resolve_ref("#", "#")?;
        // Lower every definition, used or not, so that mistakes in them surface.
        for name in def_names {
            self.resolve_ref(&child_ptr("#", &["$defs", &name]), "#")?;
        }
        Ok(TypeSpec::new(std::mem::take(&mut self.types), root_id))
    }

    fn alloc(&mut self) -> TypeId {
        self.types.push(TypeDef {
            kind: Kind::Bool,
            source: String::new(),
            name: None,
            title: None,
            description: None,
            choices: Vec::new(),
        });
        TypeId(u32::try_from(self.types.len() - 1).expect("fewer than 2^32 types"))
    }

    /// Finds the schema a `$ref` target names.
    fn find(&self, target: &str, at: &str) -> Res<&'a Value> {
        let root: &'a Value = self.root;
        if target == "#" {
            return Ok(root);
        }
        let Some(name) = target.strip_prefix("#/$defs/") else {
            return err(
                "$ref",
                at,
                tr!(
                    "`{target}` is not supported: use `#` or `#/$defs/Name`",
                    target = target
                ),
            );
        };
        if name.contains('/') {
            return err(
                "$ref",
                at,
                tr!(
                    "`{target}` is not supported: use `#` or `#/$defs/Name`",
                    target = target
                ),
            );
        }
        let name = unescape(name);
        root.as_object()
            .and_then(|o| o.get("$defs"))
            .and_then(|d| d.value().as_object())
            .and_then(|d| d.get(&name))
            .map(Node::value)
            .map_or_else(
                || {
                    err(
                        "$ref",
                        at,
                        tr!(
                            "there is no definition named `{name}` in `$defs`",
                            name = name
                        ),
                    )
                },
                Ok,
            )
    }

    /// Resolves a `$ref` target to a type, lowering it the first time.
    fn resolve_ref(&mut self, target: &str, at: &str) -> Res<TypeId> {
        if let Some(&id) = self.refs.get(target) {
            return Ok(id);
        }
        let schema = self.find(target, at)?;
        let location = target.to_string();
        if let Some(obj) = schema.as_object().filter(|o| o.contains_key("$ref")) {
            // A plain alias: the same type as what it refers to.
            self.check_ref_siblings(obj, &location)?;
            let next = ref_target(obj, &location)?;
            if !self.aliasing.insert(target.to_string()) {
                return err(
                    "$ref",
                    &location,
                    tr!("references form a cycle with no structure in it"),
                );
            }
            let id = self.resolve_ref(&next, &location);
            self.aliasing.remove(target);
            let id = id?;
            self.refs.insert(target.to_string(), id);
            return Ok(id);
        }
        let id = self.alloc();
        self.refs.insert(target.to_string(), id);
        let mut def = self.lower_schema(schema, &location)?;
        if let Some(name) = target.strip_prefix("#/$defs/") {
            def.name = Some(unescape(name));
        }
        self.types[id.0 as usize] = def;
        Ok(id)
    }

    fn check_ref_siblings(&self, obj: &IndexMap<String, Node>, ptr: &str) -> Res<()> {
        for key in obj.keys() {
            let root_only = ptr == "#" && (key == "$defs" || key == "$schema");
            if !(key == "$ref" || key == "title" || key == "description" || root_only) {
                return err(
                    key,
                    ptr,
                    tr!(
                        "cannot be combined with `$ref`; only `title` and `description` may sit beside it"
                    ),
                );
            }
        }
        Ok(())
    }

    /// Lowers a subschema (the value of `properties.x`, `items`, ...) to a type.
    fn lower_sub(&mut self, schema: &'a Value, ptr: &str) -> Res<TypeId> {
        if self.depth >= MAX_DEPTH {
            return err(
                "type",
                ptr,
                tr!(
                    "schemas nested more than {MAX_DEPTH} levels deep are not supported",
                    MAX_DEPTH = MAX_DEPTH
                ),
            );
        }
        let Some(obj) = schema.as_object() else {
            return err(
                "type",
                ptr,
                tr!(
                    "a schema must be an object, not {kind}",
                    kind = schema.kind_name()
                ),
            );
        };
        self.depth += 1;
        let result = if obj.contains_key("$ref") {
            self.check_ref_siblings(obj, ptr)
                .and_then(|()| ref_target(obj, ptr))
                .and_then(|target| self.resolve_ref(&target, ptr))
        } else {
            self.lower_schema(schema, ptr).map(|def| {
                self.types.push(def);
                TypeId(u32::try_from(self.types.len() - 1).expect("fewer than 2^32 types"))
            })
        };
        self.depth -= 1;
        result
    }

    /// Lowers one schema object that has no `$ref`.
    fn lower_schema(&mut self, schema: &'a Value, ptr: &str) -> Res<TypeDef> {
        let obj = schema.as_object().expect("callers pass objects");
        let ty = declared_type(obj, ptr)?;
        for key in obj.keys() {
            if !COMMON.contains(&key.as_str()) && !ty.keywords().contains(&key.as_str()) {
                return err(
                    key,
                    ptr,
                    match ty {
                        Ty::Any => {
                            tr!("does not apply to a schema with no `type` (any value)").to_owned()
                        }
                        _ => tr!("does not apply to a schema of type `{ty}`", ty = ty.name()),
                    },
                );
            }
        }
        let kind = match ty {
            Ty::String => {
                let strings = self.string_constraints(obj, ptr)?;
                match obj.get("enum") {
                    Some(node) => Kind::Enum {
                        values: enum_values(node.value(), ptr)?,
                        strings,
                    },
                    None => Kind::String(strings),
                }
            }
            Ty::Integer => Kind::Integer(range(obj, ptr)?),
            Ty::Number => Kind::Number(range(obj, ptr)?),
            Ty::Boolean => Kind::Bool,
            Ty::Any => Kind::Any,
            Ty::Array => {
                let Some(items) = obj.get("items") else {
                    return err(
                        "items",
                        ptr,
                        tr!("an array needs `items`, a schema for its elements"),
                    );
                };
                if items.value().as_object().is_none() {
                    return err("items", ptr, tr!("`items` must be a single schema"));
                }
                let items = self.lower_sub(items.value(), &child_ptr(ptr, &["items"]))?;
                Kind::Array {
                    items,
                    min_items: count(obj, "minItems", ptr)?,
                    max_items: count(obj, "maxItems", ptr)?,
                }
            }
            Ty::Object => self.object_kind(obj, ptr)?,
        };
        Ok(TypeDef {
            kind,
            source: ptr.to_string(),
            name: None,
            title: annotation(obj, "title", ptr)?,
            description: annotation(obj, "description", ptr)?,
            choices: Vec::new(),
        })
    }

    fn string_constraints(
        &self,
        obj: &IndexMap<String, Node>,
        ptr: &str,
    ) -> Res<StringConstraints> {
        let pattern = match obj.get("pattern") {
            None => None,
            Some(node) => {
                let Some(source) = node.value().as_str() else {
                    return err("pattern", ptr, tr!("`pattern` must be a string"));
                };
                match Pattern::new(source) {
                    Ok(p) => Some(p),
                    Err(e) => {
                        return err(
                            "pattern",
                            ptr,
                            tr!("this regular expression is not supported: {e}", e = e),
                        );
                    }
                }
            }
        };
        let format = match obj.get("format").map(|n| n.value().as_str()) {
            None => None,
            Some(Some("date")) => Some(StringFormat::Date),
            Some(Some("date-time")) => Some(StringFormat::DateTime),
            Some(_) => {
                return err(
                    "format",
                    ptr,
                    tr!("only the formats `date` and `date-time` are supported"),
                );
            }
        };
        Ok(StringConstraints {
            pattern,
            min_length: count(obj, "minLength", ptr)?,
            max_length: count(obj, "maxLength", ptr)?,
            format,
        })
    }

    fn object_kind(&mut self, obj: &'a IndexMap<String, Node>, ptr: &str) -> Res<Kind> {
        let properties = match obj.get("properties") {
            None => None,
            Some(node) => match node.value().as_object() {
                Some(props) => Some(props),
                None => return err("properties", ptr, tr!("`properties` must be an object")),
            },
        };
        match obj.get("additionalProperties").map(Node::value) {
            Some(Value::Object(_)) => {
                if properties.is_some() {
                    return err(
                        "additionalProperties",
                        ptr,
                        tr!(
                            "an object has either `properties` or a schema-valued `additionalProperties`, not both"
                        ),
                    );
                }
                if obj.contains_key("required") {
                    return err(
                        "required",
                        ptr,
                        tr!("does not apply to a map (an object with `additionalProperties`)"),
                    );
                }
                let value = self.lower_sub(
                    obj.get("additionalProperties")
                        .expect("just matched")
                        .value(),
                    &child_ptr(ptr, &["additionalProperties"]),
                )?;
                return Ok(Kind::Map { value });
            }
            // Objects are closed already, so `false` only says it out loud.
            Some(Value::Bool(false)) | None => {}
            Some(Value::Bool(true)) => {
                return err(
                    "additionalProperties",
                    ptr,
                    tr!("`true` would allow any extra content; objects are closed"),
                );
            }
            Some(_) => {
                return err(
                    "additionalProperties",
                    ptr,
                    tr!("must be a schema or `false`"),
                );
            }
        }
        let required = required_names(obj, ptr)?;
        let Some(properties) = properties else {
            if obj.get("additionalProperties").is_some() && required.is_empty() {
                return Ok(Kind::Object { fields: Vec::new() });
            }
            return err(
                "properties",
                ptr,
                tr!("an object needs `properties` (or a schema-valued `additionalProperties`)"),
            );
        };
        for name in &required {
            if !properties.contains_key(name) {
                return err(
                    "required",
                    ptr,
                    tr!(
                        "`{name}` is required but is not one of the `properties`",
                        name = name
                    ),
                );
            }
        }
        let mut fields = Vec::with_capacity(properties.len());
        for (name, node) in properties {
            let ty = self.lower_sub(node.value(), &child_ptr(ptr, &["properties", name]))?;
            fields.push(Field {
                name: name.clone(),
                ty,
                required: required.contains(name),
                namespace: None,
            });
        }
        Ok(Kind::Object { fields })
    }
}

/// The `$ref` target as text.
fn ref_target(obj: &IndexMap<String, Node>, ptr: &str) -> Res<String> {
    match obj.get("$ref").and_then(|n| n.value().as_str()) {
        Some(target) => Ok(target.to_string()),
        None => err("$ref", ptr, tr!("`$ref` must be a string")),
    }
}

fn declared_type(obj: &IndexMap<String, Node>, ptr: &str) -> Res<Ty> {
    match obj.get("type").map(Node::value) {
        Some(Value::Str(name)) => match name.as_str() {
            "string" => Ok(Ty::String),
            "integer" => Ok(Ty::Integer),
            "number" => Ok(Ty::Number),
            "boolean" => Ok(Ty::Boolean),
            "array" => Ok(Ty::Array),
            "object" => Ok(Ty::Object),
            "null" => err(
                "type",
                ptr,
                tr!("`null` is not supported: there is no null in version 1 schemas"),
            ),
            other => err("type", ptr, tr!("`{other}` is not a type", other = other)),
        },
        Some(Value::Array(_)) => err(
            "type",
            ptr,
            tr!("type lists (unions, nullable types) are not supported in version 1"),
        ),
        Some(_) => err("type", ptr, tr!("`type` must be a type name")),
        None if obj.contains_key("enum") => Ok(Ty::String),
        // No `type` is any value, as in JSON Schema.
        None => Ok(Ty::Any),
    }
}

fn enum_values(value: &Value, ptr: &str) -> Res<Vec<String>> {
    let Some(items) = value.as_array() else {
        return err("enum", ptr, tr!("`enum` must be an array"));
    };
    if items.is_empty() {
        return err("enum", ptr, tr!("`enum` needs at least one value"));
    }
    let mut values: Vec<String> = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let Some(text) = item.value().as_str() else {
            return err(
                "enum",
                ptr,
                tr!(
                    "value {i} is {kind}: version 1 supports only string enumerations",
                    i = i,
                    kind = item.value().kind_name()
                ),
            );
        };
        if values.iter().any(|v| v == text) {
            return err("enum", ptr, tr!("`{text}` is listed twice", text = text));
        }
        values.push(text.to_string());
    }
    Ok(values)
}

fn required_names(obj: &IndexMap<String, Node>, ptr: &str) -> Res<Vec<String>> {
    let Some(node) = obj.get("required") else {
        return Ok(Vec::new());
    };
    let Some(items) = node.value().as_array() else {
        return err("required", ptr, tr!("`required` must be an array of names"));
    };
    let mut names: Vec<String> = Vec::with_capacity(items.len());
    for item in items {
        let Some(name) = item.value().as_str() else {
            return err("required", ptr, tr!("`required` must contain only strings"));
        };
        if names.iter().any(|n| n == name) {
            return err(
                "required",
                ptr,
                tr!("`{name}` is listed twice", name = name),
            );
        }
        names.push(name.to_string());
    }
    Ok(names)
}

/// A non-negative whole number such as `minItems`. A float with no fractional
/// part is accepted, as JSON Schema does.
fn count(obj: &IndexMap<String, Node>, key: &str, ptr: &str) -> Res<Option<usize>> {
    let Some(node) = obj.get(key) else {
        return Ok(None);
    };
    let n = match node.value() {
        Value::Int(i) if *i >= 0 => usize::try_from(*i).ok(),
        // `x as i64` is exact here: the value is whole and below 2^63.
        Value::Float(x) if *x >= 0.0 && x.fract() == 0.0 && *x < 9.0e18 => {
            usize::try_from(*x as i64).ok()
        }
        _ => None,
    };
    match n {
        Some(n) => Ok(Some(n)),
        None => err(
            key,
            ptr,
            tr!("`{key}` must be a non-negative whole number", key = key),
        ),
    }
}

fn range(obj: &IndexMap<String, Node>, ptr: &str) -> Res<Range> {
    let bound = |key: &str| -> Res<Option<Num>> {
        match obj.get(key).map(Node::value) {
            None => Ok(None),
            Some(Value::Int(i)) => Ok(Some(Num::Int(*i))),
            Some(Value::Float(x)) if x.is_finite() => Ok(Some(Num::Float(*x))),
            Some(_) => err(key, ptr, tr!("`{key}` must be a number", key = key)),
        }
    };
    Ok(Range {
        min: bound("minimum")?,
        max: bound("maximum")?,
    })
}

fn annotation(obj: &IndexMap<String, Node>, key: &str, ptr: &str) -> Res<Option<String>> {
    match obj.get(key).map(Node::value) {
        None => Ok(None),
        Some(Value::Str(s)) => Ok(Some(s.clone())),
        Some(_) => err(key, ptr, tr!("`{key}` must be a string", key = key)),
    }
}

#[cfg(test)]
#[path = "jsonschema_tests.rs"]
mod tests;
