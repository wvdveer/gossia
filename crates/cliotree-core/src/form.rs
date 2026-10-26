//! Form designs (DESIGN.md, "Forms"): the tree of controls that a form is
//! drawn from, the built-in schema it is stored under (`builtin:form`), the
//! generated form, and the checks of a design against the schema of its tab.
//!
//! A design holds no data and no pixel positions. It is a list of controls laid
//! out one under the other; a group arranges its children in one or more
//! columns. A control is addressed by its place in the tree, a list of
//! indices ([`Address`]).

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::typespec::{Kind, TypeSpec};
use crate::validate::validate;
use crate::{Path, json, jsonschema};

/// The version of the design document this build writes.
pub const DESIGN_VERSION: u32 = 1;

/// The place of a control in a design: the index in the list of the design,
/// then in the children of that control, and so on.
pub type Address = Vec<usize>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ControlKind {
    /// The control for the node at a path: an editor for a scalar, a list of
    /// entries for a map.
    Field,
    /// Controls arranged in columns, with a title; bound to an object, it can
    /// make the object when it is absent.
    Group,
    /// An array, as a table or one item at a time.
    Repeater,
    Label,
    /// Runs a procedure of a module (step 7c).
    Button,
    Separator,
}

impl ControlKind {
    pub const ALL: [ControlKind; 6] = [
        ControlKind::Field,
        ControlKind::Group,
        ControlKind::Repeater,
        ControlKind::Label,
        ControlKind::Button,
        ControlKind::Separator,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ControlKind::Field => "Field",
            ControlKind::Group => "Group",
            ControlKind::Repeater => "Repeater",
            ControlKind::Label => "Label",
            ControlKind::Button => "Button",
            ControlKind::Separator => "Separator",
        }
    }
}

/// The editor a field uses, when the one its type calls for is not wanted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Editor {
    /// What the type calls for: a toggle, a dropdown, a text field.
    Auto,
    /// One line of text, for any scalar.
    Text,
    /// Several lines of text, for a string.
    Multiline,
}

impl Editor {
    pub const ALL: [Editor; 3] = [Editor::Auto, Editor::Text, Editor::Multiline];

    pub fn label(self) -> &'static str {
        match self {
            Editor::Auto => "Automatic",
            Editor::Text => "One line of text",
            Editor::Multiline => "Several lines of text",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RepeaterMode {
    /// A table with a row for each item.
    Table,
    /// One item at a time, with a stepper.
    Stacked,
}

impl RepeaterMode {
    pub fn label(self) -> &'static str {
        match self {
            RepeaterMode::Table => "Table",
            RepeaterMode::Stacked => "One at a time",
        }
    }
}

/// The typeface of the text of a control, or of the data views. The
/// application bundles a font for each (see `cliotree-app`, `style.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FontFamily {
    Sans,
    Serif,
    Monospace,
    Condensed,
    Script,
    Handwriting,
}

impl FontFamily {
    pub const ALL: [FontFamily; 6] = [
        FontFamily::Sans,
        FontFamily::Serif,
        FontFamily::Monospace,
        FontFamily::Condensed,
        FontFamily::Script,
        FontFamily::Handwriting,
    ];

    /// The name in a document.
    pub fn key(self) -> &'static str {
        match self {
            FontFamily::Sans => "sans",
            FontFamily::Serif => "serif",
            FontFamily::Monospace => "monospace",
            FontFamily::Condensed => "condensed",
            FontFamily::Script => "script",
            FontFamily::Handwriting => "handwriting",
        }
    }

    pub fn from_key(key: &str) -> Option<FontFamily> {
        FontFamily::ALL.into_iter().find(|f| f.key() == key)
    }

    pub fn label(self) -> &'static str {
        match self {
            FontFamily::Sans => "Sans",
            FontFamily::Serif => "Serif",
            FontFamily::Monospace => "Monospace",
            FontFamily::Condensed => "Condensed",
            FontFamily::Script => "Script",
            FontFamily::Handwriting => "Handwriting",
        }
    }
}

/// The font sizes a document may give, in points.
pub const FONT_SIZES: std::ops::RangeInclusive<u32> = 6..=72;

/// The widths a table column may be given, in points.
pub const COLUMN_WIDTHS: std::ops::RangeInclusive<u32> = 24..=800;

/// The name of the column of row numbers in a table, `[0]`, `[1]`, ...
pub const ROW_NUMBER: &str = "#";

/// Where the text of a table column sits in its cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Alignment {
    Left,
    Center,
    Right,
}

impl Alignment {
    pub const ALL: [Alignment; 3] = [Alignment::Left, Alignment::Center, Alignment::Right];

    pub fn label(self) -> &'static str {
        match self {
            Alignment::Left => "Left",
            Alignment::Center => "Centre",
            Alignment::Right => "Right",
        }
    }
}

