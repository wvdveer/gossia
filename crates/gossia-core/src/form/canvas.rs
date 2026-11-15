//! Form canvases (DESIGN.md, Later versions, "Form canvases, 0.7.0"): a form
//! as a stack of section canvases, as a multipage report is a stack of
//! bands.
//!
//! At the top level of a canvas design the controls are its sections, in
//! the order of the stack: the form header, then each repeater section with
//! a fixed section between each two, then the form footer. Every section is
//! a Free canvas as wide as the form; a repeater section is a repeater with
//! a Free layout, its controls being one row, drawn for each item.
//!
//! A path on a canvas form can take an index or a key from a context: a
//! context field of the form header, or the row number of a repeater
//! (`items[sel].name`, `orders[row].lines`). [`ContextPath`] reads such a
//! path and binds it to the values of the contexts.
//!
//! Designs made before canvases, and the generated form, are turned into
//! canvases by [`to_canvas`].

use std::fmt;
use std::ops::RangeInclusive;
use std::str::FromStr;

use gossia_i18n::tr;
use serde::{Deserialize, Serialize};

use super::{
    Address, CANVAS_VERSION, CHART_SIZE, Control, ControlKind, Editor, FormDesign, GroupLayout,
    LabelPosition, flow_rows,
};
use crate::typespec::{Kind, TypeSpec};
use crate::{Path, Segment, Value};

/// The widths a canvas form can have, in points.
pub const FORM_WIDTHS: RangeInclusive<u32> = 240..=2000;

/// The width of a canvas form that has none of its own.
pub const DEFAULT_FORM_WIDTH: u32 = 600;

/// The least height of a repeater section's row.
pub const MIN_ROW_HEIGHT: u32 = 16;

/// The space between the edge of a section and what the converter places on
/// it, and between the controls it places one under another.
const MARGIN: u32 = 8;

/// The height of a row of fields, and of the column titles above it, as the
/// converter makes them.
const ROW: u32 = 24;

/// The width of the add/delete button the converter puts at the end of a
/// row.
const ROW_BUTTON_WIDTH: u32 = 72;

/// The least width of a column of fields the converter makes in a row; more
/// columns than fit start another line of the row.
const MIN_COLUMN: u32 = 80;

// ------------------------------------------------------------ the kinds

/// The type of a context field's value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContextType {
    /// A whole number: what indexes an array.
    #[default]
    Integer,
    Number,
    /// Text: what keys a map, or an object's field.
    Text,
    Date,
    /// Yes or no.
    Boolean,
}

impl ContextType {
    pub const ALL: [ContextType; 5] = [
        ContextType::Integer,
        ContextType::Number,
        ContextType::Text,
        ContextType::Date,
        ContextType::Boolean,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ContextType::Integer => tr!("Whole number"),
            ContextType::Number => tr!("Number"),
            ContextType::Text => tr!("Text"),
            ContextType::Date => tr!("Date"),
            ContextType::Boolean => tr!("Yes/no"),
        }
    }

    /// The value `text` stands for, as a default or as typed into the field:
    /// empty text is `None`. Says in words why it is not one.
    pub fn read(self, text: &str) -> Result<Value, String> {
        let t = text.trim();
        if t.is_empty() {
            return Ok(Value::Null);
        }
        let bad = || {
            tr!(
                "{text} is not a valid value of the type {kind}",
                text = format!("{t:?}"),
                kind = self.label()
            )
        };
        match self {
            ContextType::Integer => t.parse().map(Value::Int).map_err(|_| bad()),
            ContextType::Number => t
                .parse::<f64>()
                .ok()
                .filter(|f| f.is_finite())
                .map(Value::Float)
                .ok_or_else(bad),
            ContextType::Text => Ok(Value::Str(text.to_owned())),
            ContextType::Date => chrono::NaiveDate::parse_from_str(t, "%Y-%m-%d")
                .map(Value::Date)
                .map_err(|_| bad()),
            ContextType::Boolean => match t.to_lowercase().as_str() {
                "true" | "yes" => Ok(Value::Bool(true)),
                "false" | "no" => Ok(Value::Bool(false)),
                _ => Err(bad()),
            },
        }
    }
}

/// What the add/delete button of a repeater section does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RowAllows {
    #[default]
    Both,
    Add,
    Delete,
}

impl RowAllows {
    pub const ALL: [RowAllows; 3] = [RowAllows::Both, RowAllows::Add, RowAllows::Delete];

