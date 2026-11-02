//! Form designs (DESIGN.md, "Forms"): the tree of controls that a form is
//! drawn from, the built-in schema it is stored under (`builtin:form`), the
//! generated form, and the checks of a design against the schema of its tab.
//!
//! A design holds no data. It is a list of controls laid out one under the
//! other; a group arranges its children in rows of one or more columns, where
//! a control can span columns and start a new row. A control can have a
//! width, a height and an alignment in its cell, and a field a label position
//! and width (DESIGN.md, "Forms", "Size and place of a control"). A control is
//! addressed by its place in the tree, a list of indices ([`Address`]).

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
    /// A list to choose from, the items of the field's *choices*.
    Dropdown,
    /// A text box that lists the items of the field's *choices* that match
    /// what is typed.
    Typeahead,
}

impl Editor {
    pub const ALL: [Editor; 5] = [
        Editor::Auto,
        Editor::Text,
        Editor::Multiline,
        Editor::Dropdown,
        Editor::Typeahead,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Editor::Auto => "Automatic",
            Editor::Text => "One line of text",
            Editor::Multiline => "Several lines of text",
            Editor::Dropdown => "Dropdown list",
            Editor::Typeahead => "Type to search",
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

/// The widths a control may be given, in points.
pub const CONTROL_WIDTHS: std::ops::RangeInclusive<u32> = 24..=2000;

/// The heights a control may be given, in points.
pub const CONTROL_HEIGHTS: std::ops::RangeInclusive<u32> = 16..=2000;

/// The widths of the label of a field, in points.
pub const LABEL_WIDTHS: std::ops::RangeInclusive<u32> = 0..=600;

/// The width of the label of a field when nothing sets it, in points.
pub const DEFAULT_LABEL_WIDTH: u32 = 170;

/// The space between the controls of a group, and its padding, in points.
pub const SPACINGS: std::ops::RangeInclusive<u32> = 0..=48;

/// The most columns a group can have.
pub const MAX_COLUMNS: u32 = 6;

/// The positions of a control in a Free group, and the sizes a Free group's
/// places were made for, in points.
pub const POSITIONS: std::ops::RangeInclusive<u32> = 0..=4000;

/// How a group arranges its controls (DESIGN.md, "Forms", "Layout").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GroupLayout {
    /// One after the other, in rows of the group's columns.
    Flow,
    /// Each at a position of its own.
    Free,
    /// In the cells of rows and columns that the group defines.
    Grid,
}

impl GroupLayout {
    pub const ALL: [GroupLayout; 3] = [GroupLayout::Flow, GroupLayout::Free, GroupLayout::Grid];

    pub fn label(self) -> &'static str {
        match self {
            GroupLayout::Flow => "Flow",
            GroupLayout::Free => "Free",
            GroupLayout::Grid => "Grid",
        }
    }
}

/// The most rows or columns a Grid group can have.
pub const MAX_TRACKS: u32 = 50;

/// The sizes a row or column of a Grid group may be given, in points.
pub const TRACK_POINTS: std::ops::RangeInclusive<u32> = 8..=2000;

/// The shares a row or column may take of what is left.
pub const TRACK_SHARES: std::ops::RangeInclusive<u32> = 1..=99;

/// The size of a row or column of a Grid group. Written as `auto`, a number
/// of points (`120`), or a share of what the others leave (`2fr`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum TrackSize {
    /// As big as its biggest control.
    Auto,
    Points(u32),
    /// A share of the room the others leave.
    Share(u32),
}

impl TrackSize {
    pub fn label(self) -> &'static str {
        match self {
            TrackSize::Auto => "Automatic",
            TrackSize::Points(_) => "Points",
            TrackSize::Share(_) => "Share",
        }
    }
}

impl From<TrackSize> for String {
    fn from(size: TrackSize) -> String {
        match size {
            TrackSize::Auto => "auto".to_owned(),
            TrackSize::Points(p) => p.to_string(),
            TrackSize::Share(s) => format!("{s}fr"),
        }
    }
}

impl TryFrom<String> for TrackSize {
    type Error = String;

    fn try_from(text: String) -> Result<TrackSize, String> {
        let wrong =
            || format!("`{text}` is not a size: auto, points such as 120, or a share such as 2fr");
        if text == "auto" {
            Ok(TrackSize::Auto)
        } else if let Some(share) = text.strip_suffix("fr") {
            let s: u32 = share.parse().map_err(|_| wrong())?;
            TRACK_SHARES
                .contains(&s)
                .then_some(TrackSize::Share(s))
                .ok_or_else(wrong)
        } else {
            let p: u32 = text.parse().map_err(|_| wrong())?;
            TRACK_POINTS
                .contains(&p)
                .then_some(TrackSize::Points(p))
                .ok_or_else(wrong)
        }
    }
}

/// Where a control of a Grid group is: its first row and column, from 0,
/// and how many of each it takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridCell {
    /// The index of the control among the group's.
    pub index: usize,
    pub row: usize,
    pub column: usize,
    pub rows: usize,
    pub columns: usize,
}

/// The cells of the controls of a Grid group, and how many rows and columns
/// it has: those it defines, or more if a control is placed beyond them. A
/// control with a row and a column is there; one without is put in the next
/// cell that is free, row by row, after the one before it.
pub fn grid_cells(group: &Control) -> (Vec<GridCell>, usize, usize) {
    let max = MAX_TRACKS as usize;
    let span = |v: Option<u32>| (v.unwrap_or(1).max(1) as usize).min(max);
    let placed = |c: &Control| match (c.grid_row, c.grid_column) {
        (Some(r), Some(k)) => Some((
            (r.max(1) as usize - 1).min(max - 1),
            (k.max(1) as usize - 1).min(max - 1),
        )),
        _ => None,
    };
    let mut columns = group.grid_columns.len().max(1);
    for c in &group.children {
        if let Some((_, k)) = placed(c) {
            columns = columns.max(k + span(c.span));
        }
    }
    let columns = columns.min(max);
    let mut taken: Vec<Vec<bool>> = Vec::new();
    let take = |taken: &mut Vec<Vec<bool>>, cell: &GridCell| {
        for r in cell.row..cell.row + cell.rows {
            while taken.len() <= r {
                taken.push(vec![false; columns]);
            }
            let end = (cell.column + cell.columns).min(columns);
            taken[r][cell.column..end].fill(true);
        }
    };
    let mut cells: Vec<Option<GridCell>> = vec![None; group.children.len()];
    for (index, c) in group.children.iter().enumerate() {
        if let Some((row, column)) = placed(c) {
            let cell = GridCell {
                index,
                row,
                column,
                rows: span(c.row_span),
                columns: span(c.span).min(columns - column.min(columns - 1)),
            };
            take(&mut taken, &cell);
            cells[index] = Some(cell);
        }
    }
    let free = |taken: &Vec<Vec<bool>>, row: usize, column: usize, rows: usize, width: usize| {
        (row..row + rows)
            .all(|r| (column..column + width).all(|k| taken.get(r).is_none_or(|line| !line[k])))
    };
    let (mut row, mut column) = (0, 0);
    for (index, c) in group.children.iter().enumerate() {
        if cells[index].is_some() {
            continue;
        }
        let width = span(c.span).min(columns);
        let rows = span(c.row_span);
        loop {
            if column + width > columns {
                row += 1;
                column = 0;
            }
            if free(&taken, row, column, rows, width) {
                break;
            }
            column += 1;
        }
        let cell = GridCell {
            index,
            row,
            column,
            rows,
            columns: width,
        };
        take(&mut taken, &cell);
        cells[index] = Some(cell);
        column += width;
    }
    let cells: Vec<GridCell> = cells.into_iter().flatten().collect();
    let rows = cells
        .iter()
        .map(|c| c.row + c.rows)
        .max()
        .unwrap_or(0)
        .max(group.grid_rows.len())
        .max(1);
    (cells, rows, columns)
}