/// One column of a repeater drawn as a table: a field of the items (`value`
/// for items that are scalars), or the row numbers ([`ROW_NUMBER`]). Its place
/// in the list of the table is its place on the screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ColumnDesign {
    pub name: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub hidden: bool,
    /// The title of the column, when it is not the name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Shown on hover over the title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    /// In points. Absent: as wide as the table makes it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    /// Where the text of the cells sits. Absent: numbers on the right, the
    /// rest on the left.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub align: Option<Alignment>,
    /// Where the title sits. Absent: on the left.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_align: Option<Alignment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub editor: Option<Editor>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub read_only: bool,
    /// For a number: commas between the thousands.
    #[serde(default, skip_serializing_if = "is_false")]
    pub thousands: bool,
    /// For a number: decimal places, 0 to 10. Absent: as many as it has.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decimals: Option<u32>,
    /// For a date or a date-time: a pattern such as `dd/mm/yyyy`. Absent:
    /// ISO 8601, `yyyy-mm-dd`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date_format: Option<String>,
    /// The font of the cells. Absent: the font of the table. The titles of
    /// the columns are in the table's `header_font`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<FontFamily>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_size: Option<u32>,
    /// For the row numbers: how they are written. Absent: `[0]`, `[1]`, ...
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub numbering: Option<Numbering>,
    /// For the row numbers: the number of the first row. Absent: 0 for
    /// [`Numbering::Index`], 1 for the others.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<i64>,
}

/// How the row numbers of a table are written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Numbering {
    /// `[0]`, `[1]`, `[2]`: the index of the item, as a path writes it.
    Index,
    /// 1, 2, 3
    Decimal,
    /// a, b, c, ... z, aa, ab
    LowerAlpha,
    UpperAlpha,
    /// i, ii, iii, iv
    LowerRoman,
    UpperRoman,
}

impl Numbering {
    pub const ALL: [Numbering; 6] = [
        Numbering::Index,
        Numbering::Decimal,
        Numbering::LowerAlpha,
        Numbering::UpperAlpha,
        Numbering::LowerRoman,
        Numbering::UpperRoman,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Numbering::Index => "[0], [1], [2]",
            Numbering::Decimal => "1, 2, 3",
            Numbering::LowerAlpha => "a, b, c",
            Numbering::UpperAlpha => "A, B, C",
            Numbering::LowerRoman => "i, ii, iii",
            Numbering::UpperRoman => "I, II, III",
        }
    }

    /// The number of the first row, unless one is given.
    pub fn default_start(self) -> i64 {
        match self {
            Numbering::Index => 0,
            _ => 1,
        }
    }
}

impl ColumnDesign {
    pub fn named(name: &str) -> ColumnDesign {
        ColumnDesign {
            name: name.to_owned(),
            hidden: false,
            label: None,
            help: None,
            width: None,
            align: None,
            title_align: None,
            editor: None,
            read_only: false,
            thousands: false,
            decimals: None,
            date_format: None,
            font: None,
            font_size: None,
            numbering: None,
            start: None,
        }
    }

    /// What the row numbers column shows for the item at `index`.
    pub fn row_label(&self, index: usize) -> String {
        let numbering = self.numbering.unwrap_or(Numbering::Index);
        let start = self.start.unwrap_or_else(|| numbering.default_start());
        let n = start.saturating_add(index as i64);
        crate::format::sequence(n, numbering)
    }

    pub fn is_row_number(&self) -> bool {
        self.name == ROW_NUMBER
    }

    pub fn format(&self) -> Format {
        Format {
            thousands: self.thousands,
            decimals: self.decimals,
            date_format: self.date_format.clone(),
        }
    }
}

/// How a number or a date is shown (see [`crate::format`]), as a field or a
/// column gives it. A number format applies to numbers, a date format to
/// dates and date-times.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Format {
    pub thousands: bool,
    pub decimals: Option<u32>,
    pub date_format: Option<String>,
}

impl Format {
    pub fn is_empty(&self) -> bool {
        self == &Format::default()
    }

    /// What a value looks like in this format, or None if it does not apply.
    pub fn apply(&self, value: &crate::Value) -> Option<String> {
        crate::format::formatted(
            value,
            self.thousands,
            self.decimals,
            self.date_format.as_deref(),
        )
    }
}