    pub fn label(self) -> &'static str {
        match self {
            RowAllows::Both => tr!("Add and delete"),
            RowAllows::Add => tr!("Add only"),
            RowAllows::Delete => tr!("Delete only"),
        }
    }

    pub fn adds(self) -> bool {
        self != RowAllows::Delete
    }

    pub fn deletes(self) -> bool {
        self != RowAllows::Add
    }
}

impl Control {
    /// What an add/delete button allows.
    pub fn row_allows(&self) -> RowAllows {
        self.allows.unwrap_or_default()
    }

    /// The label of an add/delete button in its add or delete configuration.
    pub fn row_button_label(&self, add: bool) -> String {
        let (own, default) = if add {
            (&self.add_label, tr!("Add"))
        } else {
            (&self.delete_label, tr!("Delete"))
        };
        own.clone().unwrap_or_else(|| default.to_owned())
    }

    /// The type of a context field.
    pub fn context_kind(&self) -> ContextType {
        self.context_type.unwrap_or_default()
    }

    /// A context field's value when the form is first shown, or why its
    /// default cannot be read.
    pub fn context_start(&self) -> Result<Value, String> {
        self.context_kind()
            .read(self.context_default.as_deref().unwrap_or(""))
    }
}

// ----------------------------------------------------- paths with contexts

/// One step of a path that can take an index or a key from a context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Fixed(Segment),
    /// `[name]`: the value of the context `name`.
    Context(String),
}

/// A path whose steps can be contexts: `items[sel].name`. A path without
/// contexts reads and writes as a [`Path`] does.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ContextPath(Vec<Step>);

impl ContextPath {
    pub fn steps(&self) -> &[Step] {
        &self.0
    }

    /// The names of the contexts it uses, in order, each once.
    pub fn names(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for step in &self.0 {
            if let Step::Context(name) = step
                && !out.contains(&name.as_str())
            {
                out.push(name);
            }
        }
        out
    }

    pub fn has_context(&self) -> bool {
        self.0.iter().any(|s| matches!(s, Step::Context(_)))
    }

    /// The path, when it uses no context.
    pub fn fixed(&self) -> Option<Path> {
        self.0
            .iter()
            .map(|s| match s {
                Step::Fixed(seg) => Some(seg.clone()),
                Step::Context(_) => None,
            })
            .collect::<Option<Vec<_>>>()
            .map(Path::from)
    }

    /// The path it is for the contexts' values: a whole number is an index,
    /// text a key. Says in words why it is none, for a context that has no
    /// value or one of another kind.
    pub fn bind(&self, value_of: impl Fn(&str) -> Option<Value>) -> Result<Path, String> {
        let mut out = Vec::with_capacity(self.0.len());
        for step in &self.0 {
            out.push(match step {
                Step::Fixed(seg) => seg.clone(),
                Step::Context(name) => match value_of(name) {
                    Some(Value::Int(i)) if i >= 0 => Segment::Index(i as usize),
                    Some(Value::Str(s)) => Segment::Key(s),
                    Some(Value::Int(i)) => {
                        return Err(tr!("{name} is {i}, below 0", name = name, i = i));
                    }
                    Some(Value::Null) | None => {
                        return Err(tr!("{name} has no value", name = name));
                    }
                    Some(other) => {
                        return Err(tr!(
                            "{name} is {kind}, not a whole number or text",
                            name = name,
                            kind = other.kind_name()
                        ));
                    }
                },
            });
        }
        Ok(Path::from(out))
    }
}

/// Where in `chars` a `[name]` context starts at `i`, and the name and the
/// index after its `]`.
fn context_at(chars: &[char], i: usize) -> Option<(String, usize)> {
    if chars.get(i) != Some(&'[') {
        return None;
    }
    let first = *chars.get(i + 1)?;
    if !(first == '_' || first.is_alphabetic()) {
        return None;
    }
    let mut j = i + 1;
    while chars
        .get(j)
        .is_some_and(|&c| c == '_' || c.is_alphanumeric())
    {
        j += 1;
    }
    (chars.get(j) == Some(&']')).then(|| (chars[i + 1..j].iter().collect(), j + 1))
}

impl FromStr for ContextPath {
    type Err = String;

