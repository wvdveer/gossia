//! Report designs (DESIGN.md, "Reports"): what a report tab holds. A report
//! is a kind (single page or multipage), a page setup and a layout, which is
//! the control tree of forms ([`crate::form`]), so that the report designer
//! can be the form designer. The design is stored as a JSON document
//! validated against `builtin:report`, whose definitions come from
//! `builtin:form`, so a property has one name in both.
//!
//! A report holds no data and edits none: its fields show expressions.
//! [`ReportDesign::problems`] lists what a report cannot have.
//!
//! A multipage report's controls at the top level are its bands, in the
//! order of the designer's stack (DESIGN.md, Reports, "Multipage"): report
//! header, page header, the header of each section from the outermost in,
//! the detail band, the footer of each section from the innermost out, page
//! footer and report footer. Each band is a Free group with a height (0
//! hides it). The items of a section are on its header band (`items`,
//! `item_name`, `list_name`), and those of the detail band on it.
//! [`paginate`] breaks such a report into pages.

mod paging;
mod params;

pub use paging::{DETAIL_LIMIT, Paging, Placed, paginate};
pub use params::{
    ParamKind, PeriodPreset, ReportParam, choice_values, is_empty as param_is_empty, param_text,
    period, period_dates,
};

use gossia_i18n::tr;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::form::{self, Address, Control, ControlKind, DESIGN_VERSION, FormDesign};
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
            ReportKind::SinglePage => tr!("Single page"),
            ReportKind::Multipage => tr!("Multipage"),
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
            Paper::Letter => tr!("Letter"),
            Paper::Legal => tr!("Legal"),
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
    /// The controls, as a form's. On a canvas each control at the top level
    /// has its place and size, as in a Free group; a design made before the
    /// canvas lists them as a Flow group does, and is turned into a canvas
    /// when it is first shown.
    pub layout: FormDesign,
    /// Whether the top level is a canvas (DESIGN.md, Reports, "Single page").
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub canvas: bool,
    /// The height of the canvas in points, when it is dragged taller than its
    /// lowest control.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    /// The options of the sections of a multipage report, the outermost
    /// first. A section without an entry has the default options.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sections: Vec<SectionOptions>,
    /// The parameters the user chooses when the report is shown
    /// (DESIGN.md, Reports, "Report parameters"), in the order of the
    /// parameter bar.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<ReportParam>,
}

/// How a section of a multipage report breaks into pages (DESIGN.md,
/// Reports, "Breaking into pages").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SectionOptions {
    /// Each occurrence starts on a new page.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub new_page: bool,
    /// The header stays on the page of the first band after it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub keep_with_first: bool,
    /// The header is drawn again at the top of a page the section continues
    /// on.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub repeat_header: bool,
}

/// Which band of a multipage report a control at the top level is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BandKind {
    ReportHeader,
    PageHeader,
    /// The header of the section at this depth, 0 the outermost.
    SectionHeader(usize),
    Detail,
    SectionFooter(usize),
    PageFooter,
    ReportFooter,
}

impl BandKind {
    /// The bands of a report with `sections` sections, in the order of the
    /// stack.
    pub fn stack(sections: usize) -> Vec<BandKind> {
        let mut out = vec![BandKind::ReportHeader, BandKind::PageHeader];
        out.extend((0..sections).map(BandKind::SectionHeader));
        out.push(BandKind::Detail);
        out.extend((0..sections).rev().map(BandKind::SectionFooter));
        out.extend([BandKind::PageFooter, BandKind::ReportFooter]);
        out
    }

    /// Where it is in the stack of a report with `sections` sections.
    pub fn index(self, sections: usize) -> usize {
        match self {
            BandKind::ReportHeader => 0,
            BandKind::PageHeader => 1,
            BandKind::SectionHeader(k) => 2 + k,
            BandKind::Detail => 2 + sections,
            BandKind::SectionFooter(k) => 3 + sections + (sections - 1 - k),
            BandKind::PageFooter => 3 + 2 * sections,
            BandKind::ReportFooter => 4 + 2 * sections,
        }
    }