/// One control. All kinds share the one record, so that the stored document
/// can be described in the schema subset, which has no unions; the checks say
/// which properties a kind uses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Control {
    #[serde(rename = "type")]
    pub kind: ControlKind,
    /// The node a field, group or repeater is bound to, as a formula writes
    /// it without the tab name: `customer.name`, `items`. An empty path is the
    /// root. No index appears in it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// The label of a field or repeater, or the title of a group, when it is
    /// not the name of the field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Help text, when it is not the description in the schema.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    /// The text of a label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub editor: Option<Editor>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub read_only: bool,
    /// For a group: the number of columns its children are arranged in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub columns: Option<u32>,
    /// For a repeater: a table or one at a time. Absent: a table when the
    /// items are flat, one at a time otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<RepeaterMode>,
    /// For a repeater drawn as a table, as the first designs wrote it: the
    /// fields that are its columns, in order. Read, and replaced by
    /// `table_columns` at the first change to the columns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show: Option<Vec<String>>,
    /// For a repeater drawn as a table: its columns, the row numbers among
    /// them, in their order, each with whether it is shown and its own
    /// properties. A column of the items that is not listed comes after the
    /// others, shown. Empty: every column in the order of the schema.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub table_columns: Vec<ColumnDesign>,
    /// For a number: commas between the thousands.
    #[serde(default, skip_serializing_if = "is_false")]
    pub thousands: bool,
    /// For a number: decimal places, 0 to 10. Absent: as many as it has.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decimals: Option<u32>,
    /// For a date or a date-time: a pattern such as `dd/mm/yyyy`. Absent:
    /// ISO 8601, `yyyy-mm-dd`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date_format: Option<String>,
    /// For a button: the procedure it runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub procedure: Option<String>,
    /// Python expressions, without the `=`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible_when: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled_when: Option<String>,
    /// The typeface of the control's text. Absent: the one of the group it is
    /// in, or the interface font.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<FontFamily>,
    /// The size of the control's text, in points. Absent: as for `font`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_size: Option<u32>,
    /// For a repeater: the font of its title. Absent: the control's font.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_font: Option<FontFamily>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_font_size: Option<u32>,
    /// For a repeater drawn as a table: the font of the titles of its
    /// columns. Absent: the control's font.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header_font: Option<FontFamily>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header_font_size: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Control>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl Control {
    pub fn new(kind: ControlKind) -> Control {
        Control {
            kind,
            path: None,
            label: None,
            help: None,
            text: None,
            editor: None,
            read_only: false,
            columns: None,
            mode: None,
            show: None,
            table_columns: Vec::new(),
            thousands: false,
            decimals: None,
            date_format: None,
            procedure: None,
            visible_when: None,
            enabled_when: None,
            font: None,
            font_size: None,
            title_font: None,
            title_font_size: None,
            header_font: None,
            header_font_size: None,
            children: Vec::new(),
        }
    }

    pub fn bound(kind: ControlKind, path: &Path) -> Control {
        Control {
            path: Some(path.to_string()),
            ..Control::new(kind)
        }
    }

    /// The path this control is bound to, if it has one that can be read.
    pub fn bound_path(&self) -> Option<Path> {
        self.path.as_deref().and_then(|p| p.parse().ok())
    }

    /// Whether the kind binds to a node.
    pub fn binds(&self) -> bool {
        matches!(
            self.kind,
            ControlKind::Field | ControlKind::Group | ControlKind::Repeater
        ) && self.path.is_some()
    }

    /// How the field shows a number or a date.
    pub fn format(&self) -> Format {
        Format {
            thousands: self.thousands,
            decimals: self.decimals,
            date_format: self.date_format.clone(),
        }
    }

    /// The columns of the table of a repeater whose items have the columns
    /// `names`, in their order on the screen, the row numbers among them, each
    /// with its properties and whether it is hidden. A column the control
    /// names that the items do not have is left out (and kept in the design).
    pub fn arrange_columns(&self, names: &[String]) -> Vec<ColumnDesign> {
        let known = |name: &str| name == ROW_NUMBER || names.iter().any(|n| n == name);
        let mut out: Vec<ColumnDesign> = Vec::new();
        if !self.table_columns.is_empty() {
            for column in &self.table_columns {
                if known(&column.name) && !out.iter().any(|c| c.name == column.name) {
                    out.push(column.clone());
                }
            }
            if !out.iter().any(ColumnDesign::is_row_number) {
                out.insert(0, ColumnDesign::named(ROW_NUMBER));
            }
            for name in names {
                if !out.iter().any(|c| &c.name == name) {
                    out.push(ColumnDesign::named(name));
                }
            }
            return out;
        }
        out.push(ColumnDesign::named(ROW_NUMBER));
        // The first designs listed the columns shown, in order, and hid the
        // rest. A list that names no column is no choice.
        let shown: Vec<&String> = self
            .show
            .iter()
            .flatten()
            .filter(|n| names.contains(n))
            .collect();
        if shown.is_empty() {
            out.extend(names.iter().map(|n| ColumnDesign::named(n)));
            return out;
        }
        out.extend(shown.iter().map(|n| ColumnDesign::named(n)));
        for name in names {
            if !shown.contains(&name) {
                out.push(ColumnDesign {
                    hidden: true,
                    ..ColumnDesign::named(name)
                });
            }
        }
        out
    }

    /// Stores the columns of the table, as [`Control::arrange_columns`] gives
    /// them, after a change. A list that is the default (every column shown,
    /// in the order of the schema, with no properties) is stored as nothing.
    /// Designs of columns the items do not have now are kept, at the end.
    pub fn set_columns(&mut self, columns: Vec<ColumnDesign>, names: &[String]) {
        let mut default = vec![ColumnDesign::named(ROW_NUMBER)];
        default.extend(names.iter().map(|n| ColumnDesign::named(n)));
        let absent: Vec<ColumnDesign> = self
            .table_columns
            .iter()
            .filter(|c| !c.is_row_number() && !names.contains(&c.name))
            .cloned()
            .collect();
        self.show = None;
        self.table_columns = if columns == default && absent.is_empty() {
            Vec::new()
        } else {
            columns.into_iter().chain(absent).collect()
        };
    }
}

/// A form: the controls under its title, laid out one under the other.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FormDesign {
    pub version: u32,
    /// Shown above the controls. Absent: the title of the schema, or the name
    /// of the tab.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub controls: Vec<Control>,
}

impl Default for FormDesign {
    fn default() -> Self {
        FormDesign {
            version: DESIGN_VERSION,
            title: None,
            controls: Vec::new(),
        }
    }
}

// ----------------------------------------------------------- the schema

