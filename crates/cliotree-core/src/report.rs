//! Report designs (DESIGN.md, "Reports"): what a report tab holds. A report
//! is a kind (single page or multipage), a page setup and a layout, which is
//! the control tree of forms ([`crate::form`]), so that the report designer
//! can be the form designer. The design is stored as a JSON document
//! validated against `builtin:report`, whose definitions come from
//! `builtin:form`, so a property has one name in both.
//!
//! A report holds no data and edits none: its fields show expressions, and
//! its repeaters take their items from expressions. [`ReportDesign::problems`]
//! lists what a report cannot have.

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::form::{
    self, Address, ColumnDesign, Control, ControlKind, DESIGN_VERSION, FormDesign, RepeaterMode,
};
use crate::typespec::{Kind, TypeSpec};
use crate::validate::validate;
use crate::{Path, Segment, TypeId, json, jsonschema};

/// The version of the report document this build writes.
pub const REPORT_VERSION: u32 = 1;

/// Single page (one page that scrolls, like a dashboard) or multipage
/// (banded, page by page).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportKind {
    #[default]
    SinglePage,
    Multipage,
}

impl ReportKind {
    pub const ALL: [ReportKind; 2] = [ReportKind::SinglePage, ReportKind::Multipage];

    pub fn label(self) -> &'static str {
        match self {
            ReportKind::SinglePage => "Single page",
            ReportKind::Multipage => "Multipage",
        }
    }
}

/// The paper of the page setup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Paper {
    #[default]
    A4,
    A3,
    Letter,
    Legal,
}

impl Paper {
    pub const ALL: [Paper; 4] = [Paper::A4, Paper::A3, Paper::Letter, Paper::Legal];

    pub fn label(self) -> &'static str {
        match self {
            Paper::A4 => "A4",
            Paper::A3 => "A3",
            Paper::Letter => "Letter",
            Paper::Legal => "Legal",
        }
    }

    /// Width and height in points, upright.
    pub fn size(self) -> (f32, f32) {
        match self {
            Paper::A4 => (595.28, 841.89),
            Paper::A3 => (841.89, 1190.55),
            Paper::Letter => (612.0, 792.0),
            Paper::Legal => (612.0, 1008.0),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Orientation {
    #[default]
    Portrait,
    Landscape,
}

/// The margins of the page, in points.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Margins {
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
    pub left: u32,
}

impl Default for Margins {
    /// Half an inch all round.
    fn default() -> Self {
        Margins {
            top: 36,
            right: 36,
            bottom: 36,
            left: 36,
        }
    }
}

/// Paper, orientation and margins. A single page report uses it only for
/// the PDF.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PageSetup {
    #[serde(default)]
    pub paper: Paper,
    #[serde(default)]
    pub orientation: Orientation,
    #[serde(default)]
    pub margins: Margins,
}

impl PageSetup {
    /// Width and height of the page in points, as it is turned.
    pub fn size(&self) -> (f32, f32) {
        let (w, h) = self.paper.size();
        match self.orientation {
            Orientation::Portrait => (w, h),
            Orientation::Landscape => (h, w),
        }
    }

    /// The width inside the margins, in points.
    pub fn content_width(&self) -> f32 {
        let (w, _) = self.size();
        (w - (self.margins.left + self.margins.right) as f32).max(0.0)
    }
}

/// What a report tab holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReportDesign {
    pub version: u32,
    pub kind: ReportKind,
    #[serde(default)]
    pub page: PageSetup,
    /// The controls, as a form's: a Flow group at the top, as a form's top
    /// level is.
    pub layout: FormDesign,
}

impl ReportDesign {
    /// An empty report of `kind`.
    pub fn new(kind: ReportKind) -> ReportDesign {
        ReportDesign {
            version: REPORT_VERSION,
            kind,
            page: PageSetup::default(),
            layout: FormDesign::default(),
        }
    }

    /// The design as the JSON document it is stored as, pretty-printed.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("a design always serializes")
    }

    /// Reads a report document. It must be JSON that conforms to
    /// `builtin:report`; the reason is given in words if it does not.
    pub fn from_json(text: &str) -> Result<ReportDesign, String> {
        let parsed = json::parse(text).map_err(|e| format!("not valid JSON: {e}"))?;
        if let Some(issue) = validate(report_schema(), &parsed.node).first() {
            let at = if issue.path.is_root() {
                "the document".to_owned()
            } else {
                issue.path.to_string()
            };
            return Err(format!("{at}: {}", issue.message));
        }
        let design: ReportDesign = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if design.version > REPORT_VERSION || design.layout.version > DESIGN_VERSION {
            return Err(format!(
                "this report was made by a newer version (report version {}, this build reads \
                 up to {REPORT_VERSION})",
                design.version
            ));
        }
        Ok(design)
    }

    /// What the report has that a report cannot have, by the address of the
    /// control: a report does not edit, so nothing is bound to a path, and
    /// there are no buttons, editors, choices, masks, *enabled when* or
    /// write procedures.
    pub fn problems(&self) -> Vec<(Address, String)> {
        let mut out = Vec::new();
        for (at, control) in self.layout.walk() {
            if let Some(why) = control_problem(control) {
                out.push((at, why));
            }
        }
        out
    }
}