    /// Reads a path in which `[name]` is a context. The rest is read as a
    /// [`Path`] is, with its messages.
    fn from_str(text: &str) -> Result<ContextPath, String> {
        let chars: Vec<char> = text.chars().collect();
        // Each context is cut out, and what is between them read as a path;
        // the text after a context goes on from it, so it starts with `.` or
        // `[`, which a path cannot: it is read after a stand-in first step.
        let mut steps = Vec::new();
        let mut piece = String::new();
        let mut quoted = false;
        let mut i = 0;
        let mut first = true;
        while i < chars.len() {
            let c = chars[i];
            if quoted {
                piece.push(c);
                if c == '\\' {
                    if let Some(&next) = chars.get(i + 1) {
                        piece.push(next);
                        i += 1;
                    }
                } else if c == '"' {
                    quoted = false;
                }
                i += 1;
                continue;
            }
            if let Some((name, next)) = context_at(&chars, i) {
                if first && piece.is_empty() {
                    return Err(tr!("a path cannot start with a context").to_owned());
                }
                flush(&mut piece, &mut steps, first)?;
                first = false;
                steps.push(Step::Context(name));
                i = next;
                continue;
            }
            if c == '"' {
                quoted = true;
            }
            piece.push(c);
            i += 1;
        }
        flush(&mut piece, &mut steps, first)?;
        Ok(ContextPath(steps))
    }
}

/// Reads the text between contexts (or before the first) as the steps of a
/// path, and empties it. After a context it must go on with `.` or `[`.
fn flush(piece: &mut String, steps: &mut Vec<Step>, first: bool) -> Result<(), String> {
    if piece.is_empty() {
        return Ok(());
    }
    let parsed: Path = if first {
        piece
            .parse()
            .map_err(|e: crate::ParsePathError| e.to_string())?
    } else {
        if !(piece.starts_with('.') || piece.starts_with('[')) {
            return Err(tr!("expected . or [ after a context").to_owned());
        }
        let p: Path = format!("x{piece}")
            .parse()
            .map_err(|e: crate::ParsePathError| e.to_string())?;
        Path::from(p.segments()[1..].to_vec())
    };
    steps.extend(parsed.segments().iter().cloned().map(Step::Fixed));
    piece.clear();
    Ok(())
}

impl fmt::Display for ContextPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (n, step) in self.0.iter().enumerate() {
            match step {
                Step::Context(name) => write!(f, "[{name}]")?,
                Step::Fixed(seg) => {
                    let one = Path::from(vec![seg.clone()]).to_string();
                    if n > 0 && matches!(seg, Segment::Key(_)) && !one.starts_with('[') {
                        f.write_str(".")?;
                    }
                    f.write_str(&one)?;
                }
            }
        }
        Ok(())
    }
}

/// A path that stands for `text` read from `base` when the schema is
/// checked: each context is the first item of an array, or any key of a
/// map. `Ok(None)` when a context names a field of an object, so what it
/// reaches cannot be known; an error when `text` is not a path.
pub fn sample_path(spec: &TypeSpec, base: &Path, text: &str) -> Result<Option<Path>, String> {
    let parsed: ContextPath = text.parse()?;
    let mut path = base.clone();
    for step in parsed.steps() {
        match step {
            Step::Fixed(seg) => path.push(seg.clone()),
            Step::Context(_) => {
                let Some(here) = spec.resolve(&path) else {
                    // Nothing there: the check says so.
                    return Ok(Some(path));
                };
                match &spec.get(here.ty).kind {
                    Kind::Array { .. } | Kind::Any => path.push(Segment::Index(0)),
                    Kind::Map { .. } => path.push(Segment::Key(String::new())),
                    Kind::Object { .. } | Kind::Choice { .. } => return Ok(None),
                    // A scalar has no steps: the check says so.
                    _ => path.push(Segment::Index(0)),
                }
            }
        }
    }
    Ok(Some(path))
}

// ------------------------------------------------------------ the sections

/// A section of a canvas form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Header,
    /// The repeater section that is the `k`th repeater, from 0.
    Repeater(usize),
    /// The fixed section after the `k`th repeater, before the next.
    Between(usize),
    Footer,
}

impl Section {
    /// Its name in the designer.
    pub fn label(self) -> &'static str {
        match self {
            Section::Header => tr!("Form header"),
            Section::Repeater(_) => tr!("Repeater"),
            Section::Between(_) => tr!("Section"),
            Section::Footer => tr!("Form footer"),
        }
    }

    pub fn is_repeater(self) -> bool {
        matches!(self, Section::Repeater(_))
    }
}

/// The section that is at `index` of a stack of `count` controls: header,
/// repeater, section, repeater, ..., footer.
fn section_at(index: usize, count: usize) -> Section {
    if index == 0 {
        Section::Header
    } else if index + 1 == count {
        Section::Footer
    } else if index % 2 == 1 {
        Section::Repeater(index / 2)
    } else {
        Section::Between(index / 2 - 1)
    }
}

/// A new fixed section, `height` points tall.
pub fn new_section(height: u32) -> Control {
    let mut section = Control::new(ControlKind::Group);
    section.layout = Some(GroupLayout::Free);
    section.height = Some(height);
    section
}