/// The text of the schema of design documents (`builtin:form`).
pub fn form_schema_source() -> &'static str {
    r##"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$ref": "#/$defs/Form",
  "$defs": {
    "Form": {
      "type": "object",
      "title": "Form design",
      "properties": {
        "version": {"type": "integer", "minimum": 1},
        "title": {"type": "string"},
        "controls": {"type": "array", "items": {"$ref": "#/$defs/Control"}}
      },
      "required": ["version", "controls"]
    },
    "Control": {
      "type": "object",
      "properties": {
        "type": {"type": "string", "enum": ["field", "group", "repeater", "label", "button", "separator"]},
        "path": {"type": "string"},
        "label": {"type": "string"},
        "help": {"type": "string"},
        "text": {"type": "string"},
        "editor": {"type": "string", "enum": ["auto", "text", "multiline"]},
        "read_only": {"type": "boolean"},
        "columns": {"type": "integer", "minimum": 1, "maximum": 6},
        "mode": {"type": "string", "enum": ["table", "stacked"]},
        "show": {"type": "array", "items": {"type": "string"}},
        "table_columns": {"type": "array", "items": {"$ref": "#/$defs/Column"}},
        "thousands": {"type": "boolean"},
        "decimals": {"type": "integer", "minimum": 0, "maximum": 10},
        "date_format": {"type": "string", "minLength": 1},
        "procedure": {"type": "string"},
        "visible_when": {"type": "string"},
        "enabled_when": {"type": "string"},
        "font": {"type": "string", "enum": ["sans", "serif", "monospace", "condensed", "script", "handwriting"]},
        "font_size": {"type": "integer", "minimum": 6, "maximum": 72},
        "title_font": {"type": "string", "enum": ["sans", "serif", "monospace", "condensed", "script", "handwriting"]},
        "title_font_size": {"type": "integer", "minimum": 6, "maximum": 72},
        "header_font": {"type": "string", "enum": ["sans", "serif", "monospace", "condensed", "script", "handwriting"]},
        "header_font_size": {"type": "integer", "minimum": 6, "maximum": 72},
        "children": {"type": "array", "items": {"$ref": "#/$defs/Control"}}
      },
      "required": ["type"]
    },
    "Column": {
      "type": "object",
      "properties": {
        "name": {"type": "string", "minLength": 1},
        "hidden": {"type": "boolean"},
        "label": {"type": "string"},
        "help": {"type": "string"},
        "width": {"type": "integer", "minimum": 24, "maximum": 800},
        "align": {"type": "string", "enum": ["left", "center", "right"]},
        "title_align": {"type": "string", "enum": ["left", "center", "right"]},
        "editor": {"type": "string", "enum": ["auto", "text", "multiline"]},
        "read_only": {"type": "boolean"},
        "thousands": {"type": "boolean"},
        "decimals": {"type": "integer", "minimum": 0, "maximum": 10},
        "date_format": {"type": "string", "minLength": 1},
        "font": {"type": "string", "enum": ["sans", "serif", "monospace", "condensed", "script", "handwriting"]},
        "font_size": {"type": "integer", "minimum": 6, "maximum": 72},
        "numbering": {"type": "string", "enum": ["index", "decimal", "lower-alpha", "upper-alpha", "lower-roman", "upper-roman"]},
        "start": {"type": "integer"}
      },
      "required": ["name"]
    }
  }
}"##
}

/// The lowered schema of design documents, made once.
pub fn form_schema() -> &'static TypeSpec {
    static SPEC: OnceLock<TypeSpec> = OnceLock::new();
    SPEC.get_or_init(|| {
        jsonschema::lower_str(form_schema_source()).expect("the bundled form schema is valid")
    })
}

impl FormDesign {
    /// The design as the JSON document it is stored as, pretty-printed.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("a design always serializes")
    }

    /// Reads a design document. It must be JSON that conforms to the form
    /// schema; the reason is given in words if it does not. Whether its paths
    /// exist in a tab is a different question, see [`FormDesign::broken`].
    pub fn from_json(text: &str) -> Result<FormDesign, String> {
        let parsed = json::parse(text).map_err(|e| format!("not valid JSON: {e}"))?;
        let issues = validate(form_schema(), &parsed.node);
        if let Some(issue) = issues.first() {
            let at = if issue.path.is_root() {
                "the document".to_owned()
            } else {
                issue.path.to_string()
            };
            return Err(format!("{at}: {}", issue.message));
        }
        let design: FormDesign = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if design.version > DESIGN_VERSION {
            return Err(format!(
                "this form was made by a newer version (design version {}, this build reads up to {DESIGN_VERSION})",
                design.version
            ));
        }
        Ok(design)
    }
}

// ----------------------------------------------------------- the tree

impl FormDesign {
    fn list(&self, parent: &[usize]) -> Option<&Vec<Control>> {
        let mut list = &self.controls;
        for &i in parent {
            list = &list.get(i)?.children;
        }
        Some(list)
    }

    fn list_mut(&mut self, parent: &[usize]) -> Option<&mut Vec<Control>> {
        let mut list = &mut self.controls;
        for &i in parent {
            list = &mut list.get_mut(i)?.children;
        }
        Some(list)
    }

    pub fn get(&self, address: &[usize]) -> Option<&Control> {
        let (last, parent) = address.split_last()?;
        self.list(parent)?.get(*last)
    }

    pub fn get_mut(&mut self, address: &[usize]) -> Option<&mut Control> {
        let (last, parent) = address.split_last()?;
        self.list_mut(parent)?.get_mut(*last)
    }

    /// Adds `control` as the child of `parent` (the top level for an empty
    /// address) at `index`, or at the end. Returns where it is.
    pub fn insert(
        &mut self,
        parent: &[usize],
        index: Option<usize>,
        control: Control,
    ) -> Option<Address> {
        let list = self.list_mut(parent)?;
        let index = index.unwrap_or(list.len()).min(list.len());
        list.insert(index, control);
        let mut at = parent.to_vec();
        at.push(index);
        Some(at)
    }

