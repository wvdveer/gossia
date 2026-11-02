//! Measuring text and breaking it into lines, in points, as egui would draw
//! it on screen (DESIGN.md, Reports, "The report designer").
//!
//! egui 0.37 reads fonts with skrifa and shapes text with harfrust, and so
//! does this module, at the same versions and with the same fonts, so a width
//! measured here is the width egui gives the same text. The height of a row
//! is worked out as egui does it too: from the first font of the typeface,
//! its ascent, descent and line gap, each rounded to 1/32 point.
//!
//! What is measured is what the report draws, on screen and in the PDF: a
//! line is broken here, and drawn as one line by both.

use std::ops::Range;
use std::sync::OnceLock;

use cliotree_core::form::FontFamily as Typeface;
use harfrust::{
    BufferFlags, FontRef, ShapeOptions, ShaperData, ShaperInstance, Tag, UnicodeBuffer, Variation,
};
use skrifa::MetadataProvider as _;
use skrifa::charmap::Charmap;
use skrifa::instance::{LocationRef, Size};

use crate::fonts::{self, BOLD_WEIGHT, FILES, REGULAR_WEIGHT};

/// How text is set: a typeface, a size in points, and bold and italic.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextStyle {
    pub typeface: Typeface,
    pub size: f32,
    pub bold: bool,
    /// Italic is drawn slanted (the bundled fonts have no italic files), so
    /// it does not change a width.
    pub italic: bool,
}

impl TextStyle {
    pub fn new(typeface: Typeface, size: f32) -> TextStyle {
        TextStyle {
            typeface,
            size,
            bold: false,
            italic: false,
        }
    }

    pub fn bold(self) -> TextStyle {
        TextStyle { bold: true, ..self }
    }

    pub fn italic(self) -> TextStyle {
        TextStyle {
            italic: true,
            ..self
        }
    }
}

/// The vertical measures of a style, in points: the distance from the top of
/// a row to the baseline, the descent below it (negative), the gap, and the
/// height of a row.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    pub ascent: f32,
    pub descent: f32,
    pub line_gap: f32,
    pub row_height: f32,
}

/// One glyph of shaped text. `x` is where its pen position is, from the start
/// of the text it was shaped in; the offsets move the glyph from there (y
/// downwards). `cluster` is the byte in that text where its characters begin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Glyph {
    /// The font, as a place in [`fonts::FILES`].
    pub font: usize,
    pub id: u32,
    pub cluster: usize,
    pub x: f32,
    pub advance: f32,
    pub x_offset: f32,
    pub y_offset: f32,
}

/// Text shaped on one line: its glyphs in the order of the text, and its width.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Shaped {
    pub glyphs: Vec<Glyph>,
    pub width: f32,
}

/// One line of laid-out text: the bytes of the text it shows (trailing spaces
/// left out), its width, and its glyphs, with `x` from the start of the line.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub range: Range<usize>,
    pub width: f32,
    pub glyphs: Vec<Glyph>,
}

/// Text broken into lines. Line `i` is drawn with its top at
/// `i * metrics.row_height` and its baseline `metrics.ascent` below that.
#[derive(Debug, Clone, PartialEq)]
pub struct TextLayout {
    pub lines: Vec<Line>,
    pub metrics: Metrics,
    /// The width of the widest line.
    pub width: f32,
}

impl TextLayout {
    pub fn height(&self) -> f32 {
        self.lines.len() as f32 * self.metrics.row_height
    }
}

/// A font, read once.
struct Face {
    font: FontRef<'static>,
    charmap: Charmap<'static>,
    shaper: ShaperData,
    regular: ShaperInstance,
    bold: ShaperInstance,
    units_per_em: f32,
    ascent: f32,
    descent: f32,
    leading: f32,
}

impl Face {
    fn read(bytes: &'static [u8]) -> Result<Face, String> {
        let font = FontRef::new(bytes).map_err(|e| e.to_string())?;
        let metrics = font.metrics(Size::unscaled(), LocationRef::default());
        let weight = |value| {
            ShaperInstance::from_variations(
                &font,
                [Variation {
                    tag: Tag::new(b"wght"),
                    value,
                }],
            )
        };
        Ok(Face {
            charmap: font.charmap(),
            shaper: ShaperData::new(&font),
            regular: weight(REGULAR_WEIGHT),
            bold: weight(BOLD_WEIGHT),
            units_per_em: f32::from(metrics.units_per_em),
            ascent: metrics.ascent,
            descent: metrics.descent,
            leading: metrics.leading,
            font,
        })
    }