    /// Its name in the designer, without the section's item.
    pub fn label(self) -> &'static str {
        match self {
            BandKind::ReportHeader => tr!("Report header"),
            BandKind::PageHeader => tr!("Page header"),
            BandKind::SectionHeader(_) => tr!("Section header"),
            BandKind::Detail => tr!("Detail"),
            BandKind::SectionFooter(_) => tr!("Section footer"),
            BandKind::PageFooter => tr!("Page footer"),
            BandKind::ReportFooter => tr!("Report footer"),
        }
    }

    /// Whether its band is drawn on every page, and so can use `page`.
    pub fn on_every_page(self) -> bool {
        matches!(self, BandKind::PageHeader | BandKind::PageFooter)
    }
}

/// The heights the bands of a new multipage report start with, in points:
/// the report header and footer are optional, so they start hidden.
const START_HEIGHTS: [(BandKind, u32); 5] = [
    (BandKind::ReportHeader, 0),
    (BandKind::PageHeader, 28),
    (BandKind::Detail, 24),
    (BandKind::PageFooter, 28),
    (BandKind::ReportFooter, 0),
];

/// A new band: a Free group `height` points tall.
pub fn new_band(height: u32) -> Control {
    let mut band = Control::new(ControlKind::Group);
    band.layout = Some(form::GroupLayout::Free);
    band.height = Some(height);
    band
}

/// The number of sections of a layout that is a stack of bands, or None if
/// it is not one: an odd number of controls, at least five, all Free groups.
pub fn band_sections(layout: &FormDesign) -> Option<usize> {
    let n = layout.controls.len();
    let bands = n >= 5 && n % 2 == 1 && layout.controls.iter().all(Control::is_free);
    bands.then(|| (n - 5) / 2)
}

/// Adds a section inside the innermost one (or the first), around the
/// detail band: its header goes just above the detail band and its footer
/// just under it. Returns its depth. None if the layout is not a stack of
/// bands.
pub fn add_section(layout: &mut FormDesign) -> Option<usize> {
    let n = band_sections(layout)?;
    let detail = BandKind::Detail.index(n);
    let mut header = new_band(24);
    header.item_name = Some(if n == 0 {
        "group".to_owned()
    } else {
        format!("group{}", n + 1)
    });
    layout.controls.insert(detail, header);
    // The detail band is now one further down; the footer goes under it.
    layout.controls.insert(detail + 2, new_band(0));
    Some(n)
}

/// Takes away the section at depth `k`, its header and its footer with what
/// is on them. Returns whether there was one.
pub fn remove_section(layout: &mut FormDesign, k: usize) -> bool {
    let Some(n) = band_sections(layout) else {
        return false;
    };
    if k >= n {
        return false;
    }
    let footer = BandKind::SectionFooter(k).index(n);
    let header = BandKind::SectionHeader(k).index(n);
    layout.controls.remove(footer);
    layout.controls.remove(header);
    true
}

impl ReportDesign {
    /// An empty report of `kind`.
    /// A multipage report starts as its stack of bands, without sections.
    pub fn new(kind: ReportKind) -> ReportDesign {
        let mut layout = FormDesign::default();
        if kind == ReportKind::Multipage {
            layout.controls = BandKind::stack(0)
                .into_iter()
                .map(|band| {
                    let height = START_HEIGHTS
                        .iter()
                        .find(|(b, _)| *b == band)
                        .map_or(24, |(_, h)| *h);
                    new_band(height)
                })
                .collect();
        }
        ReportDesign {
            version: REPORT_VERSION,
            kind,
            page: PageSetup::default(),
            layout,
            canvas: kind == ReportKind::Multipage,
            height: None,
            sections: Vec::new(),
            params: Vec::new(),
        }
    }

    /// The number of sections, if this is a multipage report whose controls
    /// are its stack of bands.
    pub fn bands(&self) -> Option<usize> {
        if self.kind != ReportKind::Multipage || !self.canvas {
            return None;
        }
        band_sections(&self.layout)
    }

    /// Which band the control at the top level at `index` is.
    pub fn band_kind(&self, index: usize) -> Option<BandKind> {
        BandKind::stack(self.bands()?).get(index).copied()
    }