    pub fn remove(&mut self, address: &[usize]) -> Option<Control> {
        let (last, parent) = address.split_last()?;
        let list = self.list_mut(parent)?;
        (*last < list.len()).then(|| list.remove(*last))
    }

    /// Moves a control one place up or down among its siblings. Returns the
    /// new address, or None if it is at the end already.
    pub fn shift(&mut self, address: &[usize], up: bool) -> Option<Address> {
        let (last, parent) = address.split_last()?;
        let list = self.list_mut(parent)?;
        let other = if up {
            last.checked_sub(1)?
        } else {
            (*last + 1 < list.len()).then_some(*last + 1)?
        };
        list.swap(*last, other);
        let mut at = parent.to_vec();
        at.push(other);
        Some(at)
    }

    /// Moves a control into the list that ends before it (the group above it
    /// takes it as its last child), or out of its group, to just after it.
    /// `into` chooses which. Returns the new address.
    pub fn nest(&mut self, address: &[usize], into: bool) -> Option<Address> {
        let (last, parent) = address.split_last()?;
        if into {
            let above = last.checked_sub(1)?;
            let mut target = parent.to_vec();
            target.push(above);
            if self.get(&target)?.kind != ControlKind::Group {
                return None;
            }
            let control = self.remove(address)?;
            let list = self.list_mut(&target)?;
            list.push(control);
            let mut at = target;
            at.push(list.len() - 1);
            Some(at)
        } else {
            let (group, grand) = parent.split_last()?;
            let control = self.remove(address)?;
            let index = group + 1;
            let mut at = grand.to_vec();
            self.list_mut(&at)?.insert(index, control);
            at.push(index);
            Some(at)
        }
    }

    /// Every control with its address, in the order of the form.
    pub fn walk(&self) -> Vec<(Address, &Control)> {
        fn visit<'a>(list: &'a [Control], at: &mut Address, out: &mut Vec<(Address, &'a Control)>) {
            for (i, control) in list.iter().enumerate() {
                at.push(i);
                out.push((at.clone(), control));
                visit(&control.children, at, out);
                at.pop();
            }
        }
        let mut out = Vec::new();
        visit(&self.controls, &mut Vec::new(), &mut out);
        out
    }
}

// ------------------------------------------------------ the generated form

/// The form drawn from a schema when none has been made: an object becomes a
/// group, an array a repeater, anything else a field.
pub fn generate(spec: &TypeSpec) -> FormDesign {
    let root = spec.root();
    let controls = match &spec.get(root).kind {
        Kind::Object { .. } => generated_children(spec, root, &Path::root()),
        _ => vec![generated(spec, root, &Path::root(), "")],
    };
    FormDesign {
        version: DESIGN_VERSION,
        title: None,
        controls,
    }
}

/// The control the generated form has for the node at `path`: what *Add* puts
/// on a form for a field that is not on it. None if the schema has no such node.
pub fn control_for(spec: &TypeSpec, path: &Path) -> Option<Control> {
    let resolved = spec.resolve(path)?;
    let name = match path.segments().last() {
        Some(crate::Segment::Key(key)) => key.clone(),
        _ => String::new(),
    };
    Some(generated(spec, resolved.ty, path, &name))
}

fn generated_children(spec: &TypeSpec, ty: crate::TypeId, path: &Path) -> Vec<Control> {
    let Kind::Object { fields } = &spec.get(ty).kind else {
        return Vec::new();
    };
    fields
        .iter()
        .map(|f| generated(spec, f.ty, &path.key(f.name.as_str()), &f.name))
        .collect()
}

fn generated(spec: &TypeSpec, ty: crate::TypeId, path: &Path, name: &str) -> Control {
    let label = (!name.is_empty()).then(|| name.to_owned());
    let kind = match &spec.get(ty).kind {
        Kind::Object { .. } => ControlKind::Group,
        Kind::Array { .. } => ControlKind::Repeater,
        _ => ControlKind::Field,
    };
    let mut control = Control::bound(kind, path);
    control.label = label;
    if kind == ControlKind::Group {
        control.children = generated_children(spec, ty, path);
    }
    control
}

/// The paths a control of `kind` can be bound to: a field takes a scalar or a
/// map, a group an object, a repeater an array. Only nodes that exist
/// without an index are offered, and the root is not.
pub fn bindable(spec: &TypeSpec, kind: ControlKind) -> Vec<Path> {
    fn visit(
        spec: &TypeSpec,
        ty: crate::TypeId,
        path: &Path,
        kind: ControlKind,
        out: &mut Vec<Path>,
    ) {
        let Kind::Object { fields } = &spec.get(ty).kind else {
            return;
        };
        for field in fields {
            let child = path.key(field.name.as_str());
            let wanted = match (&spec.get(field.ty).kind, kind) {
                (Kind::Object { .. }, ControlKind::Group) => true,
                (Kind::Array { .. }, ControlKind::Repeater) => true,
                (Kind::Object { .. } | Kind::Array { .. }, _) => false,
                (_, ControlKind::Field) => true,
                _ => false,
            };
            if wanted {
                out.push(child.clone());
            }
            visit(spec, field.ty, &child, kind, out);
        }
    }
    let mut out = Vec::new();
    visit(spec, spec.root(), &Path::root(), kind, &mut out);
    out
}

// ------------------------------------------------- a design and its schema