    fn has(&self, c: char) -> bool {
        self.charmap
            .map(c)
            .is_some_and(|id| id != skrifa::GlyphId::NOTDEF)
    }

    fn scale(&self, size: f32) -> f32 {
        size / self.units_per_em
    }
}

/// egui rounds sizes in its interface to 1/32 point.
fn round_ui(x: f32) -> f32 {
    (x * 32.0).round() / 32.0
}

/// The bundled fonts, ready to measure with.
pub struct Fonts {
    faces: Vec<Face>,
}

impl Fonts {
    /// Reads the bundled fonts. They are part of the program, so failing to
    /// read one is a fault in the build, and panics.
    pub fn new() -> Fonts {
        let faces = FILES
            .iter()
            .map(|file| {
                Face::read(file.bytes)
                    .unwrap_or_else(|e| panic!("the bundled font {} is unreadable: {e}", file.name))
            })
            .collect();
        Fonts { faces }
    }

    /// One set of fonts for the whole program.
    pub fn shared() -> &'static Fonts {
        static FONTS: OnceLock<Fonts> = OnceLock::new();
        FONTS.get_or_init(Fonts::new)
    }

    pub fn metrics(&self, style: &TextStyle) -> Metrics {
        let face = &self.faces[fonts::chain(style.typeface)[0]];
        let scale = face.scale(style.size);
        let ascent = round_ui(face.ascent * scale);
        let descent = round_ui(face.descent * scale);
        let line_gap = round_ui(face.leading * scale);
        Metrics {
            ascent,
            descent,
            line_gap,
            row_height: ascent - descent + line_gap,
        }
    }

    /// The font a character is drawn from: the first of the typeface's chain
    /// that has it, or the first of all.
    fn font_for(&self, typeface: Typeface, c: char) -> usize {
        let chain = fonts::chain(typeface);
        let c = if c == '\t' { ' ' } else { c };
        chain
            .iter()
            .copied()
            .find(|&i| self.faces[i].has(c))
            .unwrap_or(chain[0])
    }

    /// Shapes text as one line. A line break in it is shaped as whatever the
    /// font has for it; [`Fonts::layout`] splits lines first.
    pub fn shape(&self, text: &str, style: &TextStyle) -> Shaped {
        let mut glyphs = Vec::new();
        let mut x = 0.0;
        let mut runs: Vec<(usize, Range<usize>)> = Vec::new();
        for (at, c) in text.char_indices() {
            let font = self.font_for(style.typeface, c);
            match runs.last_mut() {
                Some((f, range)) if *f == font => range.end = at + c.len_utf8(),
                _ => runs.push((font, at..at + c.len_utf8())),
            }
        }
        let count = runs.len();
        for (n, (font, range)) in runs.into_iter().enumerate() {
            let face = &self.faces[font];
            let scale = face.scale(style.size);
            let instance = if style.bold {
                &face.bold
            } else {
                &face.regular
            };
            let shaper = face
                .shaper
                .shaper(&face.font)
                .instance(Some(instance))
                .build();
            let mut buffer = UnicodeBuffer::new();
            let mut flags = BufferFlags::empty();
            if n == 0 {
                flags |= BufferFlags::BEGINNING_OF_TEXT;
            }
            if n + 1 == count {
                flags |= BufferFlags::END_OF_TEXT;
            }
            buffer.set_flags(flags);
            let run = &text[range.clone()];
            buffer.push_str(run);
            buffer.guess_segment_properties();
            let shaped = shaper.shape(buffer, ShapeOptions::new());
            let space = if run.contains('\t') {
                self.shape(" ", style).width
            } else {
                0.0
            };
            for (info, pos) in shaped.glyph_infos().iter().zip(shaped.glyph_positions()) {
                let cluster = range.start + info.cluster as usize;
                let mut advance = pos.x_advance as f32 * scale;
                if text[cluster..].starts_with('\t') {
                    // A tab is four spaces wide, as egui draws it.
                    advance = 4.0 * space;
                }
                glyphs.push(Glyph {
                    font,
                    id: info.glyph_id,
                    cluster,
                    x,
                    advance,
                    x_offset: pos.x_offset as f32 * scale,
                    y_offset: -(pos.y_offset as f32 * scale),
                });
                x += advance;
            }
        }
        Shaped { glyphs, width: x }
    }

    /// The width of text set on one line.
    pub fn width(&self, text: &str, style: &TextStyle) -> f32 {
        self.shape(text, style).width
    }

    /// Text broken into lines no wider than `wrap` (none: only at line
    /// breaks). Lines break after spaces and after a hyphen between letters;
    /// a word wider than the whole line is broken between its characters.
    /// Empty text is one empty line, as egui has it.
    pub fn layout(&self, text: &str, style: &TextStyle, wrap: Option<f32>) -> TextLayout {
        let metrics = self.metrics(style);
        let mut lines = Vec::new();
        let mut start = 0;
        for paragraph in text.split('\n') {
            let end = start + paragraph.len();
            let body = paragraph.strip_suffix('\r').unwrap_or(paragraph);
            self.paragraph(body, start, style, wrap, &mut lines);
            start = end + 1;
        }
        let width = lines.iter().map(|l| l.width).fold(0.0, f32::max);
        TextLayout {
            lines,
            metrics,
            width,
        }
    }

    /// Lays out one paragraph (no line breaks in it) that starts at byte
    /// `offset` of the whole text.
    fn paragraph(
        &self,
        text: &str,
        offset: usize,
        style: &TextStyle,
        wrap: Option<f32>,
        lines: &mut Vec<Line>,
    ) {
        let shaped = self.shape(text, style);
        // Where each cluster starts, and the pen there, with the end last.
        let mut stops: Vec<(usize, f32)> = Vec::new();
        for g in &shaped.glyphs {
            if stops.last().is_none_or(|&(at, _)| at != g.cluster) {
                stops.push((g.cluster, g.x));
            }
        }
        stops.push((text.len(), shaped.width));
        let x_at = |at: usize| -> f32 {
            let i = stops.partition_point(|&(s, _)| s < at);
            stops.get(i).map_or(shaped.width, |&(_, x)| x)
        };
        let visible_end =
            |from: usize, to: usize| -> usize { from + text[from..to].trim_end().len() };
        let mut emit = |from: usize, to: usize| {
            let to = visible_end(from, to);
            let base = x_at(from);
            let glyphs = shaped
                .glyphs
                .iter()
                .filter(|g| g.cluster >= from && g.cluster < to)
                .map(|g| Glyph {
                    cluster: g.cluster + offset,
                    x: g.x - base,
                    ..*g
                })
                .collect();
            lines.push(Line {
                range: from + offset..to + offset,
                width: x_at(to) - base,
                glyphs,
            });
        };
        let Some(max) = wrap else {
            emit(0, text.len());
            return;
        };
        let mut line = 0;
        let mut segment = 0;
        for end in break_points(text) {
            // Put the segment that ends here on the line, or start a new one.
            if segment > line && x_at(visible_end(line, end)) - x_at(line) > max {
                emit(line, segment);
                line = segment;
            }
            // A segment wider than a whole line is broken between clusters.
            while x_at(visible_end(line, end)) - x_at(line) > max {
                let limit = x_at(line) + max;
                let cut = stops
                    .iter()
                    .map(|&(at, _)| at)
                    .rfind(|&at| at > line && at < end && x_at(at) <= limit)
                    .or_else(|| stops.iter().map(|&(at, _)| at).find(|&at| at > line))
                    .unwrap_or(end);
                if cut >= visible_end(line, end) {
                    break;
                }
                emit(line, cut);
                line = cut;
            }
            segment = end;
        }
        emit(line, text.len());
    }
}

impl Default for Fonts {
    fn default() -> Fonts {
        Fonts::new()
    }
}

/// The places a line may end, in order, the end of the text last: after a
/// run of spaces (where the next word starts), and after a hyphen that is
/// between two letters.
fn break_points(text: &str) -> Vec<usize> {
    let mut points = Vec::new();
    let mut chars = text.char_indices().peekable();
    let mut previous: Option<char> = None;
    while let Some((at, c)) = chars.next() {
        let next = chars.peek().map(|&(_, n)| n);
        let after = at + c.len_utf8();
        let end_of_spaces = c.is_whitespace() && next.is_some_and(|n| !n.is_whitespace());
        let hyphen = c == '-'
            && previous.is_some_and(char::is_alphabetic)
            && next.is_some_and(char::is_alphabetic);
        if (end_of_spaces || hyphen) && after < text.len() {
            points.push(after);
        }
        previous = Some(c);
    }
    points.push(text.len());
    points
}

#[cfg(test)]
#[path = "text_tests.rs"]
mod tests;
