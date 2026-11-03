//! Placing the controls of a report (DESIGN.md, Reports, "Single page and
//! multipage reports" and "The report designer"): the sizes and places of
//! the controls, and the lines of their text, in points from the top left of
//! the report. The result is a list of things to paint, which the window
//! draws (and, later, the PDF), so the two place everything the same way.
//!
//! The values come from a [`Source`]: the results of the report's
//! expressions, or (in later steps) its results cache. The layout runs no
//! Python and does not depend on the workbook.
//!
//! The placement rules of groups are the forms' ([`form::flow_rows`],
//! [`form::grid_cells`], [`form::free_place`]); only the measuring differs.

use cliotree_core::Value;
use cliotree_core::form::{
    self, Address, Alignment, CONTROL_HEIGHTS, CONTROL_WIDTHS, Control, ControlKind,
    DEFAULT_LABEL_WIDTH, FontFamily as Typeface, Format, GroupLayout, LabelPosition, POSITIONS,
    template,
};
use cliotree_core::report::{BandKind, ReportDesign, ReportKind, new_band, paginate};

use crate::text::{Fonts, TextStyle};

/// The size of text in a report when nothing sets it, in points.
pub const BODY_SIZE: f32 = 12.0;

/// The space between the controls of a group when it sets none, in points.
pub const SPACING: f32 = 6.0;

/// The padding inside a group with a border or a fill, in points.
pub const PADDING: f32 = 8.0;

/// The space between a field's label on the left and its value.
const LABEL_GAP: f32 = 8.0;

/// The space between a title and what it is the title of.
const TITLE_GAP: f32 = 4.0;

/// The narrowest a column of a table is made when the table is shrunk.
const MIN_COLUMN: f32 = 24.0;

/// The thickness of the lines of a table and of a separator.
const RULE: f32 = 0.75;

/// A rectangle, in points.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x <= self.right() && y >= self.y && y <= self.bottom()
    }

    fn moved(self, dx: f32, dy: f32) -> Rect {
        Rect {
            x: self.x + dx,
            y: self.y + dy,
            ..self
        }
    }
}

/// A colour: one of the theme's, which the painter chooses, or a colour of
/// the design or of an expression, as red, green, blue and alpha.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ink {
    /// The text colour.
    Text,
    /// Text that matters less: a message in place of a value.
    Weak,
    /// An error value, `#ERROR!` or `#UNTRUSTED!`.
    Error,
    /// The lines of separators and borders.
    Rule,
    Rgba([u8; 4]),
}

/// One thing to paint.
#[derive(Debug, Clone, PartialEq)]
pub enum Paint {
    /// One line of text, as it was broken here: `x` and `y` are the top left
    /// of its row.
    Text {
        x: f32,
        y: f32,
        text: String,
        style: TextStyle,
        ink: Ink,
    },
    Fill {
        rect: Rect,
        ink: Ink,
    },
    Border {
        rect: Rect,
        width: f32,
        ink: Ink,
    },
    Line {
        from: (f32, f32),
        to: (f32, f32),
        width: f32,
        ink: Ink,
    },
    /// The marker of a value waiting to be calculated, with its top right
    /// corner at `x`, `y`.
    Pending {
        x: f32,
        y: f32,
    },
    /// The SVG a chart function returned, checked by
    /// [`crate::chart::parse_svg`], scaled into `rect` keeping its
    /// proportions.
    /// `style` is the style sheet that gives it the chart's colours (see
    /// [`crate::chart::style_sheet`]).
    Svg {
        rect: Rect,
        svg: std::sync::Arc<str>,
        style: Option<std::sync::Arc<str>>,
    },
}

/// Text shown on hover over a part of the report: why a value is an error.
#[derive(Debug, Clone, PartialEq)]
pub struct Hover {
    pub rect: Rect,
    pub text: String,
}

/// A control as it was placed: where, and which control (its address in the
/// design and the item of each repeater around it).
#[derive(Debug, Clone, PartialEq)]
pub struct Area {
    pub rect: Rect,
    pub at: Address,
    pub rows: Vec<usize>,
}

/// A report laid out: its size, what to paint, the hovers and where each
/// control is.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Laid {
    pub width: f32,
    pub height: f32,
    pub paints: Vec<Paint>,
    pub hovers: Vec<Hover>,
    pub areas: Vec<Area>,
    /// The inside of each Free group, and of the canvas (the empty
    /// address), where the designer places what is dropped or moved.
    pub insides: Vec<Area>,
    /// The boxes of the controls whose content was cut because it is taller
    /// than the box (DESIGN.md, Reports, "Fixed boxes").
    pub cuts: Vec<Rect>,
    /// In the designer of a multipage report: each band, by its place in the
    /// stack, where it is drawn (a hidden band as a thin strip).
    pub bands: Vec<(usize, Rect)>,
}

impl Laid {
    fn append(&mut self, other: Laid, dx: f32, dy: f32) {
        for paint in other.paints {
            self.paints.push(match paint {
                Paint::Text {
                    x,
                    y,
                    text,
                    style,
                    ink,
                } => Paint::Text {
                    x: x + dx,
                    y: y + dy,
                    text,
                    style,
                    ink,
                },
                Paint::Fill { rect, ink } => Paint::Fill {
                    rect: rect.moved(dx, dy),
                    ink,
                },
                Paint::Border { rect, width, ink } => Paint::Border {
                    rect: rect.moved(dx, dy),
                    width,
                    ink,
                },
                Paint::Line {
                    from,
                    to,
                    width,
                    ink,
                } => Paint::Line {
                    from: (from.0 + dx, from.1 + dy),
                    to: (to.0 + dx, to.1 + dy),
                    width,
                    ink,
                },
                Paint::Pending { x, y } => Paint::Pending {
                    x: x + dx,
                    y: y + dy,
                },
                Paint::Svg { rect, svg, style } => Paint::Svg {
                    rect: rect.moved(dx, dy),
                    svg,
                    style,
                },
            });
        }
        self.hovers.extend(other.hovers.into_iter().map(|h| Hover {
            rect: h.rect.moved(dx, dy),
            text: h.text,
        }));
        self.areas.extend(other.areas.into_iter().map(|a| Area {
            rect: a.rect.moved(dx, dy),
            ..a
        }));
        self.insides.extend(other.insides.into_iter().map(|a| Area {
            rect: a.rect.moved(dx, dy),
            ..a
        }));
        self.cuts
            .extend(other.cuts.into_iter().map(|r| r.moved(dx, dy)));
    }