/// A new repeater section over the array at `path`, with rows `height`
/// points tall.
pub fn new_repeater(path: &str, row_name: Option<String>, height: u32) -> Control {
    let mut repeater = Control::new(ControlKind::Repeater);
    repeater.path = Some(path.to_owned());
    repeater.layout = Some(GroupLayout::Free);
    repeater.height = Some(height.max(MIN_ROW_HEIGHT));
    repeater.row_name = row_name;
    repeater
}

/// The row name a repeater has when it has none of its own: `row` for the
/// first, `row2`, `row3`... for the others.
pub fn default_row_name(k: usize) -> String {
    if k == 0 {
        "row".to_owned()
    } else {
        format!("row{}", k + 1)
    }
}

/// Whether `name` can be the name of a context: a Python identifier.
pub fn is_context_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c == '_' || c.is_ascii_alphabetic())
        && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

impl FormDesign {
    /// The width of a canvas form, in points.
    pub fn form_width(&self) -> u32 {
        self.width
            .unwrap_or(DEFAULT_FORM_WIDTH)
            .clamp(*FORM_WIDTHS.start(), *FORM_WIDTHS.end())
    }

    /// The section the control at the top level `index` is, on a canvas
    /// form.
    pub fn section(&self, index: usize) -> Option<Section> {
        (self.canvas && index < self.controls.len()).then(|| section_at(index, self.controls.len()))
    }

    /// The sections of a canvas form, in the order of the stack; empty for a
    /// form that is not one.
    pub fn sections(&self) -> Vec<Section> {
        if !self.canvas {
            return Vec::new();
        }
        (0..self.controls.len())
            .map(|i| section_at(i, self.controls.len()))
            .collect()
    }

    /// The top-level indices of the repeater sections, in order.
    pub fn repeater_sections(&self) -> Vec<usize> {
        self.sections()
            .iter()
            .enumerate()
            .filter(|(_, s)| s.is_repeater())
            .map(|(i, _)| i)
            .collect()
    }

    /// The row name of the repeater section at the top level `index`.
    pub fn row_name_of(&self, index: usize) -> Option<String> {
        let Some(Section::Repeater(k)) = self.section(index) else {
            return None;
        };
        let own = self.controls[index].row_name.clone();
        Some(own.unwrap_or_else(|| default_row_name(k)))
    }

    /// Every row name, in the order of the repeaters.
    pub fn row_names(&self) -> Vec<String> {
        self.repeater_sections()
            .into_iter()
            .filter_map(|i| self.row_name_of(i))
            .collect()
    }

    /// The context fields, with their addresses, in the order of the form.
    pub fn context_fields(&self) -> Vec<(Address, &Control)> {
        self.walk()
            .into_iter()
            .filter(|(_, c)| c.kind == ControlKind::Field && c.context.is_some())
            .collect()
    }

    /// The names of every context a path can use: the context fields and the
    /// row names.
    pub fn context_names(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .context_fields()
            .into_iter()
            .filter_map(|(_, c)| c.context.clone())
            .collect();
        out.extend(self.row_names());
        out
    }

    /// A row name that no repeater or context field has yet.
    fn free_row_name(&self) -> String {
        let taken = self.context_names();
        (0..)
            .map(default_row_name)
            .find(|n| !taken.contains(n))
            .expect("some name is free")
    }

    /// Adds a repeater section over the array at `path` before the footer,
    /// after a new fixed section if there is a repeater already. Returns its
    /// top-level index. None if the form is not a canvas.
    pub fn add_repeater(&mut self, path: &str) -> Option<usize> {
        if !self.canvas || self.controls.len() < 2 {
            return None;
        }
        let row_name = self.free_row_name();
        let footer = self.controls.len() - 1;
        let mut at = footer;
        if !self.repeater_sections().is_empty() {
            self.controls.insert(at, new_section(0));
            at += 1;
        }
        self.controls
            .insert(at, new_repeater(path, Some(row_name), ROW + MARGIN));
        Some(at)
    }

    /// Takes away the repeater section at the top level `index`, with the
    /// fixed section under it (above it, for the last repeater) and what is
    /// on them. Returns whether there was one.
    pub fn remove_repeater(&mut self, index: usize) -> bool {
        let Some(Section::Repeater(_)) = self.section(index) else {
            return false;
        };
        let count = self.repeater_sections().len();
        let last = self.controls.len() - 1;
        if count == 1 {
            self.controls.remove(index);
        } else if index + 1 < last {
            self.controls.remove(index + 1);
            self.controls.remove(index);
        } else {
            self.controls.remove(index);
            self.controls.remove(index - 1);
        }
        true
    }