    /// The name of a band in the designer: *Section header: unit*, with the
    /// item name of its section.
    pub fn band_name(&self, index: usize) -> String {
        let Some(kind) = self.band_kind(index) else {
            return String::new();
        };
        match kind {
            BandKind::SectionHeader(k) | BandKind::SectionFooter(k) => {
                let n = self.bands().unwrap_or(0);
                let header = BandKind::SectionHeader(k).index(n);
                let item = self.layout.controls[header]
                    .item_name
                    .as_deref()
                    .map(str::trim)
                    .filter(|t| !t.is_empty())
                    .map_or_else(|| format!("{}", k + 1), str::to_owned);
                format!("{}: {item}", kind.label())
            }
            _ => kind.label().to_owned(),
        }
    }

    /// The options of the section at depth `k`.
    pub fn section_options(&self, k: usize) -> SectionOptions {
        self.sections.get(k).copied().unwrap_or_default()
    }

    /// The design as the JSON document it is stored as, pretty-printed.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("a design always serializes")
    }

    /// Reads a report document. It must be JSON that conforms to
    /// `builtin:report`; the reason is given in words if it does not.
    pub fn from_json(text: &str) -> Result<ReportDesign, String> {
        let parsed = json::parse(text).map_err(|e| tr!("not valid JSON: {e}", e = e))?;
        if let Some(issue) = validate(report_schema(), &parsed.node).first() {
            let at = if issue.path.is_root() {
                tr!("the document").to_owned()
            } else {
                issue.path.to_string()
            };
            return Err(format!("{at}: {}", issue.message));
        }
        let design: ReportDesign = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if design.version > REPORT_VERSION || design.layout.version > DESIGN_VERSION {
            return Err(tr!(
                "this report was made by a newer version (report version {version}, this build reads \
                 up to {REPORT_VERSION})",
                version = design.version,
                REPORT_VERSION = REPORT_VERSION
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
        if self.kind == ReportKind::Multipage && self.canvas {
            match self.bands() {
                None => out.push((
                    Vec::new(),
                    tr!(
                        "the controls of a multipage report are its bands: report header, page \
                     header, a header and a footer for each section, detail, page footer and \
                     report footer, each a Free group"
                    )
                    .to_owned(),
                )),
                Some(n) => {
                    for k in 0..n {
                        let at = BandKind::SectionHeader(k).index(n);
                        if !has_text(&self.layout.controls[at].items) {
                            out.push((
                                vec![at],
                                tr!(
                                    "section {section} has no items: give its header band an items \
                                     expression",
                                    section = k + 1
                                ),
                            ));
                        }
                    }
                }
            }
        }
        for (i, p) in self.params.iter().enumerate() {
            if let Some(why) = self.param_problem(i, p) {
                out.push((Vec::new(), why));
            }
        }
        for (at, control) in self.layout.walk() {
            if let Some(why) = control_problem(control) {
                out.push((at, why));
            }
        }
        out
    }
}

impl ReportDesign {
    /// What is wrong with the parameter `p`, the `i`th: its name, or a list
    /// to choose from that it needs.
    fn param_problem(&self, i: usize, p: &ReportParam) -> Option<String> {
        let name = p.name.trim();
        if !is_identifier(name) || is_keyword(name) {
            return Some(tr!(
                "the parameter {name} needs a name of letters, digits and _, not starting with a digit",
                name = format!("{name:?}")
            ));
        }
        if self.params[..i].iter().any(|q| q.name.trim() == name) {
            return Some(tr!("two parameters are named {name}", name = name));
        }
        if p.kind.has_choices() && p.choices_expression().is_none() {
            return Some(tr!(
                "the parameter {name} is chosen from a list: give it choices",
                name = name
            ));
        }
        None
    }

    /// The parameter named `name`.
    pub fn param(&self, name: &str) -> Option<&ReportParam> {
        self.params.iter().find(|p| p.name == name)
    }
}

fn has_text(text: &Option<String>) -> bool {
    text.as_deref().is_some_and(|t| !t.trim().is_empty())
}

fn control_problem(c: &Control) -> Option<String> {
    let kind = c.kind.label().to_lowercase();
    if c.kind == ControlKind::Button {
        return Some(tr!("a report has no buttons").to_owned());
    }
    // A list is the detail band of a multipage report (owner, 2026-11-03).
    if c.kind == ControlKind::Repeater {
        return Some(
            tr!(
                "a report has no repeaters: a list of rows is the detail band of a multipage report"
            )
            .to_owned(),
        );
    }
    if c.path.is_some() {
        return Some(match c.kind {
            ControlKind::Field => tr!(
                "a report is not bound to a tab, so this {kind} has a path; in a report it shows a value",
                kind = kind
            ),
            _ => tr!(
                "a report is not bound to a tab, so this {kind} has a path; in a report it has no path",
                kind = kind
            ),
        });
    }
    let editing = [
        (c.editor.is_some(), tr!("an editor")),
        (has_text(&c.choices), "choices"),
        (c.masked, tr!("a masked box")),
        (has_text(&c.enabled_when), tr!("enabled when")),
        (has_text(&c.write_procedure), tr!("a write procedure")),
        (has_text(&c.procedure), "a procedure"),
    ];
    if let Some((_, what)) = editing.iter().find(|(has, _)| *has) {
        return Some(tr!(
            "a report does not edit, so this {kind} cannot have {what}",
            kind = kind,
            what = what
        ));
    }
    for column in &c.table_columns {
        let editing = [
            (column.editor.is_some(), tr!("an editor")),
            (has_text(&column.choices), "choices"),
            (has_text(&column.enabled_when), tr!("enabled when")),
            (has_text(&column.write_procedure), tr!("a write procedure")),
        ];
        if let Some((_, what)) = editing.iter().find(|(has, _)| *has) {
            return Some(tr!(
                "a report does not edit, so the column {name} cannot have {what}",
                name = column.name,
                what = what
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
      "layout": {"$ref": "#/$defs/Form"},
      "canvas": {"type": "boolean"},
      "height": {"type": "integer", "minimum": 0, "maximum": 20000},
      "sections": {"type": "array", "items": {"$ref": "#/$defs/SectionOptions"}},
      "params": {"type": "array", "items": {"$ref": "#/$defs/ReportParam"}}
    },
    "required": ["version", "kind", "layout"]
  },
  "ReportParam": {
    "type": "object",
    "properties": {
      "name": {"type": "string"},
      "label": {"type": "string"},
      "kind": {"type": "string", "enum": ["date", "period", "choice", "choices", "number", "text", "yes_no"]},
      "default": {"type": "string"},
      "preset": {"type": "string", "enum": ["this_month", "last_month", "this_quarter", "last_quarter", "this_year", "last_year", "year_to_date"]},
      "choices": {"type": "string"},
      "required": {"type": "boolean"}
    },
    "required": ["name", "kind"]
  },
  "SectionOptions": {
    "type": "object",
    "properties": {
      "new_page": {"type": "boolean"},
      "keep_with_first": {"type": "boolean"},
      "repeat_header": {"type": "boolean"}
    }
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

/// The height of a row of the fields a report starts with, and its gap.
const START_ROW: u32 = 18;
const START_GAP: u32 = 12;

/// The narrowest column a report starts with.
const START_MIN_COLUMN: u32 = 60;

/// A new multipage report that starts from the array at `array` of the tab
/// `tab` (whose schema is `spec`), as DESIGN.md, The report designer, "A
/// starting point" has it: the detail band repeats for each item
/// (`Orders.items as order`), with a field for each scalar of the item that
/// shows an expression (`order.qty`), side by side, and the page header has
/// a heading over each. The page footer numbers the pages. None if there is
/// no array there.
pub fn starting_design(spec: &TypeSpec, tab: &str, array: &Path) -> Option<ReportDesign> {
    let resolved = spec.resolve(array)?;
    let Kind::Array { items, .. } = &spec.get(resolved.ty).kind else {
        return None;
    };
    let item = item_name(tab, array, &[]);
    let columns = flat_columns(spec, *items).unwrap_or_else(|| scalar_fields(spec, *items));
    let mut design = ReportDesign::new(ReportKind::Multipage);
    let width = design.page.content_width().floor() as u32;
    let count = columns.len().max(1) as u32;
    let fits = (width + START_GAP) / (START_MIN_COLUMN + START_GAP);
    let per_row = count.min(fits.max(1));
    let column_w = (width - START_GAP * (per_row - 1)) / per_row;
    let rows = count.div_ceil(per_row);
    let mut header = new_band(rows * (START_ROW + START_GAP) + 4);
    let mut detail = new_band(rows * (START_ROW + START_GAP));
    detail.items = Some(attribute_chain(tab, array));
    detail.item_name = Some(item.clone());
    for (i, fields) in columns.iter().enumerate() {
        let i = i as u32;
        let x = (i % per_row) * (column_w + START_GAP);
        let y = (i / per_row) * (START_ROW + START_GAP);
        let mut heading = Control::new(ControlKind::Label);
        heading.text = Some(if fields.is_empty() {
            item.clone()
        } else {
            fields.join(".")
        });
        heading.bold = Some(true);
        let mut field = Control::new(ControlKind::Field);
        field.value = Some(read_chain(spec, *items, &item, fields));
        // A number is on the right of its column, under its heading.
        let number = field_type(spec, *items, fields)
            .is_some_and(|t| matches!(spec.get(t).kind, Kind::Integer(_) | Kind::Number(_)));
        for (c, top) in [(&mut heading, y + 4), (&mut field, y)] {
            if number {
                c.align = Some(form::Alignment::Right);
            }
            c.x = Some(x);
            c.y = Some(top);
            c.width = Some(column_w);
            c.height = Some(START_ROW);
        }
        header.children.push(heading);
        detail.children.push(field);
    }
    let mut footer = new_band(28);
    let mut number = Control::new(ControlKind::Label);
    number.template = true;
    number.text = Some("Page {page.number} of {page.count}".to_owned());
    number.align = Some(form::Alignment::Right);
    number.x = Some(width.saturating_sub(160));
    number.y = Some(6);
    number.width = Some(160);
    number.height = Some(START_ROW);
    footer.children.push(number);
    let n = 0;
    design.layout.controls[BandKind::PageHeader.index(n)] = header;
    design.layout.controls[BandKind::Detail.index(n)] = detail;
    design.layout.controls[BandKind::PageFooter.index(n)] = footer;
    Some(design)
}

/// The type reached from `ty` by the field names `fields`.
fn field_type(spec: &TypeSpec, ty: TypeId, fields: &[String]) -> Option<TypeId> {
    let mut here = ty;
    for name in fields {
        here = match &spec.get(here).kind {
            Kind::Object { fields } => fields.iter().find(|f| &f.name == name)?.ty,
            _ => return None,
        };
    }
    Some(here)
}

/// The scalar fields of items of type `ty` that are objects with arrays or
/// maps in them: each as the names that reach it, objects flattened.
fn scalar_fields(spec: &TypeSpec, ty: TypeId) -> Vec<Vec<String>> {
    fn collect(
        spec: &TypeSpec,
        ty: TypeId,
        prefix: &mut Vec<String>,
        out: &mut Vec<Vec<String>>,
        open: &mut Vec<TypeId>,
    ) {
        let Kind::Object { fields } = &spec.get(ty).kind else {
            return;
        };
        for field in fields {
            prefix.push(field.name.clone());
            match &spec.get(field.ty).kind {
                Kind::Object { .. } if !open.contains(&field.ty) => {
                    open.push(field.ty);
                    collect(spec, field.ty, prefix, out, open);
                    open.pop();
                }
                Kind::Object { .. } | Kind::Array { .. } | Kind::Map { .. } => {}
                _ => out.push(prefix.clone()),
            }
            prefix.pop();
        }
    }
    let mut out = Vec::new();
    collect(spec, ty, &mut Vec::new(), &mut out, &mut vec![ty]);
    out
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

/// The expression that reads `fields` from `item`, of type `ty`: a field
/// that may be absent is read with `.get("name")`, so that it gives `None`
/// rather than an error.
fn read_chain(spec: &TypeSpec, ty: TypeId, item: &str, fields: &[String]) -> String {
    let mut out = item.to_owned();
    let mut here = Some(ty);
    for name in fields {
        let field = here.and_then(|t| match &spec.get(t).kind {
            Kind::Object { fields } => fields.iter().find(|f| &f.name == name),
            _ => None,
        });
        let quoted = serde_json::Value::from(name.as_str()).to_string();
        match field {
            Some(f) if !f.required => out.push_str(&format!(".get({quoted})")),
            _ => out = chain_from(&out, std::slice::from_ref(name)),
        }
        here = field.map(|f| f.ty);
    }
    out
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