    /// All the text, line by line in the order it is painted: for tests, and
    /// for finding a report's text.
    pub fn texts(&self) -> Vec<&str> {
        self.paints
            .iter()
            .filter_map(|p| match p {
                Paint::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }
}

/// Which expression of a control.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Prop {
    Visible,
    TextColour,
    FillColour,
    Value,
    Template,
    Items,
    /// The SVG of a chart.
    Chart,
}

/// What an expression came to, as the report shows it.
#[derive(Debug, Clone, PartialEq)]
pub enum Got {
    /// The control has no such expression.
    None,
    /// It has not been calculated yet.
    Waiting,
    /// Its value, and whether it is waiting to be calculated again.
    Value(Value, bool),
    Error(String),
    /// It runs Python, and the workbook is not trusted.
    Untrusted,
}

impl Got {
    fn pending(&self) -> bool {
        matches!(self, Got::Waiting | Got::Value(_, true))
    }
}

/// Where the values of a report's expressions come from.
pub trait Source {
    /// An expression of the control at `at`, inside the designed items
    /// `rows` of the repeaters around it.
    fn control(&self, at: &[usize], rows: &[usize], prop: Prop) -> Got;
    /// An expression of a column of the table of the control at `at`: for
    /// the row, or with none for the column (*visible when*).
    fn column(
        &self,
        at: &[usize],
        rows: &[usize],
        column: &str,
        row: Option<usize>,
        prop: Prop,
    ) -> Got;
    /// The largest SVG a chart may return, in bytes.
    fn svg_limit(&self) -> usize {
        crate::chart::SVG_LIMIT
    }
}

/// The text shown for an expression that is not run in a workbook that is
/// not trusted.
pub const UNTRUSTED: &str = "#UNTRUSTED!";

/// Lays out a single page report `width` points wide: its controls one under
/// the other, as a form's top level is (a Flow group of one column).
pub fn single_page(design: &ReportDesign, width: f32, source: &dyn Source) -> Laid {
    let fonts = Fonts::shared();
    let mut placer = Placer { fonts, source };
    let style = Style::body();
    let width = width.max(MIN_COLUMN);
    let mut top = Control::new(ControlKind::Group);
    top.children = design.layout.controls.clone();
    let mut laid = placer.flow(&top, &[], &[], width, &style);
    laid.width = width;
    laid
}

/// Lays out a single page report on its canvas (DESIGN.md, Reports, "Single
/// page"): as wide as the paper less the margins, each control at the top
/// level at its place and in its box, as in a Free group. The canvas is as
/// tall as its lowest control, or its own height if that is more.
pub fn canvas(design: &ReportDesign, source: &dyn Source) -> Laid {
    let fonts = Fonts::shared();
    let mut placer = Placer { fonts, source };
    let width = design.page.content_width().max(MIN_COLUMN);
    let mut top = Control::new(ControlKind::Group);
    top.layout = Some(GroupLayout::Free);
    top.children = design.layout.controls.clone();
    let mut laid = placer.free(&top, &[], &[], width, &Style::body());
    laid.width = width;
    laid.height = laid.height.max(design.height.map_or(0.0, |h| h as f32));
    laid.insides.insert(
        0,
        Area {
            rect: Rect::new(0.0, 0.0, width, laid.height),
            at: Vec::new(),
            rows: Vec::new(),
        },
    );
    laid
}

/// The height a hidden band is drawn in the designer, so that its bottom
/// edge can be dragged down to show it again.
pub const HIDDEN_BAND: f32 = 8.0;

/// The space between the bands in the designer.
pub const BAND_GAP: f32 = 4.0;

/// The items the expressions of a band are shown for in the designer: the
/// first item of each section and the first detail item, and the first page.
fn first_rows(kind: BandKind, sections: usize) -> Vec<usize> {
    match kind {
        BandKind::ReportHeader | BandKind::ReportFooter => Vec::new(),
        BandKind::PageHeader | BandKind::PageFooter => vec![0],
        BandKind::SectionHeader(k) | BandKind::SectionFooter(k) => vec![0; k + 1],
        BandKind::Detail => vec![0; sections + 1],
    }
}

/// A source in which every band is visible, for the designer.
struct AllBands<'a>(&'a dyn Source);

impl Source for AllBands<'_> {
    fn svg_limit(&self) -> usize {
        self.0.svg_limit()
    }

    fn control(&self, at: &[usize], rows: &[usize], prop: Prop) -> Got {
        if at.len() == 1 && prop == Prop::Visible {
            return Got::None;
        }
        self.0.control(at, rows, prop)
    }

    fn column(
        &self,
        at: &[usize],
        rows: &[usize],
        column: &str,
        row: Option<usize>,
        prop: Prop,
    ) -> Got {
        self.0.column(at, rows, column, row, prop)
    }
}