    /// The new empty canvas form `width` points wide: a header and a footer.
    pub fn new_canvas(width: Option<u32>) -> FormDesign {
        FormDesign {
            version: CANVAS_VERSION,
            title: None,
            canvas: true,
            width,
            controls: vec![new_section(ROW * 2), new_section(0)],
        }
    }

    /// Why the control at `address` is wrong on a canvas form, or on a form
    /// that is not one, for what only a canvas has. Paths that use a context
    /// must name one the form has.
    pub(super) fn canvas_problem(
        &self,
        spec: &TypeSpec,
        address: &[usize],
        control: &Control,
    ) -> Option<String> {
        let _ = spec; // for checks to come that read the schema
        let top = address.len() == 1;
        if self.canvas && top {
            let index = address[0];
            let wanted = section_at(index, self.controls.len());
            let fits = match wanted {
                Section::Repeater(_) => control.is_section(),
                _ => control.is_free(),
            };
            if !fits || self.controls.len() < 2 {
                return Some(
                    tr!("the sections of a form are its header, its repeaters with a section between each two, and its footer")
                        .to_owned(),
                );
            }
            if wanted.is_repeater() && control.lists_items() {
                return Some(
                    tr!("a repeater section needs a path to an array, not items").to_owned(),
                );
            }
        }
        if control.kind == ControlKind::Repeater && !(self.canvas && top) && self.canvas {
            return Some(tr!("on a canvas form a repeater is a section of its own").to_owned());
        }
        if control.is_section() && !(self.canvas && top) {
            return Some(tr!("only a canvas form has repeater sections").to_owned());
        }
        if let Some(name) = &control.row_name {
            if !self.canvas || !control.is_section() {
                return Some(tr!("only a repeater section has a row name").to_owned());
            }
            let clash = self
                .context_fields()
                .iter()
                .any(|(_, c)| c.context.as_deref() == Some(name.as_str()))
                || self.row_names().iter().filter(|n| *n == name).count() > 1;
            if clash {
                return Some(tr!("another context is called {name}", name = name));
            }
        }
        if control.kind == ControlKind::RowButton {
            let Some(&index) = address.first().filter(|_| self.canvas && address.len() > 1) else {
                return Some(tr!("an add/delete button goes on a repeater section").to_owned());
            };
            if !self.controls.get(index).is_some_and(Control::is_section) {
                return Some(tr!("an add/delete button goes on a repeater section").to_owned());
            }
            let first = self
                .walk()
                .into_iter()
                .find(|(at, c)| at.first() == Some(&index) && c.kind == ControlKind::RowButton)
                .map(|(at, _)| at);
            if first.as_deref() != Some(address) {
                return Some(tr!("a repeater has one add/delete button").to_owned());
            }
        }
        if let Some(name) = &control.context {
            if control.kind != ControlKind::Field {
                return Some(tr!("only a field can be a context field").to_owned());
            }
            if !self.canvas || address.first() != Some(&0) {
                return Some(tr!("a context field goes on the form header").to_owned());
            }
            if control.path.is_some() || control.value.is_some() {
                return Some(tr!("a context field has no path and no value").to_owned());
            }
            if !is_context_name(name) {
                return Some(tr!("{name} is not a name", name = format!("{name:?}")));
            }
            if self.context_names().iter().filter(|n| *n == name).count() > 1 {
                return Some(tr!("another context is called {name}", name = name));
            }
            if let Err(e) = control.context_start() {
                return Some(tr!("its default: {e}", e = e));
            }
        }
        // The contexts a path uses must be the form's.
        if matches!(
            control.kind,
            ControlKind::Field | ControlKind::Group | ControlKind::Repeater
        ) && let Some(text) = control.path.as_deref()
            && let Ok(parsed) = text.parse::<ContextPath>()
        {
            let names = self.context_names();
            for name in parsed.names() {
                if !names.iter().any(|n| n == name) {
                    return Some(tr!(
                        "the form has no context field or row name called {name}",
                        name = name
                    ));
                }
                if top
                    && control.is_section()
                    && self.row_name_of(address[0]).as_deref() == Some(name)
                {
                    return Some(tr!("a repeater's path cannot use its own row name").to_owned());
                }
            }
        }
        None
    }
}

// --------------------------------------------- turning a form into canvases

/// What [`to_canvas`] could not keep.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Converted {
    pub design: FormDesign,
    /// The repeaters over an items expression that were taken out, each by
    /// its label, or its items expression when it has none.
    pub dropped: Vec<String>,
}

/// The generated form of a schema, as canvases (DESIGN.md, Form canvases,
/// "The generated form").
pub fn generate_canvas(spec: &TypeSpec) -> FormDesign {
    to_canvas(&super::generate(spec), spec).design
}