fn has_text(text: &Option<String>) -> bool {
    text.as_deref().is_some_and(|t| !t.trim().is_empty())
}

fn control_problem(c: &Control) -> Option<String> {
    let kind = c.kind.label().to_lowercase();
    if c.kind == ControlKind::Button {
        return Some("a report has no buttons".to_owned());
    }
    if c.path.is_some() {
        let instead = match c.kind {
            ControlKind::Repeater => "its items come from an expression",
            ControlKind::Field => "it shows a value",
            _ => "it has no path",
        };
        return Some(format!(
            "a report is not bound to a tab, so this {kind} has a path; in a report {instead}"
        ));
    }
    let editing = [
        (c.editor.is_some(), "an editor"),
        (has_text(&c.choices), "choices"),
        (c.masked, "a masked box"),
        (has_text(&c.enabled_when), "enabled when"),
        (has_text(&c.write_procedure), "a write procedure"),
        (has_text(&c.procedure), "a procedure"),
    ];
    if let Some((_, what)) = editing.iter().find(|(has, _)| *has) {
        return Some(format!(
            "a report does not edit, so this {kind} cannot have {what}"
        ));
    }
    for column in &c.table_columns {
        let editing = [
            (column.editor.is_some(), "an editor"),
            (has_text(&column.choices), "choices"),
            (has_text(&column.enabled_when), "enabled when"),
            (has_text(&column.write_procedure), "a write procedure"),
        ];
        if let Some((_, what)) = editing.iter().find(|(has, _)| *has) {
            return Some(format!(
                "a report does not edit, so the column {} cannot have {what}",
                column.name
            ));
        }
    }
    None
}

// ----------------------------------------------------------- the schema

/// The text of the schema of report documents (`builtin:report`): the
/// definitions of `builtin:form`, with the report around them.
pub fn report_schema_source() -> &'static str {
    static SOURCE: OnceLock<String> = OnceLock::new();
    SOURCE.get_or_init(|| {
        let mut schema: serde_json::Value = serde_json::from_str(form::form_schema_source())
            .expect("the bundled form schema is JSON");
        let report: serde_json::Value =
            serde_json::from_str(REPORT_DEFS).expect("the report definitions are JSON");
        schema["$ref"] = serde_json::Value::from("#/$defs/Report");
        let defs = schema["$defs"]
            .as_object_mut()
            .expect("the form schema has definitions");
        for (name, def) in report.as_object().expect("an object") {
            defs.insert(name.clone(), def.clone());
        }
        serde_json::to_string_pretty(&schema).expect("a schema serializes")
    })
}

/// What `builtin:report` adds to the definitions of `builtin:form`.
const REPORT_DEFS: &str = r##"{
  "Report": {
    "type": "object",
    "title": "Report design",
    "properties": {
      "version": {"type": "integer", "minimum": 1},
      "kind": {"type": "string", "enum": ["single_page", "multipage"]},
      "page": {"$ref": "#/$defs/PageSetup"},
      "layout": {"$ref": "#/$defs/Form"}
    },
    "required": ["version", "kind", "layout"]
  },
  "PageSetup": {
    "type": "object",
    "properties": {
      "paper": {"type": "string", "enum": ["a4", "a3", "letter", "legal"]},
      "orientation": {"type": "string", "enum": ["portrait", "landscape"]},
      "margins": {"$ref": "#/$defs/Margins"}
    }
  },
  "Margins": {
    "type": "object",
    "properties": {
      "top": {"type": "integer", "minimum": 0, "maximum": 400},
      "right": {"type": "integer", "minimum": 0, "maximum": 400},
      "bottom": {"type": "integer", "minimum": 0, "maximum": 400},
      "left": {"type": "integer", "minimum": 0, "maximum": 400}
    },
    "required": ["top", "right", "bottom", "left"]
  }
}"##;

/// The lowered schema of report documents, made once.
pub fn report_schema() -> &'static TypeSpec {
    static SPEC: OnceLock<TypeSpec> = OnceLock::new();
    SPEC.get_or_init(|| {
        jsonschema::lower_str(report_schema_source()).expect("the bundled report schema is valid")
    })
}

// ------------------------------------------------- the starting point