/// Lays out the stack of bands of a multipage report for the designer
/// (DESIGN.md, The report designer, "A multipage report in layout mode"):
/// each band a canvas as wide as the page less its margins, at its height,
/// one under the other, drawn with the first item of each section and the
/// first page. A hidden band is a thin strip.
pub fn bands(design: &ReportDesign, source: &dyn Source) -> Laid {
    let Some(sections) = design.bands() else {
        return canvas(design, source);
    };
    let fonts = Fonts::shared();
    let all = AllBands(source);
    let mut placer = Placer {
        fonts,
        source: &all,
    };
    let width = design.page.content_width().max(MIN_COLUMN);
    let mut out = Laid {
        width,
        ..Laid::default()
    };
    let mut y = 0.0;
    for (i, kind) in BandKind::stack(sections).into_iter().enumerate() {
        let band = &design.layout.controls[i];
        let h = band.height.unwrap_or(0) as f32;
        let rect = Rect::new(0.0, y, width, h.max(HIDDEN_BAND));
        if h > 0.0
            && let Some(laid) = placer.control(
                band,
                &[i],
                &first_rows(kind, sections),
                width,
                &Style::body(),
            )
        {
            out.append(laid, 0.0, y);
        } else {
            // A hidden band can still be selected, and things dropped on it
            // go in it.
            let area = Area {
                rect,
                at: vec![i],
                rows: Vec::new(),
            };
            out.areas.push(area.clone());
            out.insides.push(area);
        }
        out.bands.push((i, rect));
        y = rect.bottom() + BAND_GAP;
    }
    out.height = (y - BAND_GAP).max(0.0);
    out
}

/// A multipage report laid out page by page.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Paged {
    /// Each page, in points from the top left of the paper, margins
    /// included.
    pub pages: Vec<Laid>,
    /// The size of the paper.
    pub width: f32,
    pub height: f32,
    /// What the report says about itself: bands cut at the end of a page,
    /// and the limit of detail bands.
    pub warnings: Vec<String>,
}

/// Lays out a multipage report page by page (DESIGN.md, Reports, "Breaking
/// into pages"), as the PDF will have it: the bands where
/// [`paginate`] puts them, each with the values of its place, inside the
/// margins of the paper. The report stops after `limit` detail bands and
/// says so on its last page.
pub fn pages(design: &ReportDesign, source: &dyn Source, limit: usize) -> Paged {
    let (paper_w, paper_h) = design.page.size();
    let mut out = Paged {
        width: paper_w,
        height: paper_h,
        ..Paged::default()
    };
    let Some(sections) = design.bands() else {
        return out;
    };
    let level_band = |level: usize| {
        if level < sections {
            BandKind::SectionHeader(level).index(sections)
        } else {
            BandKind::Detail.index(sections)
        }
    };
    let items = |level: usize, rows: &[usize]| match source.control(
        &[level_band(level)],
        rows,
        Prop::Items,
    ) {
        Got::Value(Value::Array(items), _) => items.len(),
        _ => 0,
    };
    let visible = |band: usize, rows: &[usize]| match source.control(&[band], rows, Prop::Visible) {
        Got::Value(v, _) => form::truthy(&v),
        _ => true,
    };
    let paging = paginate(design, &items, &visible, limit);
    let fonts = Fonts::shared();
    let mut placer = Placer { fonts, source };
    let width = design.page.content_width().max(MIN_COLUMN);
    let (left, top) = (
        design.page.margins.left as f32,
        design.page.margins.top as f32,
    );
    let last = paging.pages.len().saturating_sub(1);
    for (p, placed) in paging.pages.iter().enumerate() {
        let mut page = Laid {
            width: paper_w,
            height: paper_h,
            ..Laid::default()
        };
        for band in placed {
            let mut control = design.layout.controls[band.band].clone();
            control.height = Some(band.height.max(0.0).floor() as u32);
            if let Some(laid) =
                placer.control(&control, &[band.band], &band.rows, width, &Style::body())
            {
                page.append(laid, left, top + band.y);
            }
        }
        if p == last && paging.limited {
            placer.text(
                &mut page,
                &format!("The report stops here: it has reached the limit of {limit} detail rows."),
                Style::body().text_style(),
                Ink::Error,
                (left, top + paging.end + 4.0, width),
                Alignment::Left,
            );
        }
        out.pages.push(page);
    }
    for band in &paging.cut {
        out.warnings.push(format!(
            "The band {} is taller than the space between the page header and the page \
             footer, so it is cut at the end of the page.",
            design.band_name(*band)
        ));
    }
    if paging.limited {
        out.warnings.push(format!(
            "The report stops after {limit} detail rows, the limit for a report."
        ));
    }
    out
}

/// A design made before the canvas (its top level a Flow list) turned into
/// a canvas: each control at the top level placed where the Flow layout put
/// it at the canvas width, with the width it had, and with the height it had
/// when `heights` (the results are final), else an automatic height. A
/// control that is not visible goes under the one before it. A multipage
/// report becomes its stack of bands, with those controls in the report
/// header.
pub fn to_canvas(design: &ReportDesign, source: &dyn Source, heights: bool) -> ReportDesign {
    let placed = flow_to_canvas(design, source, heights);
    if design.kind != ReportKind::Multipage {
        return placed;
    }
    let mut out = ReportDesign::new(ReportKind::Multipage);
    out.version = design.version;
    out.page = design.page;
    let lowest = placed
        .layout
        .controls
        .iter()
        .map(|c| c.y.unwrap_or(0) + c.height.unwrap_or(24))
        .max()
        .unwrap_or(0);
    let mut header = new_band(lowest);
    header.children = placed.layout.controls;
    out.layout.title = design.layout.title.clone();
    out.layout.controls[BandKind::ReportHeader.index(0)] = header;
    out
}