impl FormDesign {
    /// Why the control at `address` cannot be drawn against `spec`, or None.
    /// Paths are literal: one the schema does not define is broken, whatever
    /// the schema was before.
    pub fn problem(&self, spec: &TypeSpec, address: &[usize]) -> Option<String> {
        let control = self.get(address)?;
        control_problem(spec, control)
    }

    /// The controls that are broken, with the reason.
    pub fn broken(&self, spec: &TypeSpec) -> Vec<(Address, String)> {
        self.walk()
            .into_iter()
            .filter_map(|(at, c)| control_problem(spec, c).map(|p| (at, p)))
            .collect()
    }

    /// The nodes of the schema that no control shows, outermost first: a node
    /// that is shown, or lies in one that is, is not listed, and an object
    /// that is partly shown is listed by the fields it lacks. Paths in arrays
    /// are not listed, since a repeater shows its items.
    pub fn unplaced(&self, spec: &TypeSpec) -> Vec<Path> {
        // A field or a repeater shows its node and all that is in it. A group
        // shows only its children, so it only marks its object as partly shown.
        let mut bound = Vec::new();
        let mut groups = Vec::new();
        for (_, control) in self.walk() {
            if let Some(path) = control.bound_path().filter(|_| control.binds()) {
                if control.kind == ControlKind::Group {
                    groups.push(path);
                } else {
                    bound.push(path);
                }
            }
        }
        let shown = Shown { bound, groups };
        let mut out = Vec::new();
        visit_unplaced(spec, spec.root(), &Path::root(), &shown, &mut out);
        out
    }
}

/// Why a control cannot be drawn against a schema, or None.
pub fn control_problem(spec: &TypeSpec, control: &Control) -> Option<String> {
    let Some(text) = control.path.as_deref() else {
        return match control.kind {
            ControlKind::Field | ControlKind::Repeater => Some(format!(
                "a {} needs a path",
                control.kind.label().to_lowercase()
            )),
            _ => None,
        };
    };
    if !matches!(
        control.kind,
        ControlKind::Field | ControlKind::Group | ControlKind::Repeater
    ) {
        return None;
    }
    let path: Path = match text.parse() {
        Ok(path) => path,
        Err(e) => return Some(format!("the path {text:?} is not a path: {e}")),
    };
    let Some(resolved) = spec.resolve(&path) else {
        return Some(format!(
            "the schema has nothing at {}",
            if text.is_empty() { "the root" } else { text }
        ));
    };
    let kind = &spec.get(resolved.ty).kind;
    match (control.kind, kind) {
        (ControlKind::Repeater, Kind::Array { .. }) => None,
        (ControlKind::Repeater, _) => Some(format!("{text} is not an array")),
        (ControlKind::Group, Kind::Object { .. }) => None,
        (ControlKind::Group, _) => Some(format!("{text} is not an object")),
        _ => None,
    }
}

struct Shown {
    bound: Vec<Path>,
    groups: Vec<Path>,
}

fn visit_unplaced(
    spec: &TypeSpec,
    ty: crate::TypeId,
    path: &Path,
    shown: &Shown,
    out: &mut Vec<Path>,
) {
    let Kind::Object { fields } = &spec.get(ty).kind else {
        return;
    };
    for field in fields {
        let child = path.key(field.name.as_str());
        if shown.bound.iter().any(|b| inside(&child, b)) {
            continue;
        }
        let partly = shown.bound.iter().any(|b| inside(b, &child))
            || shown.groups.iter().any(|g| inside(g, &child));
        if !partly {
            out.push(child);
        } else if matches!(spec.get(field.ty).kind, Kind::Object { .. }) {
            visit_unplaced(spec, field.ty, &child, shown, out);
        }
    }
}