/// A repeater taken out of a design, with the path its section has.
struct Taken {
    path: String,
    row_name: String,
    control: Control,
}

/// Writes `outer[row]` followed by `rest`, a path relative to the item.
fn through_row(outer: &str, row: &str, rest: &str) -> String {
    if rest.is_empty() {
        format!("{outer}[{row}]")
    } else if rest.starts_with('[') {
        format!("{outer}[{row}]{rest}")
    } else {
        format!("{outer}[{row}].{rest}")
    }
}

/// Takes every repeater out of `list` and what is in it, in the order of
/// the document, an outer repeater before those in its items. `outer` is the
/// path and row name of the repeater whose items `list` is in.
fn take_repeaters(
    spec: &TypeSpec,
    list: &mut Vec<Control>,
    outer: Option<(&str, &str)>,
    taken: &mut Vec<Taken>,
    dropped: &mut Vec<String>,
) {
    let mut i = 0;
    while i < list.len() {
        if list[i].kind != ControlKind::Repeater {
            take_repeaters(spec, &mut list[i].children, outer, taken, dropped);
            i += 1;
            continue;
        }
        let mut repeater = list.remove(i);
        if repeater.lists_items() || repeater.path.is_none() {
            let name = repeater
                .label
                .clone()
                .or_else(|| repeater.items.clone())
                .unwrap_or_else(|| tr!("a repeater").to_owned());
            dropped.push(name);
            continue;
        }
        let own = repeater.path.clone().unwrap_or_default();
        let path = match outer {
            Some((o, row)) => through_row(o, row, &own),
            None => own,
        };
        let row_name = default_row_name(taken.len());
        let mut children = std::mem::take(&mut repeater.children);
        // Items shown as a table, or as generated, hold the arrays of the
        // schema's items, which become the next sections.
        let designed = repeater.is_stacked() && !children.is_empty();
        if !designed {
            children = item_arrays(spec, &path);
        }
        let at = taken.len();
        taken.push(Taken {
            path: path.clone(),
            row_name: row_name.clone(),
            control: repeater,
        });
        take_repeaters(
            spec,
            &mut children,
            Some((&path, &row_name)),
            taken,
            dropped,
        );
        taken[at].control.children = children;
    }
}

/// Repeaters for the arrays in the items of the array at `path` (a path
/// that can have contexts), reached through objects, with paths relative to
/// the item.
fn item_arrays(spec: &TypeSpec, path: &str) -> Vec<Control> {
    fn visit(
        spec: &TypeSpec,
        ty: crate::TypeId,
        rel: &Path,
        out: &mut Vec<Control>,
        open: &mut Vec<crate::TypeId>,
    ) {
        let Kind::Object { fields } = &spec.get(ty).kind else {
            return;
        };
        for f in fields {
            let child = rel.key(f.name.as_str());
            match &spec.get(f.ty).kind {
                Kind::Array { .. } => {
                    let mut r = Control::bound(ControlKind::Repeater, &child);
                    r.label = Some(f.name.clone());
                    out.push(r);
                }
                Kind::Object { .. } if !open.contains(&f.ty) => {
                    open.push(f.ty);
                    visit(spec, f.ty, &child, out, open);
                    open.pop();
                }
                _ => {}
            }
        }
    }
    let item = sample_path(spec, &Path::root(), path)
        .ok()
        .flatten()
        .and_then(|p| spec.resolve(&p))
        .and_then(|r| match &spec.get(r.ty).kind {
            Kind::Array { items, .. } => Some(*items),
            _ => None,
        });
    let mut out = Vec::new();
    if let Some(ty) = item {
        visit(spec, ty, &Path::root(), &mut out, &mut vec![ty]);
    }
    out
}