fn flow_to_canvas(design: &ReportDesign, source: &dyn Source, heights: bool) -> ReportDesign {
    let width = design.page.content_width().max(MIN_COLUMN);
    let laid = single_page(design, width, source);
    let mut out = design.clone();
    out.canvas = true;
    let points = |v: f32, range: &std::ops::RangeInclusive<u32>| {
        (v.ceil().max(0.0) as u32).clamp(*range.start(), *range.end())
    };
    let mut next_y = 0.0f32;
    for (i, c) in out.layout.controls.iter_mut().enumerate() {
        let rect = laid
            .areas
            .iter()
            .find(|a| a.at == [i])
            .map(|a| a.rect)
            .unwrap_or(Rect::new(0.0, next_y, width, 0.0));
        c.x = Some(points(rect.x.floor(), &POSITIONS));
        c.y = Some(points(rect.y.floor(), &POSITIONS));
        c.width = Some(points(rect.w, &CONTROL_WIDTHS));
        if heights && rect.h > 0.0 {
            c.height = Some(points(rect.h, &CONTROL_HEIGHTS));
        }
        c.fill = false;
        c.anchor_x = None;
        c.anchor_y = None;
        next_y = rect.bottom() + SPACING;
    }
    out
}

/// What controls take from the group they are in.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Style {
    typeface: Typeface,
    size: f32,
    bold: bool,
    italic: bool,
    label_width: f32,
    /// The colour of text, when a group sets one.
    text: Option<Ink>,
}

impl Style {
    /// The style of the top level.
    fn body() -> Style {
        Style {
            typeface: Typeface::Sans,
            size: BODY_SIZE,
            bold: false,
            italic: false,
            label_width: DEFAULT_LABEL_WIDTH as f32,
            text: None,
        }
    }

    fn of(&self, c: &Control) -> Style {
        Style {
            typeface: c.font.unwrap_or(self.typeface),
            size: c.font_size.map_or(self.size, |s| s as f32),
            bold: c.bold.unwrap_or(self.bold),
            italic: c.italic.unwrap_or(self.italic),
            label_width: c.label_width.map_or(self.label_width, |w| w as f32),
            text: self.text,
        }
    }

    fn text_style(&self) -> TextStyle {
        TextStyle {
            typeface: self.typeface,
            size: self.size,
            bold: self.bold,
            italic: self.italic,
        }
    }

    fn ink(&self) -> Ink {
        self.text.unwrap_or(Ink::Text)
    }
}

struct Placer<'a> {
    fonts: &'a Fonts,
    source: &'a dyn Source,
}

/// A colour of the design (`#rrggbb`) or of an expression.
fn colour(fixed: Option<&str>, got: &Got) -> Result<Option<Ink>, String> {
    if let Some([r, g, b]) = fixed.and_then(form::fixed_colour) {
        return Ok(Some(Ink::Rgba([r, g, b, 255])));
    }
    match got {
        Got::Value(value, _) => form::colour_of(value).map(|c| c.map(Ink::Rgba)),
        Got::Error(e) => Err(e.clone()),
        _ => Ok(None),
    }
}

/// On a fill with no text colour of its own, text is black or white,
/// whichever reads on it.
fn readable_on(fill: Ink) -> Option<Ink> {
    let Ink::Rgba([r, g, b, _]) = fill else {
        return None;
    };
    let brightness = 0.299 * f32::from(r) + 0.587 * f32::from(g) + 0.114 * f32::from(b);
    Some(if brightness > 140.0 {
        Ink::Rgba([0x20, 0x20, 0x20, 255])
    } else {
        Ink::Rgba([255, 255, 255, 255])
    })
}

/// A value as text: in the format when it applies, otherwise as the data
/// views write it. Lists and records are not shown.
pub fn value_text(value: &Value, format: &Format) -> String {
    if let Some(text) = format.apply(value) {
        return text;
    }
    match value {
        Value::Null => String::new(),
        Value::Bool(true) => "True".to_owned(),
        Value::Bool(false) => "False".to_owned(),
        Value::Int(_) | Value::Float(_) => cliotree_core::format::plain_number(value),
        Value::Str(s) => s.clone(),
        Value::Date(d) => d.format("%Y-%m-%d").to_string(),
        Value::DateTime(cliotree_core::DateTimeValue::Naive(n)) => {
            n.format("%Y-%m-%dT%H:%M:%S%.f").to_string()
        }
        Value::DateTime(cliotree_core::DateTimeValue::Zoned(z)) => z.to_rfc3339(),
        Value::Array(_) | Value::Object(_) => String::new(),
        Value::Error(e) => e.kind.code().to_owned(),
    }
}

/// The text, its colour and why, for a value an expression gave.
fn shown(got: &Got, format: &Format) -> (String, Option<Ink>, Option<String>) {
    match got {
        Got::None | Got::Waiting => (String::new(), None, None),
        Got::Value(value, _) => (value_text(value, format), None, None),
        Got::Error(e) => ("#ERROR!".to_owned(), Some(Ink::Error), Some(e.clone())),
        Got::Untrusted => (
            UNTRUSTED.to_owned(),
            Some(Ink::Error),
            Some("The report runs Python, so this workbook has to be trusted first.".to_owned()),
        ),
    }
}

fn is_number(value: &Value) -> bool {
    matches!(value, Value::Int(_) | Value::Float(_))
}