/// A new report of `kind` that starts from the array at `array` of the tab
/// `tab` (whose schema is `spec`): what the generated form has for that
/// array, with the items from an expression (`Orders.items as order`) and
/// fields that show expressions (`order.qty`) in place of paths. A table for
/// scalar or flat items, designed items otherwise. None if there is no array
/// there.
pub fn starting_design(
    kind: ReportKind,
    spec: &TypeSpec,
    tab: &str,
    array: &Path,
) -> Option<ReportDesign> {
    let resolved = spec.resolve(array)?;
    let Kind::Array { items, .. } = &spec.get(resolved.ty).kind else {
        return None;
    };
    let item = item_name(tab, array, &[]);
    let label = match array.segments().last() {
        Some(Segment::Key(key)) => key.clone(),
        _ => tab.to_owned(),
    };
    let repeater = repeater_over(
        spec,
        *items,
        attribute_chain(tab, array),
        item,
        label,
        &mut Vec::new(),
    );
    let mut design = ReportDesign::new(kind);
    design.layout.controls.push(repeater);
    Some(design)
}

/// The array paths of a schema that a report can start from: every array
/// that exists without an index, the root included.
pub fn starting_arrays(spec: &TypeSpec) -> Vec<Path> {
    let mut out = Vec::new();
    collect_arrays(spec, spec.root(), &Path::root(), &mut out, &mut Vec::new());
    out
}

fn collect_arrays(
    spec: &TypeSpec,
    ty: TypeId,
    path: &Path,
    out: &mut Vec<Path>,
    open: &mut Vec<TypeId>,
) {
    match &spec.get(ty).kind {
        Kind::Array { .. } => out.push(path.clone()),
        Kind::Object { fields } => {
            if open.contains(&ty) {
                return;
            }
            open.push(ty);
            for field in fields {
                collect_arrays(spec, field.ty, &path.key(field.name.as_str()), out, open);
            }
            open.pop();
        }
        _ => {}
    }
}

/// A repeater over `items`, of item type `ty`, naming its item `item`.
/// `outer` holds the item names of the repeaters around it.
fn repeater_over(
    spec: &TypeSpec,
    ty: TypeId,
    items: String,
    item: String,
    label: String,
    outer: &mut Vec<String>,
) -> Control {
    let mut repeater = Control::new(ControlKind::Repeater);
    repeater.label = Some(label);
    repeater.items = Some(items);
    repeater.item_name = Some(item.clone());
    match flat_columns(spec, ty) {
        Some(columns) => {
            repeater.mode = Some(RepeaterMode::Table);
            repeater.table_columns = columns
                .into_iter()
                .map(|fields| {
                    let name = if fields.is_empty() {
                        "value".to_owned()
                    } else {
                        fields.join(".")
                    };
                    let mut column = ColumnDesign::named(&name);
                    column.value = Some(chain_from(&item, &fields));
                    column
                })
                .collect();
        }
        None => {
            repeater.mode = Some(RepeaterMode::Stacked);
            outer.push(item.clone());
            repeater.children = form::item_controls(spec, ty)
                .into_iter()
                .map(|c| to_expressions(spec, ty, c, &item, outer))
                .collect();
            outer.pop();
        }
    }
    repeater
}

/// A control of the generated items, with its path (relative to the item)
/// replaced by an expression of `item`.
fn to_expressions(
    spec: &TypeSpec,
    item_ty: TypeId,
    mut control: Control,
    item: &str,
    outer: &mut Vec<String>,
) -> Control {
    let path = control.bound_path().unwrap_or_else(Path::root);
    control.path = None;
    control.allow_other = false;
    let fields: Vec<String> = path
        .segments()
        .iter()
        .filter_map(|s| match s {
            Segment::Key(k) => Some(k.clone()),
            Segment::Index(_) => None,
        })
        .collect();
    match control.kind {
        ControlKind::Field => control.value = Some(chain_from(item, &fields)),
        ControlKind::Repeater => {
            let ty = resolve_from(spec, item_ty, &fields).and_then(|t| match &spec.get(t).kind {
                Kind::Array { items, .. } => Some(*items),
                _ => None,
            });
            if let Some(inner_ty) = ty {
                let mut taken = outer.clone();
                taken.push(item.to_owned());
                let inner = item_name("", &path, &taken);
                let label = control.label.clone().unwrap_or_default();
                return repeater_over(
                    spec,
                    inner_ty,
                    chain_from(item, &fields),
                    inner,
                    label,
                    outer,
                );
            }
        }
        _ => {}
    }
    control.children = std::mem::take(&mut control.children)
        .into_iter()
        .map(|c| to_expressions(spec, item_ty, c, item, outer))
        .collect();
    control
}