/// How tall a control is drawn, near enough, when its height is automatic:
/// what the converter leaves between the controls it places one under
/// another, so that nothing overlaps.
fn estimate_height(spec: &TypeSpec, base: &Path, c: &Control) -> u32 {
    if let Some(h) = c.height {
        return h;
    }
    match c.kind {
        ControlKind::Field => {
            let map = c
                .path
                .as_deref()
                .and_then(|t| sample_path(spec, base, t).ok().flatten())
                .and_then(|p| spec.resolve(&p))
                .is_some_and(|r| matches!(spec.get(r.ty).kind, Kind::Map { .. }));
            let body = if map {
                120
            } else if c.editor == Some(Editor::Multiline) {
                72
            } else {
                ROW
            };
            body + if c.label_position == Some(LabelPosition::Top) {
                18
            } else {
                0
            }
        }
        ControlKind::Label => 20,
        ControlKind::Button | ControlKind::RowButton | ControlKind::Repeater => 28,
        ControlKind::Separator => 9,
        ControlKind::Chart => CHART_SIZE.1,
        ControlKind::Group => {
            let base = match c.path.as_deref() {
                Some(t) => sample_path(spec, base, t)
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| base.clone()),
                None => base.clone(),
            };
            let title = if c.label.is_some() { 24 } else { 0 };
            let padding = c.padding.unwrap_or(8) * 2;
            let spacing = c.spacing.unwrap_or(6);
            let inside = if c.is_free() {
                c.children
                    .iter()
                    .map(|k| k.y.unwrap_or(0) + estimate_height(spec, &base, k))
                    .max()
                    .unwrap_or(0)
            } else {
                let columns = c.columns.unwrap_or(1).max(1) as usize;
                let rows = flow_rows(&c.children, columns);
                let tall: u32 = rows
                    .iter()
                    .map(|row| {
                        row.cells
                            .iter()
                            .map(|&(i, _, _)| estimate_height(spec, &base, &c.children[i]))
                            .max()
                            .unwrap_or(0)
                    })
                    .sum();
                tall + spacing * rows.len().saturating_sub(1) as u32
            };
            title + padding + inside
        }
    }
}

/// Places `controls` one under another on a new section `width` points
/// wide, each where a one-column Flow would put it, groups as wide as the
/// section: the section, as tall as they are.
fn column_section(spec: &TypeSpec, base: &Path, controls: Vec<Control>, width: u32) -> Control {
    let mut section = new_section(0);
    let mut y = if controls.is_empty() { 0 } else { MARGIN };
    for mut c in controls {
        let height = estimate_height(spec, base, &c);
        c.x = Some(MARGIN);
        c.y = Some(y);
        if c.kind == ControlKind::Group || c.fill {
            c.width = Some(width.saturating_sub(2 * MARGIN).max(24));
            c.fill = false;
        }
        c.span = None;
        c.new_row = false;
        y += height + MARGIN;
        section.children.push(c);
    }
    section.height = Some(y);
    section
}

/// The row of fields of a repeater shown as a table: a field for each
/// column shown, side by side (in more lines when they do not fit), the
/// titles for the section above, and an add/delete button at the end.
fn table_row(spec: &TypeSpec, taken: &Taken, width: u32) -> (Control, Vec<Control>) {
    let item = sample_path(spec, &Path::root(), &taken.path)
        .ok()
        .flatten()
        .and_then(|p| spec.resolve(&p))
        .and_then(|r| match &spec.get(r.ty).kind {
            Kind::Array { items, .. } => Some(*items),
            _ => None,
        });
    let names: Vec<String> = match item {
        Some(ty) => {
            let fields = crate::report::scalar_fields(spec, ty);
            if fields.is_empty() && !matches!(spec.get(ty).kind, Kind::Object { .. }) {
                vec![String::new()]
            } else {
                fields.iter().map(|f| f.join(".")).collect()
            }
        }
        None => vec![String::new()],
    };
    let columns: Vec<super::ColumnDesign> = taken
        .control
        .arrange_columns(&names)
        .into_iter()
        .filter(|c| !c.hidden && !c.is_row_number())
        .collect();
    let inner = width.saturating_sub(3 * MARGIN + ROW_BUTTON_WIDTH);
    let count = columns.len().max(1) as u32;
    let per_line = count.min((inner / (MIN_COLUMN + MARGIN)).max(1));
    let column_w = (inner.saturating_sub(MARGIN * (per_line - 1)) / per_line).max(24);
    let lines = count.div_ceil(per_line);
    let mut row = new_repeater(
        &taken.path,
        Some(taken.row_name.clone()),
        lines * (ROW + MARGIN / 2) + MARGIN / 2,
    );
    let mut titles = Vec::new();
    for (i, column) in columns.iter().enumerate() {
        let i = i as u32;
        let x = MARGIN + (i % per_line) * (column_w + MARGIN);
        let y = MARGIN / 2 + (i / per_line) * (ROW + MARGIN / 2);
        let mut field = Control::new(ControlKind::Field);
        field.path = Some(column.name.clone());
        field.label_position = Some(LabelPosition::None);
        field.editor = column.editor;
        field.read_only = column.read_only;
        field.thousands = column.thousands;
        field.decimals = column.decimals;
        field.date_format = column.date_format.clone();
        field.percent = column.percent;
        field.prefix = column.prefix.clone();
        field.suffix = column.suffix.clone();
        field.align = column.align;
        field.font = column.font;
        field.font_size = column.font_size;
        field.bold = column.bold;
        field.italic = column.italic;
        field.visible_when = column.visible_when.clone();
        field.enabled_when = column.enabled_when.clone();
        field.text_colour = column.text_colour.clone();
        field.fill_colour = column.fill_colour.clone();
        field.choices = column.choices.clone();
        field.allow_other = column.allow_other;
        field.x = Some(x);
        field.y = Some(y);
        field.width = Some(column_w);
        field.height = Some(ROW);
        row.children.push(field);
        let mut title = Control::new(ControlKind::Label);
        title.text = Some(column.label.clone().unwrap_or_else(|| {
            if column.name.is_empty() {
                taken
                    .control
                    .label
                    .clone()
                    .unwrap_or_else(|| tr!("Item").to_owned())
            } else {
                column.name.clone()
            }
        }));
        title.help = column.help.clone();
        title.bold = Some(true);
        title.align = column.title_align;
        title.x = Some(x);
        title.y = Some((i / per_line) * ROW);
        title.width = Some(column_w);
        title.height = Some(ROW);
        titles.push(title);
    }
    let mut button = Control::new(ControlKind::RowButton);
    button.x = Some(width.saturating_sub(MARGIN + ROW_BUTTON_WIDTH));
    button.y = Some(MARGIN / 2);
    button.width = Some(ROW_BUTTON_WIDTH);
    button.height = Some(ROW);
    row.children.push(button);
    (row, titles)
}