/// Whether `inner` is `outer` or lies in it.
fn inside(inner: &Path, outer: &Path) -> bool {
    inner.segments().starts_with(outer.segments())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> TypeSpec {
        jsonschema::lower_str(
            r##"{
              "type": "object",
              "properties": {
                "name": {"type": "string"},
                "customer": {"type": "object", "properties": {
                    "city": {"type": "string"}, "vip": {"type": "boolean"}}},
                "items": {"type": "array", "items": {"type": "object",
                    "properties": {"sku": {"type": "string"}, "qty": {"type": "integer"}}}},
                "tags": {"type": "object", "additionalProperties": {"type": "string"}}
              }
            }"##,
        )
        .unwrap()
    }

    #[test]
    fn the_form_schema_lowers_and_is_not_a_meta_schema() {
        assert!(form_schema().xml().is_none());
    }

    #[test]
    fn the_generated_form_has_a_control_for_each_field() {
        let design = generate(&spec());
        let kinds: Vec<_> = design.controls.iter().map(|c| c.kind).collect();
        assert_eq!(
            kinds,
            [
                ControlKind::Field,
                ControlKind::Group,
                ControlKind::Repeater,
                ControlKind::Field
            ]
        );
        assert_eq!(design.controls[1].children.len(), 2);
        assert_eq!(design.controls[1].path.as_deref(), Some("customer"));
        assert_eq!(
            design.controls[1].children[0].path.as_deref(),
            Some("customer.city")
        );
        assert!(design.broken(&spec()).is_empty());
        assert!(design.unplaced(&spec()).is_empty());
    }

    #[test]
    fn a_design_round_trips_through_its_document() {
        let mut design = generate(&spec());
        design.title = Some("Orders".into());
        design.controls[0].read_only = true;
        design.controls[0].editor = Some(Editor::Multiline);
        design.controls[2].mode = Some(RepeaterMode::Stacked);
        design.controls[2].show = Some(vec!["sku".into()]);
        design.controls[1].font = Some(FontFamily::Serif);
        design.controls[1].font_size = Some(18);
        design.controls.push(Control::new(ControlKind::Separator));
        let text = design.to_json();
        assert_eq!(FormDesign::from_json(&text).unwrap(), design);
        // Properties that are not set are not written.
        assert!(!text.contains("null"));
    }

    #[test]
    fn a_document_that_does_not_conform_is_refused_with_the_reason() {
        let wrong = FormDesign::from_json(r#"{"version":1,"controls":[{"type":"widget"}]}"#);
        assert!(wrong.unwrap_err().contains("controls[0].type"));
        let missing = FormDesign::from_json(r#"{"controls":[]}"#).unwrap_err();
        assert!(missing.contains("version"), "{missing}");
        let extra =
            FormDesign::from_json(r#"{"version":1,"controls":[{"type":"label","colour":"red"}]}"#);
        assert!(extra.is_err());
        let not_json = FormDesign::from_json("{").unwrap_err();
        assert!(not_json.starts_with("not valid JSON"));
        let newer = FormDesign::from_json(r#"{"version":99,"controls":[]}"#).unwrap_err();
        assert!(newer.contains("newer version"));
    }

    #[test]
    fn a_font_is_one_of_the_families_and_a_size_within_bounds() {
        let ok = FormDesign::from_json(
            r#"{"version":1,"controls":[{"type":"label","font":"script","font_size":24}]}"#,
        )
        .unwrap();
        assert_eq!(ok.controls[0].font, Some(FontFamily::Script));
        assert_eq!(ok.controls[0].font_size, Some(24));
        for wrong in [
            r#""font":"comic""#,
            r#""font_size":5"#,
            r#""font_size":100"#,
        ] {
            let text = format!(r#"{{"version":1,"controls":[{{"type":"label",{wrong}}}]}}"#);
            assert!(FormDesign::from_json(&text).is_err(), "{wrong}");
        }
        // The schema's list is the enum's.
        for family in FontFamily::ALL {
            assert!(form_schema_source().contains(&format!("\"{}\"", family.key())));
            assert_eq!(FontFamily::from_key(family.key()), Some(family));
        }
    }

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn order(columns: &[ColumnDesign]) -> Vec<String> {
        columns
            .iter()
            .map(|c| {
                if c.hidden {
                    format!("({})", c.name)
                } else {
                    c.name.clone()
                }
            })
            .collect()
    }

    #[test]
    fn table_columns_and_formats_round_trip_and_are_checked() {
        let mut design = generate(&spec());
        let mut qty = ColumnDesign::named("qty");
        qty.label = Some("Quantity".into());
        qty.width = Some(80);
        qty.align = Some(Alignment::Center);
        qty.thousands = true;
        qty.decimals = Some(2);
        qty.font = Some(FontFamily::Condensed);
        qty.title_align = Some(Alignment::Right);
        design.controls[2].title_font = Some(FontFamily::Serif);
        design.controls[2].title_font_size = Some(20);
        design.controls[2].header_font = Some(FontFamily::Script);
        design.controls[2].header_font_size = Some(9);
        let row = ColumnDesign {
            hidden: true,
            numbering: Some(Numbering::UpperRoman),
            start: Some(-2),
            ..ColumnDesign::named(ROW_NUMBER)
        };
        design.controls[2].table_columns = vec![qty, row];
        design.controls[0].date_format = Some("dd/mm/yyyy".into());
        design.controls[0].decimals = Some(0);
        let text = design.to_json();
        assert_eq!(FormDesign::from_json(&text).unwrap(), design);
        assert!(!text.contains("null"));
        for wrong in [
            r#""table_columns":[{"label":"x"}]"#,
            r#""table_columns":[{"name":"a","width":10}]"#,
            r#""table_columns":[{"name":"a","align":"justify"}]"#,
            r#""table_columns":[{"name":"a","colour":"red"}]"#,
            r##""table_columns":[{"name":"#","numbering":"greek"}]"##,
            r##""table_columns":[{"name":"#","start":1.5}]"##,
            r#""decimals":11"#,
            r#""date_format":"""#,
        ] {
            let text = format!(r#"{{"version":1,"controls":[{{"type":"repeater",{wrong}}}]}}"#);
            assert!(FormDesign::from_json(&text).is_err(), "{wrong}");
        }
    }

    #[test]
    fn row_numbers_start_where_the_design_says() {
        let mut row = ColumnDesign::named(ROW_NUMBER);
        assert_eq!(row.row_label(0), "[0]");
        row.numbering = Some(Numbering::Decimal);
        assert_eq!(row.row_label(0), "1", "counting starts at 1");
        row.numbering = Some(Numbering::LowerAlpha);
        assert_eq!(row.row_label(2), "c");
        row.start = Some(0);
        row.numbering = Some(Numbering::Index);
        assert_eq!(row.row_label(2), "[2]");
        row.start = Some(10);
        row.numbering = Some(Numbering::UpperRoman);
        assert_eq!(row.row_label(4), "XIV");
    }

    #[test]
    fn the_columns_of_a_table_are_arranged_from_the_design() {
        let all = names(&["sku", "qty", "price"]);
        let mut c = Control::new(ControlKind::Repeater);
        // No say: the row numbers, then the schema's order.
        assert_eq!(
            order(&c.arrange_columns(&all)),
            ["#", "sku", "qty", "price"]
        );
        // The first designs' list: those shown, in order, and the rest hidden.
        c.show = Some(names(&["price", "sku", "gone"]));
        assert_eq!(
            order(&c.arrange_columns(&all)),
            ["#", "price", "sku", "(qty)"]
        );
        // The list of the table: its order, the row numbers anywhere, a column
        // it does not name after the others, a name the items lack left out.
        c.show = None;
        c.table_columns = vec![
            ColumnDesign::named("qty"),
            ColumnDesign::named("gone"),
            ColumnDesign {
                hidden: true,
                ..ColumnDesign::named("sku")
            },
            ColumnDesign::named(ROW_NUMBER),
        ];
        assert_eq!(
            order(&c.arrange_columns(&all)),
            ["qty", "(sku)", "#", "price"]
        );
    }

    #[test]
    fn storing_the_default_columns_stores_nothing() {
        let all = names(&["sku", "qty"]);
        let mut c = Control::new(ControlKind::Repeater);
        c.show = Some(names(&["qty"]));
        let mut columns = c.arrange_columns(&all);
        for column in &mut columns {
            column.hidden = false;
        }
        columns.swap(1, 2);
        c.set_columns(columns, &all);
        assert_eq!(c.show, None, "the old list is replaced");
        assert!(c.table_columns.is_empty());
        // A design of a column the items no longer have is kept.
        c.table_columns = vec![ColumnDesign {
            label: Some("Old".into()),
            ..ColumnDesign::named("gone")
        }];
        let columns = c.arrange_columns(&all);
        c.set_columns(columns, &all);
        assert_eq!(order(&c.table_columns), ["#", "sku", "qty", "gone"]);
    }

    #[test]
    fn controls_are_found_moved_and_removed_by_address() {
        let mut design = generate(&spec());
        assert_eq!(
            design.get(&[1, 1]).and_then(|c| c.path.as_deref()),
            Some("customer.vip")
        );
        assert_eq!(design.shift(&[1, 1], true), Some(vec![1, 0]));
        assert_eq!(
            design.get(&[1, 0]).and_then(|c| c.path.as_deref()),
            Some("customer.vip")
        );
        assert_eq!(design.shift(&[1, 0], true), None);
        assert_eq!(design.shift(&[3], false), None);
        let removed = design.remove(&[0]).unwrap();
        assert_eq!(removed.path.as_deref(), Some("name"));
        assert_eq!(design.controls.len(), 3);
    }

    #[test]
    fn nesting_moves_a_control_into_the_group_above_and_out_again() {
        let mut design = generate(&spec());
        // [1] is the customer group, [2] the items repeater.
        let inside = design.nest(&[2], true).unwrap();
        assert_eq!(inside, vec![1, 2]);
        assert_eq!(design.controls.len(), 3);
        let outside = design.nest(&inside, false).unwrap();
        assert_eq!(outside, vec![2]);
        assert_eq!(design.controls.len(), 4);
        assert_eq!(design.nest(&[0], true), None, "nothing is above the first");
        assert_eq!(
            design.nest(&[0], false),
            None,
            "the first is not in a group"
        );
        // Only a group takes a control.
        assert_eq!(design.nest(&[3], true), None);
    }

    #[test]
    fn the_paths_a_control_can_bind_to_depend_on_its_kind() {
        let names = |kind| -> Vec<String> {
            bindable(&spec(), kind)
                .iter()
                .map(Path::to_string)
                .collect()
        };
        assert_eq!(
            names(ControlKind::Field),
            ["name", "customer.city", "customer.vip", "tags"]
        );
        assert_eq!(names(ControlKind::Group), ["customer"]);
        assert_eq!(names(ControlKind::Repeater), ["items"]);
        assert!(names(ControlKind::Label).is_empty());
    }

    #[test]
    fn a_path_the_schema_lacks_is_a_broken_control() {
        let mut design = generate(&spec());
        design.controls[0].path = Some("nickname".into());
        design.controls[2].path = Some("name".into());
        let broken = design.broken(&spec());
        assert_eq!(broken.len(), 2);
        assert!(broken[0].1.contains("nothing at nickname"));
        assert!(broken[1].1.contains("not an array"));
        design.controls[1].children[0].path = Some("customer[".into());
        assert!(
            design
                .problem(&spec(), &[1, 0])
                .unwrap()
                .contains("not a path")
        );
        // A field needs a path; a label does not.
        let mut bare = FormDesign::default();
        bare.controls.push(Control::new(ControlKind::Field));
        bare.controls.push(Control::new(ControlKind::Label));
        assert_eq!(bare.broken(&spec()).len(), 1);
    }

    #[test]
    fn fields_that_are_not_on_the_form_are_listed_outermost_first() {
        let mut design = generate(&spec());
        design.remove(&[3]);
        design.remove(&[1, 0]);
        design.remove(&[0]);
        let unplaced: Vec<String> = design
            .unplaced(&spec())
            .iter()
            .map(Path::to_string)
            .collect();
        assert_eq!(unplaced, ["name", "customer.city", "tags"]);
        // A group bound to an object shows all of it, whatever it holds.
        let mut group = FormDesign::default();
        group.controls.push(Control::bound(
            ControlKind::Group,
            &"customer".parse().unwrap(),
        ));
        let rest: Vec<String> = group
            .unplaced(&spec())
            .iter()
            .map(Path::to_string)
            .collect();
        assert_eq!(
            rest,
            ["name", "customer.city", "customer.vip", "items", "tags"]
        );
    }
}