/// The sizes of the rows or columns of a Grid group, in points: a number of
/// points is that; automatic is `content`, the size of its biggest control;
/// the shares divide what the others and the gaps leave of `available` (or,
/// with nothing available, are automatic). Tracks beyond `sizes` are
/// automatic.
pub fn track_sizes(
    sizes: &[TrackSize],
    content: &[f32],
    available: Option<f32>,
    gap: f32,
) -> Vec<f32> {
    let n = content.len();
    let size = |i: usize| sizes.get(i).copied().unwrap_or(TrackSize::Auto);
    let mut out: Vec<f32> = (0..n)
        .map(|i| match size(i) {
            TrackSize::Auto => content[i],
            TrackSize::Points(p) => p as f32,
            TrackSize::Share(_) => 0.0,
        })
        .collect();
    let shares: u32 = (0..n)
        .filter_map(|i| match size(i) {
            TrackSize::Share(s) => Some(s),
            _ => None,
        })
        .sum();
    if shares > 0 {
        let used: f32 = out.iter().sum::<f32>() + gap * n.saturating_sub(1) as f32;
        for i in 0..n {
            if let TrackSize::Share(s) = size(i) {
                out[i] = match available {
                    Some(room) => ((room - used).max(0.0) * s as f32 / shares as f32).max(0.0),
                    None => content[i],
                };
            }
        }
    }
    out
}

/// Which edges of a Free group a control keeps its distance to when the
/// group is wider (or taller) than its places were made for: the first
/// edge (left, top), the last (right, bottom), or both, which stretches it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Anchor {
    Start,
    End,
    Both,
}

impl Anchor {
    pub const ALL: [Anchor; 3] = [Anchor::Start, Anchor::End, Anchor::Both];

    pub fn label(self, across: bool) -> &'static str {
        match (self, across) {
            (Anchor::Start, true) => "Left",
            (Anchor::End, true) => "Right",
            (Anchor::Both, true) => "Left and right",
            (Anchor::Start, false) => "Top",
            (Anchor::End, false) => "Bottom",
            (Anchor::Both, false) => "Top and bottom",
        }
    }
}

/// Where a control of a Free group is drawn, in points from the top left of
/// the group's inside.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FreePlace {
    pub left: f32,
    pub top: f32,
    /// Its width and height, when it has them; stretched by its anchors.
    pub width: Option<f32>,
    pub height: Option<f32>,
}

/// How far a group of `size` is from the size its places were made for:
/// across always, down only when the group has a height of its own (else
/// it is as tall as its controls, and nothing is anchored to its bottom).
fn free_growth(group: &Control, size: (f32, f32)) -> (f32, f32) {
    let dx = group.base_width.map_or(0.0, |base| size.0 - base as f32);
    let dy = match (group.height, group.base_height) {
        (Some(_), Some(base)) => size.1 - base as f32,
        _ => 0.0,
    };
    (dx, dy)
}

fn anchored(
    anchor: Option<Anchor>,
    start: f32,
    length: Option<f32>,
    growth: f32,
) -> (f32, Option<f32>) {
    match anchor.unwrap_or(Anchor::Start) {
        Anchor::Start => (start, length),
        Anchor::End => (start + growth, length),
        // A control with no length of its own cannot stretch.
        Anchor::Both => (start, length.map(|l| (l + growth).max(1.0))),
    }
}

/// Where the child `c` of the Free group `group` is drawn when the group's
/// inside is `size`.
pub fn free_place(group: &Control, c: &Control, size: (f32, f32)) -> FreePlace {
    let (dx, dy) = free_growth(group, size);
    let (left, width) = anchored(
        c.anchor_x,
        c.x.unwrap_or(0) as f32,
        c.width.map(|w| w as f32),
        dx,
    );
    let (top, height) = anchored(
        c.anchor_y,
        c.y.unwrap_or(0) as f32,
        c.height.map(|h| h as f32),
        dy,
    );
    FreePlace {
        left: left.max(0.0),
        top: top.max(0.0),
        width,
        height,
    }
}

/// Makes the places of the children of a Free group the ones they are drawn
/// at when its inside is `size`, and that size the one they are made for;
/// so that a change made where they are seen does not move the others.
pub fn rebase_free(group: &mut Control, size: (f32, f32)) {
    let to_u32 = |v: f32, range: &std::ops::RangeInclusive<u32>| {
        (v.round().max(0.0) as u32).clamp(*range.start(), *range.end())
    };
    let places: Vec<FreePlace> = group
        .children
        .iter()
        .map(|c| free_place(group, c, size))
        .collect();
    for (c, place) in group.children.iter_mut().zip(places) {
        if c.x.is_some() || place.left > 0.0 {
            c.x = Some(to_u32(place.left, &POSITIONS));
        }
        if c.y.is_some() || place.top > 0.0 {
            c.y = Some(to_u32(place.top, &POSITIONS));
        }
        if let Some(w) = place.width.filter(|_| c.width.is_some()) {
            c.width = Some(to_u32(w, &CONTROL_WIDTHS));
        }
        if let Some(h) = place.height.filter(|_| c.height.is_some()) {
            c.height = Some(to_u32(h, &CONTROL_HEIGHTS));
        }
    }
    group.base_width = Some(to_u32(size.0, &POSITIONS));
    group.base_height = group.height.map(|_| to_u32(size.1, &POSITIONS));
}

/// Where the label of a field is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LabelPosition {
    Left,
    Top,
    None,
}

impl LabelPosition {
    pub const ALL: [LabelPosition; 3] =
        [LabelPosition::Left, LabelPosition::Top, LabelPosition::None];

    pub fn label(self) -> &'static str {
        match self {
            LabelPosition::Left => "On the left",
            LabelPosition::Top => "Above",
            LabelPosition::None => "None",
        }
    }
}

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
    /// For a number: shown times 100, with `%`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub percent: bool,
    /// For a number: text before and after it, such as `€ ` or ` kg`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suffix: Option<String>,
    /// The font of the cells. Absent: the font of the table. The titles of
    /// the columns are in the table's `header_font`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<FontFamily>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_size: Option<u32>,
    /// Bold and italic text. Absent: as the table's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bold: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub italic: Option<bool>,
    /// Python expressions, without the `=`. *Visible when* runs once for the
    /// table, at the array, and hides the whole column; the others run for
    /// each row, at the cell (`this` is the cell, `parent` the item).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible_when: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled_when: Option<String>,
    /// The colours of a cell's text and background, as for a control.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_colour: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill_colour: Option<String>,
    /// The acceptable values of a cell, as for a control; runs for each row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub choices: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub allow_other: bool,
    /// A Python expression whose result the cells show, instead of a field
    /// of the item; runs for each row. A column with one need not be a field
    /// of the items. Read-only, unless it has a write procedure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// For a column that shows an expression: the procedure that takes what
    /// is typed into a cell, `name(form, ctx, value)`, with `ctx.index` the
    /// row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write_procedure: Option<String>,
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
            percent: false,
            prefix: None,
            suffix: None,
            font: None,
            font_size: None,
            bold: None,
            italic: None,
            visible_when: None,
            enabled_when: None,
            text_colour: None,
            fill_colour: None,
            choices: None,
            allow_other: false,
            value: None,
            write_procedure: None,
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
            percent: self.percent,
            prefix: self.prefix.clone(),
            suffix: self.suffix.clone(),
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
    /// A number times 100, with `%` after it.
    pub percent: bool,
    /// Text before and after a number.
    pub prefix: Option<String>,
    pub suffix: Option<String>,
}

impl Format {
    pub fn is_empty(&self) -> bool {
        self == &Format::default()
    }