/// The section of a repeater whose items were designed: its controls placed
/// one under another on the row, as on the header.
fn designed_row(spec: &TypeSpec, taken: Taken, width: u32) -> Control {
    let base = sample_path(spec, &Path::root(), &taken.path)
        .ok()
        .flatten()
        .map(|p| p.index(0))
        .unwrap_or_default();
    let Taken {
        path,
        row_name,
        control,
    } = taken;
    let column = column_section(spec, &base, control.children, width);
    let mut row = new_repeater(&path, Some(row_name), column.height.unwrap_or(0));
    row.children = column.children;
    let mut button = Control::new(ControlKind::RowButton);
    button.x = Some(width.saturating_sub(MARGIN + ROW_BUTTON_WIDTH));
    button.y = Some(MARGIN);
    button.width = Some(ROW_BUTTON_WIDTH);
    button.height = Some(ROW);
    row.children.push(button);
    row.item_name = control.item_name;
    row.list_name = control.list_name;
    row
}

/// Puts `titles` at the bottom of `section`, making it taller.
fn add_titles(section: &mut Control, titles: Vec<Control>) {
    if titles.is_empty() {
        return;
    }
    let top = section.height.unwrap_or(0);
    let mut bottom = top;
    for mut t in titles {
        let y = top + t.y.unwrap_or(0);
        bottom = bottom.max(y + t.height.unwrap_or(ROW));
        t.y = Some(y);
        section.children.push(t);
    }
    section.height = Some(bottom);
}

/// A design as canvases (DESIGN.md, Form canvases, "Converting existing
/// designs"): what it has outside repeaters on the header, placed where a
/// one-column Flow puts it at the form's width, and each repeater over a
/// path a section of its own, in the order of the document, a repeater in
/// the items of another reading its array through the outer row name. A
/// design that is a canvas already is given back as it is.
pub fn to_canvas(design: &FormDesign, spec: &TypeSpec) -> Converted {
    if design.canvas {
        return Converted {
            design: design.clone(),
            dropped: Vec::new(),
        };
    }
    let width = design.form_width();
    let mut controls = design.controls.clone();
    let mut taken = Vec::new();
    let mut dropped = Vec::new();
    take_repeaters(spec, &mut controls, None, &mut taken, &mut dropped);
    let mut out = FormDesign {
        version: CANVAS_VERSION,
        title: design.title.clone(),
        canvas: true,
        width: design.width,
        controls: vec![column_section(spec, &Path::root(), controls, width)],
    };
    for (k, t) in taken.into_iter().enumerate() {
        if k > 0 {
            out.controls.push(new_section(0));
        }
        let section = if t.control.is_stacked() && !t.control.children.is_empty() {
            designed_row(spec, t, width)
        } else {
            let item_name = t.control.item_name.clone();
            let list_name = t.control.list_name.clone();
            let (mut row, titles) = table_row(spec, &t, width);
            row.item_name = item_name;
            row.list_name = list_name;
            let above = out.controls.last_mut().expect("the header is there");
            add_titles(above, titles);
            row
        };
        out.controls.push(section);
    }
    out.controls.push(new_section(0));
    Converted {
        design: out,
        dropped,
    }
}