impl Placer<'_> {
    /// Text broken into lines `width` wide, painted at the top left, aligned
    /// in that width. Returns its height.
    fn text(
        &self,
        laid: &mut Laid,
        text: &str,
        style: TextStyle,
        ink: Ink,
        (x, y, width): (f32, f32, f32),
        align: Alignment,
    ) -> f32 {
        let layout = self.fonts.layout(text, &style, Some(width.max(1.0)));
        let row = layout.metrics.row_height;
        for (i, line) in layout.lines.iter().enumerate() {
            let shift = match align {
                Alignment::Left => 0.0,
                Alignment::Center => ((width - line.width) / 2.0).max(0.0),
                Alignment::Right => (width - line.width).max(0.0),
            };
            let piece = text[line.range.clone()].trim_end();
            if piece.is_empty() {
                continue;
            }
            laid.paints.push(Paint::Text {
                x: x + shift,
                y: y + row * i as f32,
                text: piece.to_owned(),
                style,
                ink,
            });
        }
        row * layout.lines.len().max(1) as f32
    }

    /// The width a control would take if it had the room: its text, for a
    /// label or a field; all of `room` for the rest.
    fn natural_width(&self, c: &Control, style: &Style, room: f32) -> f32 {
        if let Some(w) = c.width {
            return w as f32;
        }
        let style = style.of(c);
        let measure = |t: &str| self.fonts.width(t, &style.text_style());
        let natural = match c.kind {
            ControlKind::Label => measure(c.text.as_deref().unwrap_or_default()),
            ControlKind::Field => {
                let label = c.label.as_deref().unwrap_or_default();
                let value = 80.0;
                match c.label_position.unwrap_or(LabelPosition::Left) {
                    LabelPosition::Left if !label.is_empty() => {
                        style.label_width + LABEL_GAP + value
                    }
                    LabelPosition::Top => measure(label).max(value),
                    _ => value,
                }
            }
            ControlKind::Separator if c.vertical => {
                (c.thickness.map_or(RULE, |t| t as f32) + 4.0).max(9.0)
            }
            ControlKind::Chart => crate::chart::CHART_SIZE.0,
            _ => room,
        };
        natural.min(room).max(1.0)
    }

    /// Lays out a control `width` wide at the top left, or None if it is not
    /// visible.
    fn control(
        &mut self,
        c: &Control,
        at: &[usize],
        rows: &[usize],
        width: f32,
        outer: &Style,
    ) -> Option<Laid> {
        let source = self.source;
        let visible = source.control(at, rows, Prop::Visible);
        if let Got::Value(v, _) = &visible
            && !form::truthy(v)
        {
            return None;
        }
        let mut style = outer.of(c);
        let mut why: Vec<String> = Vec::new();
        if let Got::Error(e) = &visible {
            why.push(format!("Visible when: {e}"));
        }
        let text_got = source.control(at, rows, Prop::TextColour);
        let fill_got = source.control(at, rows, Prop::FillColour);
        let text = colour(c.text_colour.as_deref(), &text_got).unwrap_or_else(|e| {
            why.push(format!("Text colour: {e}"));
            None
        });
        let fill = colour(c.fill_colour.as_deref(), &fill_got).unwrap_or_else(|e| {
            why.push(format!("Fill colour: {e}"));
            None
        });
        // A group's fill is drawn by the group, inside its border.
        let fill_here = fill.filter(|_| c.kind != ControlKind::Group);
        if let Some(ink) = text.or_else(|| fill.and_then(readable_on)) {
            style.text = Some(ink);
        }
        let width = c.width.map_or(width, |w| (w as f32).min(width)).max(1.0);
        let mut laid = match c.kind {
            ControlKind::Label => self.label(c, at, rows, width, &style),
            ControlKind::Field => self.field(c, at, rows, width, &style),
            ControlKind::Separator => self.separator(c, width),
            ControlKind::Chart => self.chart(c, at, rows, width, style.text, fill_here),
            ControlKind::Group => self.group(c, at, rows, width, &style, fill),
            // A report has no repeaters or buttons (see
            // `ReportDesign::problems`): a list is the detail band of a
            // multipage report.
            ControlKind::Repeater | ControlKind::Button => Laid::default(),
        };
        laid.width = width;
        if let Some(h) = c.height {
            let h = h as f32;
            // A fixed box: what does not fit is cut, and says so.
            if laid.height > h + 0.5 {
                self.cut(&mut laid, h, width);
            }
            laid.height = h;
        }
        let rect = Rect::new(0.0, 0.0, width, laid.height);
        if let Some(ink) = fill_here {
            laid.paints.insert(0, Paint::Fill { rect, ink });
        }
        let pending = [&visible, &text_got, &fill_got]
            .iter()
            .any(|g| matches!(g, Got::Value(_, true)));
        if pending {
            laid.paints.push(Paint::Pending { x: width, y: 0.0 });
        }
        if !why.is_empty() {
            laid.hovers.push(Hover {
                rect,
                text: why.join("\n"),
            });
        }
        laid.areas.insert(
            0,
            Area {
                rect,
                at: at.to_vec(),
                rows: rows.to_vec(),
            },
        );
        Some(laid)
    }

    /// A chart: the SVG its function returned, scaled into its box, or in
    /// its place why there is none. `text` and `fill` are its colours, given
    /// to the SVG by a style sheet.
    fn chart(
        &mut self,
        c: &Control,
        at: &[usize],
        rows: &[usize],
        width: f32,
        text: Option<Ink>,
        fill: Option<Ink>,
    ) -> Laid {
        let mut laid = Laid::default();
        let height = c
            .height
            .map_or(crate::chart::CHART_SIZE.1, |h| h as f32)
            .max(1.0);
        laid.height = height;
        let rect = Rect::new(0.0, 0.0, width, height);
        let got = self.source.control(at, rows, Prop::Chart);
        let problem: Option<(String, Ink, Option<String>)> = match &got {
            Got::Value(Value::Str(svg), _) => {
                match crate::chart::parse_svg(svg, self.source.svg_limit()) {
                    Ok(_) => {
                        laid.paints.push(Paint::Svg {
                            rect,
                            svg: std::sync::Arc::from(svg.as_str()),
                            style: crate::chart::style_sheet(text, fill).map(std::sync::Arc::from),
                        });
                        None
                    }
                    Err(e) => Some(("#VALUE!".to_owned(), Ink::Error, Some(e))),
                }
            }
            Got::Value(other, _) => Some((
                "#TYPE!".to_owned(),
                Ink::Error,
                Some(format!(
                    "#TYPE!: a chart function returns SVG as text, not a value of kind {}",
                    other.kind_name()
                )),
            )),
            Got::Error(e) => {
                let code = e
                    .split(':')
                    .next()
                    .filter(|c| c.starts_with('#'))
                    .unwrap_or("#ERROR!")
                    .to_owned();
                Some((code, Ink::Error, Some(e.clone())))
            }
            Got::Untrusted => Some((
                UNTRUSTED.to_owned(),
                Ink::Error,
                Some("The chart runs Python, so this workbook has to be trusted first.".to_owned()),
            )),
            Got::Waiting => Some((String::new(), Ink::Weak, None)),
            Got::None => Some((
                if c.chart.is_some() {
                    "Fill in the chart's values".to_owned()
                } else {
                    "Choose a chart type".to_owned()
                },
                Ink::Weak,
                None,
            )),
        };
        if let Some((text, ink, why)) = problem {
            laid.paints.push(Paint::Border {
                rect,
                width: RULE,
                ink: Ink::Rule,
            });
            let style = Style::body().text_style();
            let line = self.fonts.metrics(&style).row_height;
            let y = ((height - line) / 2.0).max(0.0);
            self.text(
                &mut laid,
                &text,
                style,
                ink,
                (4.0, y, (width - 8.0).max(1.0)),
                Alignment::Center,
            );
            laid.height = height;
            if let Some(why) = why {
                laid.hovers.push(Hover { rect, text: why });
            }
        }
        if got.pending() {
            laid.paints.push(Paint::Pending { x: width, y: 0.0 });
        }
        laid
    }

    fn label(
        &mut self,
        c: &Control,
        at: &[usize],
        rows: &[usize],
        width: f32,
        style: &Style,
    ) -> Laid {
        let mut laid = Laid::default();
        let fixed = c.text.as_deref().unwrap_or_default();
        let (text, ink, why) = if c.is_template() {
            match self.source.control(at, rows, Prop::Template) {
                // A template with no expression in it is its text.
                Got::None => (template_text(fixed, None), style.ink(), None),
                Got::Untrusted => (
                    template_text(fixed, Some(UNTRUSTED)),
                    style.ink(),
                    Some(
                        "The template runs Python, so this workbook has to be trusted first."
                            .to_owned(),
                    ),
                ),
                Got::Waiting => (template_text(fixed, Some("")), Ink::Weak, None),
                Got::Value(Value::Str(s), _) => (s, style.ink(), None),
                Got::Value(other, _) => (value_text(&other, &Format::default()), style.ink(), None),
                Got::Error(e) => ("#ERROR!".to_owned(), Ink::Error, Some(e)),
            }
        } else {
            (fixed.to_owned(), style.ink(), None)
        };
        let align = c.align.unwrap_or(Alignment::Left);
        laid.height = self.text(
            &mut laid,
            &text,
            style.text_style(),
            ink,
            (0.0, 0.0, width),
            align,
        );
        if self.source.control(at, rows, Prop::Template).pending() {
            laid.paints.push(Paint::Pending { x: width, y: 0.0 });
        }
        if let Some(why) = why {
            laid.hovers.push(Hover {
                rect: Rect::new(0.0, 0.0, width, laid.height),
                text: why,
            });
        }
        laid
    }

    fn field(
        &mut self,
        c: &Control,
        at: &[usize],
        rows: &[usize],
        width: f32,
        style: &Style,
    ) -> Laid {
        let mut laid = Laid::default();
        let got = self.source.control(at, rows, Prop::Value);
        let (text, ink, why) = shown(&got, &c.format());
        let ink = ink.unwrap_or(style.ink());
        let number = matches!(&got, Got::Value(v, _) if is_number(v));
        let align = c.align.unwrap_or(if number {
            Alignment::Right
        } else {
            Alignment::Left
        });
        let label = c.label.as_deref().filter(|l| !l.is_empty());
        let position = match label {
            Some(_) => c.label_position.unwrap_or(LabelPosition::Left),
            None => LabelPosition::None,
        };
        let ts = style.text_style();
        let (value_x, value_y, value_w, label_h) = match (position, label) {
            (LabelPosition::Left, Some(text)) => {
                let label_w = style.label_width.min(width * 0.6);
                let h = self.text(
                    &mut laid,
                    text,
                    ts,
                    style.ink(),
                    (0.0, 0.0, label_w),
                    Alignment::Left,
                );
                let x = label_w + LABEL_GAP;
                (x, 0.0, (width - x).max(1.0), h)
            }
            (LabelPosition::Top, Some(text)) => {
                let h = self.text(
                    &mut laid,
                    text,
                    ts.bold(),
                    style.ink(),
                    (0.0, 0.0, width),
                    Alignment::Left,
                );
                (0.0, h + 2.0, width, h + 2.0)
            }
            _ => (0.0, 0.0, width, 0.0),
        };
        let value_h = self.text(
            &mut laid,
            &text,
            ts,
            ink,
            (value_x, value_y, value_w),
            align,
        );
        laid.height = match position {
            LabelPosition::Left => label_h.max(value_h),
            _ => value_y + value_h,
        };
        let value_rect = Rect::new(value_x, value_y, value_w, value_h);
        if got.pending() {
            laid.paints.push(Paint::Pending {
                x: value_rect.right(),
                y: value_rect.y,
            });
        }
        if let Some(why) = why {
            laid.hovers.push(Hover {
                rect: value_rect,
                text: format!("Value: {why}"),
            });
        }
        laid
    }

    /// A separator: a line of its thickness (thin when it has none), with
    /// some room on each side.
    fn separator(&mut self, c: &Control, width: f32) -> Laid {
        let mut laid = Laid::default();
        let thickness = c.thickness.map_or(RULE, |t| t as f32);
        // Half the room it takes across its line.
        let half = (thickness / 2.0 + 2.0).max(4.0);
        if c.vertical {
            let h = c.height.map_or(BODY_SIZE * 1.5, |h| h as f32);
            let x = (width / 2.0).min(half + 0.5);
            laid.paints.push(Paint::Line {
                from: (x, 0.0),
                to: (x, h),
                width: thickness,
                ink: Ink::Rule,
            });
            laid.height = h;
        } else {
            laid.paints.push(Paint::Line {
                from: (0.0, half),
                to: (width, half),
                width: thickness,
                ink: Ink::Rule,
            });
            laid.height = 2.0 * half;
        }
        laid
    }

    /// A title over a group or a repeater: bold, a size larger. Returns its
    /// height with the gap under it.
    fn title(&self, laid: &mut Laid, c: &Control, width: f32, style: &Style) -> f32 {
        let Some(text) = c.label.as_deref().filter(|t| !t.trim().is_empty()) else {
            return 0.0;
        };
        let size = c.title_font_size.map_or(style.size * 1.2, |s| s as f32);
        let ts = TextStyle {
            typeface: c.title_font.unwrap_or(style.typeface),
            size,
            bold: true,
            italic: style.italic,
        };
        self.text(
            laid,
            text,
            ts,
            style.ink(),
            (0.0, 0.0, width),
            Alignment::Left,
        ) + TITLE_GAP
    }

    fn group(
        &mut self,
        c: &Control,
        at: &[usize],
        rows: &[usize],
        width: f32,
        style: &Style,
        fill: Option<Ink>,
    ) -> Laid {
        let mut laid = Laid::default();
        let border = c.border_width.map(|w| w as f32).filter(|w| *w > 0.0);
        let framed = border.is_some() || fill.is_some();
        let pad = c
            .padding
            .map_or(if framed { PADDING } else { 0.0 }, |p| p as f32);
        let inner_w = (width - 2.0 * pad).max(1.0);
        let title_h = self.title(&mut laid, c, inner_w, style);
        let body = if c.is_free() {
            self.free(c, at, rows, inner_w, style)
        } else if c.is_grid() {
            self.grid(c, at, rows, inner_w, style)
        } else {
            self.flow(c, at, rows, inner_w, style)
        };
        let body_h = body.height;
        // The title moves in with the padding.
        let title = std::mem::take(&mut laid);
        laid.append(title, pad, pad);
        laid.append(body, pad, pad + title_h);
        if c.is_free() {
            laid.insides.insert(
                0,
                Area {
                    rect: Rect::new(pad, pad + title_h, inner_w, body_h),
                    at: at.to_vec(),
                    rows: rows.to_vec(),
                },
            );
        }
        laid.height = body_h + title_h + 2.0 * pad;
        let rect = Rect::new(0.0, 0.0, width, laid.height);
        if let Some(ink) = fill {
            laid.paints.insert(0, Paint::Fill { rect, ink });
        }
        if let Some(w) = border {
            let ink = c
                .border_colour
                .as_deref()
                .and_then(form::fixed_colour)
                .map_or(Ink::Rule, |[r, g, b]| Ink::Rgba([r, g, b, 255]));
            laid.paints.push(Paint::Border {
                rect,
                width: w,
                ink,
            });
        }
        laid
    }

    fn gap(c: &Control) -> f32 {
        c.spacing.map_or(SPACING, |s| s as f32)
    }

    /// The children of a group in rows of its columns, as tall as their
    /// tallest control.
    fn flow(
        &mut self,
        c: &Control,
        at: &[usize],
        rows: &[usize],
        width: f32,
        style: &Style,
    ) -> Laid {
        let mut laid = Laid::default();
        let gap = Self::gap(c);
        let columns = c.columns.unwrap_or(1).clamp(1, form::MAX_COLUMNS) as usize;
        let cell = ((width - gap * (columns - 1) as f32) / columns as f32).max(1.0);
        let mut y = 0.0;
        let mut any = false;
        for row in form::flow_rows(&c.children, columns) {
            let mut placed: Vec<(Laid, f32)> = Vec::new();
            for &(i, column, span) in &row.cells {
                let child = &c.children[i];
                let w = cell * span as f32 + gap * (span - 1) as f32;
                let x = (cell + gap) * column as f32;
                let mut child_at = at.to_vec();
                child_at.push(i);
                if let Some(child_laid) = self.control(child, &child_at, rows, w, style) {
                    placed.push((child_laid, x));
                }
            }
            if placed.is_empty() {
                continue;
            }
            if any {
                y += gap;
            }
            any = true;
            let h = placed.iter().map(|(l, _)| l.height).fold(0.0, f32::max);
            for (child_laid, x) in placed {
                laid.append(child_laid, x, y);
            }
            y += h;
        }
        laid.height = y;
        laid
    }

    /// The children of a Grid group in the cells of its rows and columns.
    fn grid(
        &mut self,
        c: &Control,
        at: &[usize],
        rows: &[usize],
        width: f32,
        style: &Style,
    ) -> Laid {
        let mut laid = Laid::default();
        let gap = Self::gap(c);
        let (cells, row_count, column_count) = form::grid_cells(c);
        // Automatic columns are as wide as their widest control.
        let mut content_w = vec![0.0f32; column_count];
        for cell in cells.iter().filter(|cell| cell.columns == 1) {
            let w = self.natural_width(&c.children[cell.index], style, width);
            content_w[cell.column] = content_w[cell.column].max(w);
        }
        let widths = form::track_sizes(&c.grid_columns, &content_w, Some(width), gap);
        let x_of = |k: usize| widths[..k].iter().sum::<f32>() + gap * k as f32;
        let span_w = |k: usize, n: usize| {
            widths[k..(k + n).min(column_count)].iter().sum::<f32>() + gap * (n - 1) as f32
        };
        let mut placed: Vec<(form::GridCell, Laid)> = Vec::new();
        for cell in &cells {
            let mut child_at = at.to_vec();
            child_at.push(cell.index);
            let w = span_w(cell.column, cell.columns).max(1.0);
            if let Some(l) = self.control(&c.children[cell.index], &child_at, rows, w, style) {
                placed.push((*cell, l));
            }
        }
        let mut content_h = vec![0.0f32; row_count];
        for (cell, l) in placed.iter().filter(|(cell, _)| cell.rows == 1) {
            if cell.row < row_count {
                content_h[cell.row] = content_h[cell.row].max(l.height);
            }
        }
        let heights = form::track_sizes(&c.grid_rows, &content_h, None, gap);
        let y_of = |r: usize| heights[..r].iter().sum::<f32>() + gap * r as f32;
        for (cell, l) in placed {
            laid.append(l, x_of(cell.column), y_of(cell.row.min(row_count)));
        }
        laid.height = y_of(row_count) - if row_count > 0 { gap } else { 0.0 };
        laid
    }

    /// The children of a Free group, each at its place.
    fn free(
        &mut self,
        c: &Control,
        at: &[usize],
        rows: &[usize],
        width: f32,
        style: &Style,
    ) -> Laid {
        let mut laid = Laid::default();
        let height = c.height.map_or(0.0, |h| h as f32);
        let mut bottom: f32 = 0.0;
        for (i, child) in c.children.iter().enumerate() {
            let place = form::free_place(c, child, (width, height));
            let room = (width - place.left).max(1.0);
            let w = place
                .width
                .unwrap_or_else(|| self.natural_width(child, style, room));
            let mut child_at = at.to_vec();
            child_at.push(i);
            if let Some(mut l) = self.control(child, &child_at, rows, w, style) {
                if let Some(h) = place.height {
                    l.height = h;
                }
                bottom = bottom.max(place.top + l.height);
                laid.append(l, place.left, place.top);
            }
        }
        laid.height = bottom.max(height);
        laid
    }

    /// Cuts what was laid out to a box `height` tall: what is below it goes,
    /// boxes and lines are shortened, and the last line of text that stays
    /// ends with an ellipsis when text was cut.
    fn cut(&self, laid: &mut Laid, height: f32, width: f32) {
        let limit = height + 0.5;
        let mut text_cut = false;
        let mut kept: Vec<Paint> = Vec::with_capacity(laid.paints.len());
        let clip = |r: Rect| -> Option<Rect> {
            (r.y < height).then(|| Rect {
                h: r.h.min(height - r.y),
                ..r
            })
        };
        for paint in laid.paints.drain(..) {
            match paint {
                Paint::Text { y, style, .. } => {
                    let row = self.fonts.layout("", &style, None).metrics.row_height;
                    if y + row <= limit {
                        kept.push(paint);
                    } else {
                        text_cut = true;
                    }
                }
                Paint::Fill { rect, ink } => {
                    if let Some(rect) = clip(rect) {
                        kept.push(Paint::Fill { rect, ink });
                    }
                }
                // A chart in a group cut short goes whole, or not at all.
                Paint::Svg { rect, svg, style } => {
                    if rect.y + rect.h <= limit {
                        kept.push(Paint::Svg { rect, svg, style });
                    } else {
                        text_cut = true;
                    }
                }
                Paint::Border { rect, width, ink } => {
                    if let Some(rect) = clip(rect) {
                        kept.push(Paint::Border { rect, width, ink });
                    }
                }
                Paint::Line {
                    from,
                    to,
                    width,
                    ink,
                } => {
                    if from.1 <= limit && to.1 <= limit {
                        kept.push(Paint::Line {
                            from,
                            to,
                            width,
                            ink,
                        });
                    } else if from.0 == to.0 && from.1.min(to.1) < height {
                        // A vertical line is shortened.
                        kept.push(Paint::Line {
                            from: (from.0, from.1.min(height)),
                            to: (to.0, to.1.min(height)),
                            width,
                            ink,
                        });
                    }
                }
                Paint::Pending { y, .. } => {
                    if y < height {
                        kept.push(paint);
                    }
                }
            }
        }
        if text_cut {
            let last = kept
                .iter_mut()
                .filter_map(|p| match p {
                    Paint::Text {
                        x, y, text, style, ..
                    } => Some((*x, *y, text, *style)),
                    _ => None,
                })
                .max_by(|a, b| {
                    (a.1, a.0)
                        .partial_cmp(&(b.1, b.0))
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
            if let Some((x, _, text, style)) = last {
                let mut shortened = text.clone();
                loop {
                    let candidate = format!("{}\u{2026}", shortened.trim_end());
                    if shortened.is_empty() || x + self.fonts.width(&candidate, &style) <= width {
                        *text = candidate;
                        break;
                    }
                    shortened.pop();
                }
            }
        }
        laid.paints = kept;
        laid.hovers.retain_mut(|h| match clip(h.rect) {
            Some(r) => {
                h.rect = r;
                true
            }
            None => false,
        });
        laid.areas.retain_mut(|a| match clip(a.rect) {
            Some(r) => {
                a.rect = r;
                true
            }
            None => false,
        });
        laid.insides.retain_mut(|a| match clip(a.rect) {
            Some(r) => {
                a.rect = r;
                true
            }
            None => false,
        });
        laid.cuts.retain(|r| r.y < height);
        let rect = Rect::new(0.0, 0.0, width, height);
        laid.cuts.push(rect);
        laid.hovers.push(Hover {
            rect,
            text: "Cut: what it shows is taller than its box. Make it taller to show all of it."
                .to_owned(),
        });
    }
}

/// A template's text with each expression replaced by `mark` (None: the
/// template has none), and doubled braces made single.
fn template_text(text: &str, mark: Option<&str>) -> String {
    match template::parts(text) {
        Ok(parts) => parts
            .iter()
            .map(|p| match p {
                template::Part::Text(t) => t.clone(),
                _ => mark.unwrap_or_default().to_owned(),
            })
            .collect(),
        Err(_) => text.to_owned(),
    }
}