    /// What a value looks like in this format, or None if it does not apply.
    pub fn apply(&self, value: &crate::Value) -> Option<String> {
        use crate::Value;
        let number = matches!(value, Value::Int(_) | Value::Float(_));
        let decorated = self.percent || self.prefix.is_some() || self.suffix.is_some();
        if !(number && decorated) {
            return crate::format::formatted(
                value,
                self.thousands,
                self.decimals,
                self.date_format.as_deref(),
            );
        }
        let shown = match value {
            Value::Float(x) if !x.is_finite() => return None,
            Value::Int(n) if self.percent => match n.checked_mul(100) {
                Some(n) => Value::Int(n),
                None => Value::Float(*n as f64 * 100.0),
            },
            // Rounded, so that 0.07 is 7%, not 7.000000000000001%; and a whole
            // percentage, with no decimal places set, is 25%, not 25.0%.
            Value::Float(x) if self.percent => {
                let x = (x * 100.0 * 1e9).round() / 1e9;
                if self.decimals.is_none() && x.fract() == 0.0 && x.abs() < 1e15 {
                    Value::Int(x as i64)
                } else {
                    Value::Float(x)
                }
            }
            other => other.clone(),
        };
        let body = crate::format::number(&shown, self.thousands, self.decimals)
            .unwrap_or_else(|| crate::format::plain_number(&shown));
        Some(format!(
            "{}{body}{}{}",
            self.prefix.as_deref().unwrap_or_default(),
            if self.percent { "%" } else { "" },
            self.suffix.as_deref().unwrap_or_default()
        ))
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
    /// For a label: its text is a template, in which each `{expression}` is
    /// filled in as in a Python f-string (see [`crate::form::template`]).
    #[serde(default, skip_serializing_if = "is_false")]
    pub template: bool,
    /// For a field: a Python expression whose result it shows, instead of the
    /// value of a node. A field has a path or a value, not both; with a value
    /// it is read-only unless it has a write procedure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// For a repeater: a Python expression whose list gives its items,
    /// instead of an array of the tab. Such a repeater is read-only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<String>,
    /// For a repeater: the name the expressions of the controls in it give
    /// the item, and the name they give the whole list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub list_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub editor: Option<Editor>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub read_only: bool,
    /// For a field of a string: a password box, dots instead of the text.
    /// Only what is shown changes; the value is stored as typed.
    #[serde(default, skip_serializing_if = "is_false")]
    pub masked: bool,
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
    /// For a number: shown times 100, with `%`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub percent: bool,
    /// For a number: text before and after it, such as `€ ` or ` kg`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suffix: Option<String>,
    /// For a button: the procedure it runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub procedure: Option<String>,
    /// For a field that shows an expression: the procedure that takes what
    /// is typed into it, `name(form, ctx, value)`. Without one the field is
    /// read-only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write_procedure: Option<String>,
    /// Python expressions, without the `=`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible_when: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled_when: Option<String>,
    /// The colour of the control's text and of its background: a fixed
    /// colour, `#rrggbb` (see [`fixed_colour`]), which runs no Python, or a
    /// Python expression giving a name, `"#rrggbb"` or an `(r, g, b)` tuple,
    /// or `None` for the usual look.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_colour: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill_colour: Option<String>,
    /// For a field: a Python expression giving the acceptable values, for a
    /// dropdown or a typeahead. Each item is a value, a `(code, text)` pair,
    /// or a dict with `value` and `label`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub choices: Option<String>,
    /// For a typeahead with choices: typed text that matches no item is
    /// entered as the value itself, instead of being refused.
    #[serde(default, skip_serializing_if = "is_false")]
    pub allow_other: bool,
    /// The typeface of the control's text. Absent: the one of the group it is
    /// in, or the interface font.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<FontFamily>,
    /// The size of the control's text, in points. Absent: as for `font`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_size: Option<u32>,
    /// Bold and italic text. Absent: as for `font`; `false` undoes the
    /// group's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bold: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub italic: Option<bool>,
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
    /// The width in points. Absent: what the control needs (or `fill`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    /// Takes the whole width of its cell or row; `width` is then ignored.
    #[serde(default, skip_serializing_if = "is_false")]
    pub fill: bool,
    /// The height in points: of a several-lines text box, a table or stacked
    /// repeater, a label or button, or the least height of a group.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    /// Where the control sits in its cell when it is narrower. Absent: left.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub align: Option<Alignment>,
    /// For a field: where its label is. Absent: on the left.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_position: Option<LabelPosition>,
    /// For a field: the width of its label; for a group: of the labels of the
    /// fields in it. Absent: the group's, or [`DEFAULT_LABEL_WIDTH`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_width: Option<u32>,
    /// In a group of several columns: how many it takes. Absent: one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<u32>,
    /// In a group of several columns: starts a new row.
    #[serde(default, skip_serializing_if = "is_false")]
    pub new_row: bool,
    /// For a group: the space between its controls, and inside its box.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spacing: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub padding: Option<u32>,
    /// For a group: the width of its border in points, 0 to 8 (0: none),
    /// and its colour, a fixed colour `#rrggbb`. Absent: the usual look.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border_width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border_colour: Option<String>,
    /// For a separator: a vertical line instead of a horizontal one.
    #[serde(default, skip_serializing_if = "is_false")]
    pub vertical: bool,
    /// For a group: how it arranges its controls. Absent: flow.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<GroupLayout>,
    /// For a Free group: the width and height of its inside that the places
    /// of its controls were made for; anchors keep a control's distance to
    /// an edge when the group is another size.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_height: Option<u32>,
    /// In a Free group: the position, in points from the top left of the
    /// group's inside. Absent: 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<u32>,
    /// In a Free group: the edges it keeps its distance to. Absent: the left
    /// and the top.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor_x: Option<Anchor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor_y: Option<Anchor>,
    /// For a Grid group: the sizes of its columns and rows. A control placed
    /// beyond them adds automatic ones.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub grid_columns: Vec<TrackSize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub grid_rows: Vec<TrackSize>,
    /// In a Grid group: the row and column of its cell, from 1. Absent: the
    /// next free cell. It takes `span` columns and `row_span` rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grid_row: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grid_column: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_span: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Control>,
}

/// One row of a group of several columns: the controls in it, each with the
/// column it starts at and the columns it takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowRow {
    /// `(index of the child, first column, columns taken)`.
    pub cells: Vec<(usize, usize, usize)>,
}