/// The type of the field reached from type `ty` by `fields`.
fn resolve_from(spec: &TypeSpec, ty: TypeId, fields: &[String]) -> Option<TypeId> {
    let mut here = ty;
    for name in fields {
        here = match &spec.get(here).kind {
            Kind::Object { fields } => fields.iter().find(|f| &f.name == name)?.ty,
            Kind::Map { value } => *value,
            _ => return None,
        };
    }
    Some(here)
}

/// The columns of a table of items of type `ty`, as the field names of each
/// (empty for a scalar item), if the items are flat: scalars, or objects
/// whose fields, with objects flattened, hold no arrays or maps.
fn flat_columns(spec: &TypeSpec, ty: TypeId) -> Option<Vec<Vec<String>>> {
    fn collect(
        spec: &TypeSpec,
        ty: TypeId,
        prefix: &mut Vec<String>,
        out: &mut Vec<Vec<String>>,
        open: &mut Vec<TypeId>,
    ) -> bool {
        let Kind::Object { fields } = &spec.get(ty).kind else {
            return true;
        };
        for field in fields {
            prefix.push(field.name.clone());
            let fine = match &spec.get(field.ty).kind {
                Kind::Object { .. } if open.contains(&field.ty) => false,
                Kind::Object { .. } => {
                    open.push(field.ty);
                    let fine = collect(spec, field.ty, prefix, out, open);
                    open.pop();
                    fine
                }
                Kind::Array { .. } | Kind::Map { .. } => false,
                _ => {
                    out.push(prefix.clone());
                    true
                }
            };
            prefix.pop();
            if !fine {
                return false;
            }
        }
        true
    }
    match &spec.get(ty).kind {
        Kind::Array { .. } | Kind::Map { .. } => None,
        Kind::Object { .. } => {
            let mut out = Vec::new();
            (collect(spec, ty, &mut Vec::new(), &mut out, &mut vec![ty]) && !out.is_empty())
                .then_some(out)
        }
        _ => Some(vec![Vec::new()]),
    }
}

/// `base` followed by the fields: `.name` for a Python name, `["first-name"]`
/// otherwise.
fn chain_from(base: &str, fields: &[String]) -> String {
    let mut out = base.to_owned();
    for field in fields {
        if is_identifier(field) && !is_keyword(field) {
            out.push('.');
            out.push_str(field);
        } else {
            out.push_str(&format!("[{}]", serde_json::Value::from(field.as_str())));
        }
    }
    out
}

/// The expression that reads the array at `path` of the tab: `Orders.items`.
fn attribute_chain(tab: &str, path: &Path) -> String {
    let fields: Vec<String> = path
        .segments()
        .iter()
        .filter_map(|s| match s {
            Segment::Key(k) => Some(k.clone()),
            Segment::Index(_) => None,
        })
        .collect();
    chain_from(tab, &fields)
}

/// The name of the item of a repeater over the array at `path`: the array's
/// name in the singular (`lines` gives `line`); for an array called `items`,
/// or the root, the tab's name in the singular (`Orders.items as order`).
/// It is a Python name that is not in `taken`.
fn item_name(tab: &str, path: &Path, taken: &[String]) -> String {
    let key = match path.segments().last() {
        Some(Segment::Key(key)) if !key.eq_ignore_ascii_case("items") => key.clone(),
        _ => tab.to_owned(),
    };
    let mut name = singular(&key.to_lowercase());
    if !is_identifier(&name) || is_keyword(&name) || RESERVED.contains(&name.as_str()) {
        name = "item".to_owned();
    }
    let base = name.clone();
    let mut n = 2;
    while taken.contains(&name) {
        name = format!("{base}_{n}");
        n += 1;
    }
    name
}

fn singular(word: &str) -> String {
    if let Some(stem) = word.strip_suffix("ies")
        && stem.len() > 1
    {
        return format!("{stem}y");
    }
    for ending in ["sses", "xes", "ches", "shes"] {
        if word.ends_with(ending) {
            return word[..word.len() - 2].to_owned();
        }
    }
    match word.strip_suffix('s') {
        Some(stem) if stem.len() > 1 && !stem.ends_with('s') => stem.to_owned(),
        _ => word.to_owned(),
    }
}

/// Names that mean something else in a report expression.
const RESERVED: [&str; 6] = ["this", "parent", "root", "params", "page", "value"];

fn is_identifier(text: &str) -> bool {
    let mut chars = text.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn is_keyword(text: &str) -> bool {
    const KEYWORDS: [&str; 35] = [
        "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class",
        "continue", "def", "del", "elif", "else", "except", "finally", "for", "from", "global",
        "if", "import", "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return",
        "try", "while", "with", "yield",
    ];
    KEYWORDS.contains(&text)
}