/// Puts the controls of a group into rows of `columns` columns: each takes
/// its span (at most the number of columns), and goes to the next row when
/// it asks for a new row or does not fit in what is left of the row.
pub fn flow_rows(children: &[Control], columns: usize) -> Vec<FlowRow> {
    let columns = columns.max(1);
    let mut rows: Vec<FlowRow> = Vec::new();
    let mut used = columns;
    for (i, child) in children.iter().enumerate() {
        let span = (child.span.unwrap_or(1).max(1) as usize).min(columns);
        if used + span > columns || child.new_row || rows.is_empty() {
            rows.push(FlowRow { cells: Vec::new() });
            used = 0;
        }
        rows.last_mut()
            .expect("a row was pushed")
            .cells
            .push((i, used, span));
        used += span;
    }
    rows
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// Whether an optional text has something in it besides spaces.
fn has_text(text: &Option<String>) -> bool {
    text.as_deref().is_some_and(|t| !t.trim().is_empty())
}

pub mod template;

impl Control {
    /// Whether it is a repeater that shows one item at a time. Its items can
    /// be designed: the controls in it are drawn for the item shown.
    pub fn is_stacked(&self) -> bool {
        self.kind == ControlKind::Repeater && self.mode == Some(RepeaterMode::Stacked)
    }

    /// Whether it is a group that places its controls freely.
    pub fn is_free(&self) -> bool {
        self.kind == ControlKind::Group && self.layout == Some(GroupLayout::Free)
    }

    /// Whether it is a group that places its controls in a grid.
    pub fn is_grid(&self) -> bool {
        self.kind == ControlKind::Group && self.layout == Some(GroupLayout::Grid)
    }

    pub fn new(kind: ControlKind) -> Control {
        Control {
            kind,
            path: None,
            label: None,
            help: None,
            text: None,
            template: false,
            value: None,
            items: None,
            item_name: None,
            list_name: None,
            editor: None,
            read_only: false,
            masked: false,
            columns: None,
            mode: None,
            show: None,
            table_columns: Vec::new(),
            thousands: false,
            decimals: None,
            date_format: None,
            percent: false,
            prefix: None,
            suffix: None,
            procedure: None,
            write_procedure: None,
            visible_when: None,
            enabled_when: None,
            text_colour: None,
            fill_colour: None,
            choices: None,
            allow_other: false,
            font: None,
            font_size: None,
            bold: None,
            italic: None,
            title_font: None,
            title_font_size: None,
            header_font: None,
            header_font_size: None,
            width: None,
            fill: false,
            height: None,
            align: None,
            label_position: None,
            label_width: None,
            span: None,
            new_row: false,
            spacing: None,
            padding: None,
            border_width: None,
            border_colour: None,
            vertical: false,
            layout: None,
            base_width: None,
            base_height: None,
            x: None,
            y: None,
            anchor_x: None,
            anchor_y: None,
            grid_columns: Vec::new(),
            grid_rows: Vec::new(),
            grid_row: None,
            grid_column: None,
            row_span: None,
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

    /// Whether the kind binds to a node. A field that shows an expression and
    /// a repeater whose items come from one do not.
    pub fn binds(&self) -> bool {
        matches!(
            self.kind,
            ControlKind::Field | ControlKind::Group | ControlKind::Repeater
        ) && self.path.is_some()
            && !self.shows_value()
            && !self.lists_items()
    }

    /// Whether it is a field that shows an expression's result.
    pub fn shows_value(&self) -> bool {
        self.kind == ControlKind::Field && has_text(&self.value)
    }

    /// Whether it is a repeater whose items come from an expression.
    pub fn lists_items(&self) -> bool {
        self.kind == ControlKind::Repeater && has_text(&self.items)
    }

    /// Whether it is a label whose text is a template.
    pub fn is_template(&self) -> bool {
        self.kind == ControlKind::Label && self.template
    }

    /// How the field shows a number or a date.
    pub fn format(&self) -> Format {
        Format {
            thousands: self.thousands,
            decimals: self.decimals,
            date_format: self.date_format.clone(),
            percent: self.percent,
            prefix: self.prefix.clone(),
            suffix: self.suffix.clone(),
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

/// The colour a fixed colour property gives: `#rrggbb`, or None if the text
/// is something else (an expression). `#` begins a comment in Python, so no
/// expression is ever a fixed colour.
pub fn fixed_colour(text: &str) -> Option<[u8; 3]> {
    let hex = text.trim().strip_prefix('#')?;
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    Some([byte(0)?, byte(2)?, byte(4)?])
}

// ------------------------------------------- truth and colours of values

/// Python's truth of a value: false for `None`, `False`, zero, and an empty
/// string, list or record.
pub fn truthy(value: &crate::Value) -> bool {
    match value {
        crate::Value::Null => false,
        crate::Value::Bool(b) => *b,
        crate::Value::Int(i) => *i != 0,
        crate::Value::Float(f) => *f != 0.0,
        crate::Value::Str(s) => !s.is_empty(),
        crate::Value::Array(items) => !items.is_empty(),
        crate::Value::Object(fields) => !fields.is_empty(),
        crate::Value::Date(_) | crate::Value::DateTime(_) => true,
        crate::Value::Error(_) => false,
    }
}

/// The colours that can be named, as CSS names them.
const NAMES: &[(&str, [u8; 3])] = &[
    ("black", [0, 0, 0]),
    ("white", [255, 255, 255]),
    ("red", [255, 0, 0]),
    ("green", [0, 128, 0]),
    ("blue", [0, 0, 255]),
    ("yellow", [255, 255, 0]),
    ("orange", [255, 165, 0]),
    ("purple", [128, 0, 128]),
    ("pink", [255, 192, 203]),
    ("brown", [165, 42, 42]),
    ("grey", [128, 128, 128]),
    ("gray", [128, 128, 128]),
    ("silver", [192, 192, 192]),
    ("cyan", [0, 255, 255]),
    ("aqua", [0, 255, 255]),
    ("magenta", [255, 0, 255]),
    ("fuchsia", [255, 0, 255]),
    ("lime", [0, 255, 0]),
    ("maroon", [128, 0, 0]),
    ("navy", [0, 0, 128]),
    ("olive", [128, 128, 0]),
    ("teal", [0, 128, 128]),
    ("gold", [255, 215, 0]),
    ("lightgreen", [144, 238, 144]),
    ("lightblue", [173, 216, 230]),
    ("lightyellow", [255, 255, 224]),
    ("lightgrey", [211, 211, 211]),
    ("lightgray", [211, 211, 211]),
    ("darkred", [139, 0, 0]),
    ("darkgreen", [0, 100, 0]),
    ("darkblue", [0, 0, 139]),
];

/// The colour a value names: a name, `#rgb`, `#rrggbb` or `#rrggbbaa`, or a
/// list of three or four numbers from 0 to 255. `None` and the empty string
/// are the usual colour.
pub fn colour_of(value: &crate::Value) -> Result<Option<[u8; 4]>, String> {
    match value {
        crate::Value::Null => Ok(None),
        crate::Value::Str(s) if s.trim().is_empty() => Ok(None),
        crate::Value::Str(s) => named_colour(s.trim()).map(Some),
        crate::Value::Array(items) if items.len() == 3 || items.len() == 4 => {
            let mut parts = [255u8; 4];
            for (slot, item) in parts.iter_mut().zip(items) {
                *slot = match item.value() {
                    crate::Value::Int(i) if (0..=255).contains(i) => *i as u8,
                    _ => return Err(not_a_colour(value)),
                };
            }
            let [r, g, b, a] = parts;
            Ok(Some([r, g, b, a]))
        }
        _ => Err(not_a_colour(value)),
    }
}

fn not_a_colour(value: &crate::Value) -> String {
    format!(
        "a {} is not a colour; give a name such as \"red\", \"#rrggbb\" or (r, g, b)",
        value.kind_name()
    )
}

fn named_colour(text: &str) -> Result<[u8; 4], String> {
    let bad = || {
        format!("\"{text}\" is not a colour; give a name such as \"red\", \"#rrggbb\" or (r, g, b)")
    };
    if let Some(hex) = text.strip_prefix('#') {
        let digits: Option<Vec<u8>> = hex
            .chars()
            .map(|c| c.to_digit(16).map(|d| d as u8))
            .collect();
        let digits = digits.ok_or_else(bad)?;
        let pair = |i: usize| digits[i] * 16 + digits[i + 1];
        return match digits.len() {
            3 => Ok([digits[0] * 17, digits[1] * 17, digits[2] * 17, 255]),
            6 => Ok([pair(0), pair(2), pair(4), 255]),
            8 => Ok([pair(0), pair(2), pair(4), pair(6)]),
            _ => Err(bad()),
        };
    }
    let lower = text.to_ascii_lowercase();
    NAMES
        .iter()
        .find(|(name, _)| *name == lower)
        .map(|(_, [r, g, b])| [*r, *g, *b, 255])
        .ok_or_else(bad)
}

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
        "template": {"type": "boolean"},
        "value": {"type": "string"},
        "items": {"type": "string"},
        "item_name": {"type": "string", "pattern": "^[A-Za-z_][A-Za-z0-9_]*$"},
        "list_name": {"type": "string", "pattern": "^[A-Za-z_][A-Za-z0-9_]*$"},
        "editor": {"type": "string", "enum": ["auto", "text", "multiline", "dropdown", "typeahead"]},
        "read_only": {"type": "boolean"},
        "masked": {"type": "boolean"},
        "columns": {"type": "integer", "minimum": 1, "maximum": 6},
        "mode": {"type": "string", "enum": ["table", "stacked"]},
        "show": {"type": "array", "items": {"type": "string"}},
        "table_columns": {"type": "array", "items": {"$ref": "#/$defs/Column"}},
        "thousands": {"type": "boolean"},
        "decimals": {"type": "integer", "minimum": 0, "maximum": 10},
        "date_format": {"type": "string", "minLength": 1},
        "percent": {"type": "boolean"},
        "prefix": {"type": "string", "maxLength": 20},
        "suffix": {"type": "string", "maxLength": 20},
        "procedure": {"type": "string"},
        "write_procedure": {"type": "string"},
        "visible_when": {"type": "string"},
        "enabled_when": {"type": "string"},
        "text_colour": {"type": "string"},
        "fill_colour": {"type": "string"},
        "choices": {"type": "string"},
        "allow_other": {"type": "boolean"},
        "font": {"type": "string", "enum": ["sans", "serif", "monospace", "condensed", "script", "handwriting"]},
        "font_size": {"type": "integer", "minimum": 6, "maximum": 72},
        "bold": {"type": "boolean"},
        "italic": {"type": "boolean"},
        "title_font": {"type": "string", "enum": ["sans", "serif", "monospace", "condensed", "script", "handwriting"]},
        "title_font_size": {"type": "integer", "minimum": 6, "maximum": 72},
        "header_font": {"type": "string", "enum": ["sans", "serif", "monospace", "condensed", "script", "handwriting"]},
        "header_font_size": {"type": "integer", "minimum": 6, "maximum": 72},
        "width": {"type": "integer", "minimum": 24, "maximum": 2000},
        "fill": {"type": "boolean"},
        "height": {"type": "integer", "minimum": 16, "maximum": 2000},
        "align": {"type": "string", "enum": ["left", "center", "right"]},
        "label_position": {"type": "string", "enum": ["left", "top", "none"]},
        "label_width": {"type": "integer", "minimum": 0, "maximum": 600},
        "span": {"type": "integer", "minimum": 1, "maximum": 50},
        "new_row": {"type": "boolean"},
        "spacing": {"type": "integer", "minimum": 0, "maximum": 48},
        "padding": {"type": "integer", "minimum": 0, "maximum": 48},
        "border_width": {"type": "integer", "minimum": 0, "maximum": 8},
        "border_colour": {"type": "string", "pattern": "^#[0-9A-Fa-f]{6}$"},
        "vertical": {"type": "boolean"},
        "layout": {"type": "string", "enum": ["flow", "free", "grid"]},
        "grid_columns": {"type": "array", "maxItems": 50, "items": {"$ref": "#/$defs/Track"}},
        "grid_rows": {"type": "array", "maxItems": 50, "items": {"$ref": "#/$defs/Track"}},
        "grid_row": {"type": "integer", "minimum": 1, "maximum": 50},
        "grid_column": {"type": "integer", "minimum": 1, "maximum": 50},
        "row_span": {"type": "integer", "minimum": 1, "maximum": 50},
        "base_width": {"type": "integer", "minimum": 0, "maximum": 4000},
        "base_height": {"type": "integer", "minimum": 0, "maximum": 4000},
        "x": {"type": "integer", "minimum": 0, "maximum": 4000},
        "y": {"type": "integer", "minimum": 0, "maximum": 4000},
        "anchor_x": {"type": "string", "enum": ["start", "end", "both"]},
        "anchor_y": {"type": "string", "enum": ["start", "end", "both"]},
        "children": {"type": "array", "items": {"$ref": "#/$defs/Control"}}
      },
      "required": ["type"]
    },
    "Track": {"type": "string", "pattern": "^(auto|[0-9]{1,4}|[0-9]{1,2}fr)$"},
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
        "editor": {"type": "string", "enum": ["auto", "text", "multiline", "dropdown", "typeahead"]},
        "read_only": {"type": "boolean"},
        "thousands": {"type": "boolean"},
        "decimals": {"type": "integer", "minimum": 0, "maximum": 10},
        "date_format": {"type": "string", "minLength": 1},
        "percent": {"type": "boolean"},
        "prefix": {"type": "string", "maxLength": 20},
        "suffix": {"type": "string", "maxLength": 20},
        "font": {"type": "string", "enum": ["sans", "serif", "monospace", "condensed", "script", "handwriting"]},
        "font_size": {"type": "integer", "minimum": 6, "maximum": 72},
        "bold": {"type": "boolean"},
        "italic": {"type": "boolean"},
        "visible_when": {"type": "string"},
        "enabled_when": {"type": "string"},
        "text_colour": {"type": "string"},
        "fill_colour": {"type": "string"},
        "choices": {"type": "string"},
        "allow_other": {"type": "boolean"},
        "value": {"type": "string"},
        "write_procedure": {"type": "string"},
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
            if !self.takes_children(&target) {
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

    /// Moves a control to `index` in the list of `parent` (a group, or the
    /// top level for an empty address), `index` counting the places as they
    /// are before the move: what dragging a control does. A control cannot go
    /// into itself or anything inside it, and only a group or a stacked
    /// repeater takes children ([`FormDesign::takes_children`]). Returns the
    /// new address, or None if the move cannot be made.
    pub fn move_to(&mut self, from: &[usize], parent: &[usize], index: usize) -> Option<Address> {
        if parent.starts_with(from) {
            return None;
        }
        if !self.takes_children(parent) {
            return None;
        }
        let (&last, from_parent) = from.split_last()?;
        let len = self.list(parent)?.len();
        let mut index = index.min(len);
        self.get(from)?;
        // Taking the control out moves up what comes after it in its list,
        // which may be the target list or a group on the way to it.
        let mut parent = parent.to_vec();
        if parent.as_slice() == from_parent {
            if index > last {
                index -= 1;
            }
        } else if parent.starts_with(from_parent) && parent[from_parent.len()] > last {
            parent[from_parent.len()] -= 1;
        }
        let control = self.remove(from)?;
        self.insert(&parent, Some(index), control)
    }

    /// Whether controls can be put into the one at `address` (the top level
    /// for an empty address): a group, or a stacked repeater whose items are
    /// designed, its controls being that design. Its items are designed
    /// first, from the generated look ([`item_controls`]); until then, and
    /// again once its last control is taken out, it shows them as generated.
    /// A repeater inside the items of another is drawn as generated, so it
    /// takes none.
    pub fn takes_children(&self, address: &[usize]) -> bool {
        if address.is_empty() {
            return true;
        }
        match self.get(address) {
            Some(c) if c.kind == ControlKind::Group => true,
            Some(c) if c.is_stacked() => {
                !c.children.is_empty() && self.item_scope(address).is_none()
            }
            _ => false,
        }
    }

    /// The repeater whose items the control at `address` is designed in: the
    /// address of the nearest repeater it is inside. The path of such a
    /// control is relative to the item, and its expressions run for each
    /// item.
    pub fn item_scope(&self, address: &[usize]) -> Option<Address> {
        (1..address.len())
            .rev()
            .map(|n| &address[..n])
            .find(|a| self.get(a).is_some_and(|c| c.kind == ControlKind::Repeater))
            .map(<[usize]>::to_vec)
    }

    /// The node the control at `address` shows: its own path, or inside the
    /// items of a repeater, its path in item `item` (an unbound control
    /// there is at the item). `control` is the one at `address`, or the
    /// property page's copy that it is editing. None for an unbound control
    /// outside an item, and when the paths cannot be read.
    pub fn path_in_item(&self, address: &[usize], control: &Control, item: usize) -> Option<Path> {
        // Each repeater around, outermost first: item `item` of the nearest,
        // the first item of the others. Inside the items of an expression
        // the paths start again from the top of the tab.
        let scopes: Vec<usize> = (1..address.len())
            .filter(|&n| {
                self.get(&address[..n])
                    .is_some_and(|c| c.kind == ControlKind::Repeater)
            })
            .collect();
        let mut base: Option<Path> = None;
        for (k, &n) in scopes.iter().enumerate() {
            let repeater = self.get(&address[..n])?;
            if repeater.lists_items() {
                base = None;
                continue;
            }
            let path = repeater.bound_path()?;
            let array = match &base {
                Some(b) => b.join(&path),
                None => path,
            };
            base = Some(array.index(if k + 1 == scopes.len() { item } else { 0 }));
        }
        match (base, control.bound_path()) {
            (Some(base), Some(rest)) => Some(base.join(&rest)),
            (Some(base), None) => Some(base),
            (None, path) => path,
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
    // Values the type names (COBOL condition names) are its choices; the
    // field takes others too unless the author says not (owner, 2026-11-01).
    if kind == ControlKind::Field && !spec.get(ty).choices.is_empty() {
        control.allow_other = true;
    }
    if kind == ControlKind::Group {
        control.children = generated_children(spec, ty, path);
    }
    control
}

/// The controls of the generated form for an item of type `ty`, with paths
/// relative to the item: where the design of a stacked repeater's items
/// starts. An item that is not an object is one control, bound to the item
/// itself (the empty path).
pub fn item_controls(spec: &TypeSpec, ty: crate::TypeId) -> Vec<Control> {
    match &spec.get(ty).kind {
        Kind::Object { .. } => generated_children(spec, ty, &Path::root()),
        _ => vec![generated(spec, ty, &Path::root(), "")],
    }
}

/// The paths a control of `kind` can be bound to: a field takes a scalar or a
/// map, a group an object, a repeater an array. Only nodes that exist
/// without an index are offered, and the root is not.
pub fn bindable(spec: &TypeSpec, kind: ControlKind) -> Vec<Path> {
    bindable_from(spec, spec.root(), kind)
}

/// As [`bindable`], relative to a node of type `ty`: for the controls in the
/// items of a repeater, `ty` being the items' type. An item that is not an
/// object offers itself (the empty path) to a field.
pub fn bindable_from(spec: &TypeSpec, ty: crate::TypeId, kind: ControlKind) -> Vec<Path> {
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
    let scalar = !matches!(
        spec.get(ty).kind,
        Kind::Object { .. } | Kind::Array { .. } | Kind::Map { .. }
    );
    if scalar && kind == ControlKind::Field && ty != spec.root() {
        out.push(Path::root());
    }
    visit(spec, ty, &Path::root(), kind, &mut out);
    out
}

// ------------------------------------------------- a design and its schema

impl FormDesign {
    /// Why the control at `address` cannot be drawn against `spec`, or None.
    /// Paths are literal: one the schema does not define is broken, whatever
    /// the schema was before.
    pub fn problem(&self, spec: &TypeSpec, address: &[usize]) -> Option<String> {
        let control = self.get(address)?;
        self.problem_of(spec, address, control)
    }

    /// Why `control`, at `address` or standing for the one there, cannot be
    /// drawn. In the items of a repeater its path is read from the item, and
    /// the design of the items is used only when the repeater is stacked.
    pub fn problem_of(
        &self,
        spec: &TypeSpec,
        address: &[usize],
        control: &Control,
    ) -> Option<String> {
        let Some(scope) = self.item_scope(address) else {
            return control_problem(spec, control);
        };
        let repeater = self.get(&scope)?;
        if !repeater.is_stacked() {
            return Some(
                "its repeater shows a table, so the design of its items is not used".to_owned(),
            );
        }
        if self.item_scope(&scope).is_some() {
            return Some(
                "a repeater inside an item shows its items as generated, so they are not designed"
                    .to_owned(),
            );
        }
        let array = repeater.bound_path().filter(|a| {
            spec.resolve(a)
                .is_some_and(|r| matches!(spec.get(r.ty).kind, Kind::Array { .. }))
        });
        let Some(array) = array else {
            return Some("its repeater is broken".to_owned());
        };
        control_problem_at(spec, control, &array.index(0))
    }

    /// The controls that are broken, with the reason.
    pub fn broken(&self, spec: &TypeSpec) -> Vec<(Address, String)> {
        self.walk()
            .into_iter()
            .filter_map(|(at, c)| self.problem_of(spec, &at, c).map(|p| (at, p)))
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
        for (at, control) in self.walk() {
            // The controls of a repeater's items show what the repeater shows.
            if self.item_scope(&at).is_some() {
                continue;
            }
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
    control_problem_at(spec, control, &Path::root())
}

/// As [`control_problem`], for a control whose path is relative to the node
/// at `base` (an item of a repeater).
pub fn control_problem_at(spec: &TypeSpec, control: &Control, base: &Path) -> Option<String> {
    // A field has a path or a value; with a value, and a repeater over an
    // expression, there is no path to check.
    if control.shows_value() && control.path.is_some() {
        return Some("a field has a path or a value, not both".to_owned());
    }
    if control.write_procedure.is_some() && !control.shows_value() {
        return Some("a write procedure needs a value".to_owned());
    }
    if control.shows_value() || control.lists_items() {
        return None;
    }
    let Some(text) = control.path.as_deref() else {
        return match control.kind {
            ControlKind::Field | ControlKind::Repeater => Some(format!(
                "a {} needs a path{}",
                control.kind.label().to_lowercase(),
                if control.kind == ControlKind::Field {
                    " or a value"
                } else {
                    ""
                }
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
        Ok(path) => base.join(&path),
        Err(e) => return Some(format!("the path {text:?} is not a path: {e}")),
    };
    let Some(resolved) = spec.resolve(&path) else {
        return Some(format!(
            "the schema has nothing at {}",
            match (text.is_empty(), base.is_root()) {
                (true, true) => "the root",
                (true, false) => "the item",
                _ => text,
            }
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
    fn percent_prefix_and_suffix_dress_a_number() {
        use crate::Value;
        let f = |percent, prefix: Option<&str>, suffix: Option<&str>, decimals| Format {
            percent,
            prefix: prefix.map(str::to_owned),
            suffix: suffix.map(str::to_owned),
            decimals,
            ..Format::default()
        };
        assert_eq!(
            f(true, None, None, None).apply(&Value::Float(0.07)),
            Some("7%".into())
        );
        assert_eq!(
            f(true, None, None, Some(1)).apply(&Value::Float(0.125)),
            Some("12.5%".into())
        );
        assert_eq!(
            f(true, None, None, None).apply(&Value::Int(2)),
            Some("200%".into())
        );
        let euros = Format {
            thousands: true,
            ..f(false, Some("€ "), None, Some(2))
        };
        assert_eq!(
            euros.apply(&Value::Float(1234.5)),
            Some("€ 1,234.50".into())
        );
        assert_eq!(
            f(false, None, Some(" kg"), None).apply(&Value::Int(3)),
            Some("3 kg".into())
        );
        // Only numbers are dressed.
        assert_eq!(
            f(true, Some("€"), None, None).apply(&Value::Str("x".into())),
            None
        );
        assert_eq!(
            f(true, None, None, None).apply(&Value::Float(f64::NAN)),
            None
        );
        // A design without them is stored as before.
        let mut field = Control::new(ControlKind::Field);
        field.suffix = Some(" kg".into());
        let design = FormDesign {
            controls: vec![field],
            ..FormDesign::default()
        };
        let text = design.to_json();
        assert!(
            text.contains("\"suffix\": \" kg\"") && !text.contains("percent"),
            "{text}"
        );
        assert_eq!(FormDesign::from_json(&text).unwrap(), design);
    }

    #[test]
    fn a_fixed_colour_is_hash_and_six_hex_digits() {
        assert_eq!(fixed_colour("#1a2B3c"), Some([0x1a, 0x2b, 0x3c]));
        assert_eq!(fixed_colour(" #ffffff "), Some([255, 255, 255]));
        for not in [
            "'#ffffff'",
            "#fff",
            "#gggggg",
            "red",
            "(1, 2, 3)",
            "#ffffff00",
        ] {
            assert_eq!(fixed_colour(not), None, "{not}");
        }
        let mut group = Control::new(ControlKind::Group);
        group.border_width = Some(9);
        let design = FormDesign {
            controls: vec![group],
            ..FormDesign::default()
        };
        assert!(FormDesign::from_json(&design.to_json()).is_err());
    }

    /// Designed items nest: a path is read from the item of each repeater
    /// around, and starts again from the top inside the items of an
    /// expression, where a field that shows one has no path to check.
    #[test]
    fn item_paths_chain_through_the_repeaters_around() {
        let sku = Control::bound(ControlKind::Field, &"sku".parse().unwrap());
        let mut lines = Control::bound(ControlKind::Repeater, &"lines".parse().unwrap());
        lines.mode = Some(RepeaterMode::Stacked);
        lines.children = vec![sku.clone()];
        let mut orders = Control::bound(ControlKind::Repeater, &"orders".parse().unwrap());
        orders.mode = Some(RepeaterMode::Stacked);
        orders.children = vec![lines];
        let mut listed = Control::new(ControlKind::Repeater);
        listed.items = Some("[1, 2]".into());
        let mut total = Control::new(ControlKind::Field);
        total.value = Some("this * 2".into());
        listed.children = vec![sku.clone(), total.clone()];
        let design = FormDesign {
            controls: vec![orders, listed],
            ..FormDesign::default()
        };
        assert_eq!(
            design.path_in_item(&[0, 0, 0], &sku, 3),
            Some("orders[0].lines[3].sku".parse().unwrap())
        );
        assert_eq!(
            design.path_in_item(&[1, 0], &sku, 1),
            Some("sku".parse().unwrap())
        );
        assert_eq!(design.path_in_item(&[1, 1], &total, 1), None);
        assert_eq!(control_problem(&spec(), &total), None);
        // A path or a value, not both; a write procedure only with a value.
        let mut both = total.clone();
        both.path = Some("name".into());
        assert_eq!(
            control_problem(&spec(), &both).as_deref(),
            Some("a field has a path or a value, not both")
        );
        let mut writer = Control::new(ControlKind::Field);
        writer.write_procedure = Some("set".into());
        assert_eq!(
            control_problem(&spec(), &writer).as_deref(),
            Some("a write procedure needs a value")
        );
        writer.value = Some("1".into());
        assert_eq!(control_problem(&spec(), &writer), None);
        assert_eq!(
            control_problem(&spec(), &Control::new(ControlKind::Field)).as_deref(),
            Some("a field needs a path or a value")
        );
    }

    #[test]
    fn a_stacked_repeater_takes_a_design_of_its_items() {
        let spec = spec();
        let mut design = generate(&spec);
        // The repeater of items is the third control.
        let repeater = vec![2];
        assert!(!design.takes_children(&repeater), "a table takes none");
        design.get_mut(&repeater).unwrap().mode = Some(RepeaterMode::Stacked);
        assert!(
            !design.takes_children(&repeater),
            "not before its items are designed"
        );
        let item_ty = match &spec
            .get(spec.resolve(&"items".parse().unwrap()).unwrap().ty)
            .kind
        {
            Kind::Array { items, .. } => *items,
            _ => unreachable!(),
        };
        let children = item_controls(&spec, item_ty);
        assert_eq!(children[0].path.as_deref(), Some("sku"));
        design.get_mut(&repeater).unwrap().children = children;
        assert!(design.takes_children(&repeater));
        assert!(
            design.broken(&spec).is_empty(),
            "{:?}",
            design.broken(&spec)
        );
        assert!(design.unplaced(&spec).is_empty());
        assert_eq!(design.item_scope(&[2, 1]), Some(vec![2]));
        assert_eq!(design.item_scope(&[2]), None);
        let qty = design.get(&[2, 1]).unwrap().clone();
        assert_eq!(
            design.path_in_item(&[2, 1], &qty, 4),
            Some("items[4].qty".parse().unwrap())
        );
        // A label in an item is at the item.
        let label = Control::new(ControlKind::Label);
        assert_eq!(
            design.path_in_item(&[2, 0], &label, 1),
            Some("items[1]".parse().unwrap())
        );
        // Relative paths, read from the item.
        assert_eq!(
            bindable_from(&spec, item_ty, ControlKind::Field),
            vec!["sku".parse::<Path>().unwrap(), "qty".parse().unwrap()]
        );
        design.get_mut(&[2, 0]).unwrap().path = Some("name".into());
        assert_eq!(
            design.problem(&spec, &[2, 0]).as_deref(),
            Some("the schema has nothing at name")
        );
        // A control moves into it, and out again.
        let name = design.move_to(&[0], &[2], 0).unwrap();
        assert_eq!(name, vec![1, 0]);
        // Out again, after the repeater, which is now second.
        assert_eq!(design.nest(&name, false), Some(vec![2]));
        // As a table, the design of its items is not used.
        design.get_mut(&[1]).unwrap().mode = Some(RepeaterMode::Table);
        assert!(
            design
                .problem(&spec, &[1, 1])
                .is_some_and(|p| p.contains("table"))
        );
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
        design.controls[0].masked = true;
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
    fn a_control_is_moved_to_any_place_but_inside_itself() {
        let path = |d: &FormDesign, at: &[usize]| d.get(at).and_then(|c| c.path.clone());
        let mut design = generate(&spec());
        // [0] name, [1] the customer group, [2] items, [3] the last field.
        let last = path(&design, &[3]);
        // Down the same list: the places count as before the move.
        assert_eq!(design.move_to(&[0], &[], 3), Some(vec![2]));
        assert_eq!(path(&design, &[2]).as_deref(), Some("name"));
        assert_eq!(design.move_to(&[2], &[], 0), Some(vec![0]));
        assert_eq!(path(&design, &[0]).as_deref(), Some("name"));
        // Into a group that comes after it: the group moves up one place.
        let into = design.move_to(&[0], &[1], 0).unwrap();
        assert_eq!(into, vec![0, 0]);
        assert_eq!(path(&design, &[0, 0]).as_deref(), Some("name"));
        // Out of the group, to the end of the form.
        assert_eq!(design.move_to(&[0, 0], &[], 99), Some(vec![3]));
        assert_eq!(path(&design, &[3]).as_deref(), Some("name"));
        assert_eq!(path(&design, &[2]), last);
        // Not into itself, nor into what is not a group.
        assert_eq!(design.move_to(&[0], &[0], 0), None);
        assert_eq!(design.move_to(&[0], &[0, 1], 0), None);
        assert_eq!(design.move_to(&[1], &[2], 0), None);
        assert_eq!(design.move_to(&[9], &[], 0), None);
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

    #[test]
    fn flow_rows_put_controls_in_rows_by_span_and_new_row() {
        let with = |span: Option<u32>, new_row: bool| Control {
            span,
            new_row,
            ..Control::new(ControlKind::Label)
        };
        let children = [
            with(None, false),
            with(Some(2), false),
            with(None, true),
            with(Some(9), false),
            with(None, false),
            with(None, false),
        ];
        let rows: Vec<Vec<(usize, usize, usize)>> = flow_rows(&children, 3)
            .into_iter()
            .map(|r| r.cells)
            .collect();
        assert_eq!(
            rows,
            vec![
                vec![(0, 0, 1), (1, 1, 2)],
                vec![(2, 0, 1)],
                // A span wider than the group takes the whole row.
                vec![(3, 0, 3)],
                vec![(4, 0, 1), (5, 1, 1)],
            ]
        );
        assert!(flow_rows(&[], 3).is_empty());
    }

    #[test]
    fn sizes_are_stored_and_checked_against_their_ranges() {
        let mut design = FormDesign::default();
        let mut field = Control::bound(ControlKind::Field, &"name".parse().unwrap());
        field.width = Some(240);
        field.height = Some(80);
        field.align = Some(Alignment::Right);
        field.label_position = Some(LabelPosition::Top);
        field.label_width = Some(0);
        field.span = Some(2);
        field.new_row = true;
        let mut group = Control::new(ControlKind::Group);
        group.fill = true;
        group.spacing = Some(12);
        group.padding = Some(0);
        group.children.push(field);
        design.controls.push(group);
        let text = design.to_json();
        assert!(text.contains("\"label_position\": \"top\""), "{text}");
        assert_eq!(FormDesign::from_json(&text).unwrap(), design);
        let too_wide = text.replace("\"width\": 240", "\"width\": 5000");
        assert!(FormDesign::from_json(&too_wide).is_err());
        let no_space = text.replace("\"spacing\": 12", "\"spacing\": 60");
        assert!(FormDesign::from_json(&no_space).is_err());
    }

    #[test]
    fn choices_and_their_editors_are_stored() {
        let mut design = FormDesign::default();
        let mut field = Control::bound(ControlKind::Field, &"city".parse().unwrap());
        field.choices = Some("[('UT', 'Utrecht')]".into());
        field.editor = Some(Editor::Typeahead);
        field.allow_other = true;
        let mut table = Control::bound(ControlKind::Repeater, &"items".parse().unwrap());
        let mut column = ColumnDesign::named("sku");
        column.choices = Some("['A', 'B']".into());
        column.editor = Some(Editor::Dropdown);
        table.table_columns.push(column);
        design.controls.extend([field, table]);
        let text = design.to_json();
        assert!(text.contains("\"editor\": \"typeahead\""), "{text}");
        assert!(text.contains("\"allow_other\": true"), "{text}");
        assert_eq!(FormDesign::from_json(&text).unwrap(), design);
        // Absent when not set: an older design reads the same.
        let plain = Control::bound(ControlKind::Field, &"city".parse().unwrap());
        let plain = FormDesign {
            controls: vec![plain],
            ..FormDesign::default()
        };
        assert!(!plain.to_json().contains("choices"));
        assert!(!plain.to_json().contains("allow_other"));
    }

    fn free_group() -> Control {
        let mut group = Control::new(ControlKind::Group);
        group.layout = Some(GroupLayout::Free);
        group.base_width = Some(400);
        let mut a = Control::new(ControlKind::Label);
        a.x = Some(10);
        a.y = Some(20);
        a.width = Some(100);
        let mut b = a.clone();
        b.anchor_x = Some(Anchor::End);
        let mut c = a.clone();
        c.anchor_x = Some(Anchor::Both);
        group.children = vec![a, b, c];
        group
    }

    #[test]
    fn anchors_keep_a_free_control_at_its_edges() {
        let group = free_group();
        let at = |i: usize, w: f32| free_place(&group, &group.children[i], (w, 300.0));
        // At the size the places were made for, each is where it says.
        for i in 0..3 {
            assert_eq!(at(i, 400.0).left, 10.0);
            assert_eq!(at(i, 400.0).width, Some(100.0));
        }
        // Wider by 50: the left one stays, the right one moves, the one
        // anchored to both stretches.
        assert_eq!((at(0, 450.0).left, at(0, 450.0).width), (10.0, Some(100.0)));
        assert_eq!((at(1, 450.0).left, at(1, 450.0).width), (60.0, Some(100.0)));
        assert_eq!((at(2, 450.0).left, at(2, 450.0).width), (10.0, Some(150.0)));
        // Down, nothing moves while the group has no height of its own.
        let mut bottom = group.children[0].clone();
        bottom.anchor_y = Some(Anchor::End);
        assert_eq!(free_place(&group, &bottom, (400.0, 900.0)).top, 20.0);
        let mut tall = group.clone();
        tall.height = Some(300);
        tall.base_height = Some(280);
        assert_eq!(free_place(&tall, &bottom, (400.0, 300.0)).top, 40.0);
    }

    #[test]
    fn rebasing_keeps_every_control_where_it_is_seen() {
        let mut group = free_group();
        let before: Vec<FreePlace> = group
            .children
            .iter()
            .map(|c| free_place(&group, c, (450.0, 300.0)))
            .collect();
        rebase_free(&mut group, (450.0, 300.0));
        assert_eq!(group.base_width, Some(450));
        let after: Vec<FreePlace> = group
            .children
            .iter()
            .map(|c| free_place(&group, c, (450.0, 300.0)))
            .collect();
        assert_eq!(before, after);
        assert_eq!(group.children[1].x, Some(60));
        assert_eq!(group.children[2].width, Some(150));
    }

    #[test]
    fn a_free_layout_is_stored_and_checked() {
        let mut design = FormDesign::default();
        design.controls.push(free_group());
        let text = design.to_json();
        assert!(text.contains("\"layout\": \"free\""), "{text}");
        assert!(text.contains("\"anchor_x\": \"end\""), "{text}");
        assert_eq!(FormDesign::from_json(&text).unwrap(), design);
        let far = text.replacen("\"x\": 10", "\"x\": 5000", 1);
        assert!(FormDesign::from_json(&far).is_err());
    }

    fn grid_of(children: Vec<Control>, columns: usize) -> Control {
        let mut group = Control::new(ControlKind::Group);
        group.layout = Some(GroupLayout::Grid);
        group.grid_columns = vec![TrackSize::Auto; columns];
        group.children = children;
        group
    }

    fn cell(row: u32, column: u32) -> Control {
        let mut c = Control::new(ControlKind::Label);
        c.grid_row = Some(row);
        c.grid_column = Some(column);
        c
    }

    #[test]
    fn controls_of_a_grid_take_their_cells_or_the_next_free_one() {
        let label = || Control::new(ControlKind::Label);
        let mut wide = label();
        wide.span = Some(2);
        let mut tall = cell(1, 3);
        tall.row_span = Some(2);
        // Three columns: the third column of rows 1 and 2 is taken by `tall`.
        let group = grid_of(vec![label(), wide, label(), tall, label()], 3);
        let (cells, rows, columns) = grid_cells(&group);
        let at: Vec<(usize, usize, usize, usize)> = cells
            .iter()
            .map(|c| (c.row, c.column, c.rows, c.columns))
            .collect();
        assert_eq!(
            at,
            vec![
                (0, 0, 1, 1),
                // Does not fit beside the first: the next row.
                (1, 0, 1, 2),
                (2, 0, 1, 1),
                (0, 2, 2, 1),
                (2, 1, 1, 1),
            ]
        );
        assert_eq!((rows, columns), (3, 3));
        // A control placed beyond the columns adds one; cells can stay empty.
        let group = grid_of(vec![cell(4, 5)], 2);
        let (_, rows, columns) = grid_cells(&group);
        assert_eq!((rows, columns), (4, 5));
    }

    #[test]
    fn tracks_are_points_their_content_or_a_share_of_what_is_left() {
        let sizes = [
            TrackSize::Points(100),
            TrackSize::Auto,
            TrackSize::Share(1),
            TrackSize::Share(3),
        ];
        let content = [50.0, 80.0, 30.0, 30.0, 40.0];
        // 600 less 100, 80, the fifth (automatic) 40 and four gaps of 10:
        // 340, in 1 and 3 shares.
        let out = track_sizes(&sizes, &content, Some(600.0), 10.0);
        assert_eq!(out, vec![100.0, 80.0, 85.0, 255.0, 40.0]);
        // With nothing available, shares are automatic.
        let out = track_sizes(&sizes, &content, None, 10.0);
        assert_eq!(out, vec![100.0, 80.0, 30.0, 30.0, 40.0]);
    }

    #[test]
    fn a_grid_is_stored_and_its_sizes_checked() {
        let mut group = grid_of(vec![cell(2, 1)], 0);
        group.grid_columns = vec![TrackSize::Points(120), TrackSize::Share(2)];
        group.grid_rows = vec![TrackSize::Auto];
        let mut design = FormDesign::default();
        design.controls.push(group);
        let text = design.to_json();
        assert!(text.contains("\"120\""), "{text}");
        assert!(text.contains("\"2fr\""), "{text}");
        assert!(text.contains("\"auto\""), "{text}");
        assert_eq!(FormDesign::from_json(&text).unwrap(), design);
        assert!(FormDesign::from_json(&text.replace("\"2fr\"", "\"wide\"")).is_err());
        assert!(FormDesign::from_json(&text.replace("\"120\"", "\"3\"")).is_err());
    }
}
