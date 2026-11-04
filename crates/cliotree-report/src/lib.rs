//! Writing a report as PDF (DESIGN.md, Reports, "PDF export"): what
//! `cliotree-layout` laid out, drawn on the pages of a PDF with krilla, so
//! that the PDF has everything where the screen has it.
//!
//! - Text is placed glyph by glyph: each line is shaped again with the
//!   bundled fonts exactly as the layout measured it, and its glyphs are
//!   given to krilla with their advances, so the PDF breaks no line and
//!   moves no word. The fonts are embedded as subsets; the bundled fonts
//!   are variable, so bold is their instance at weight 700. Italic is
//!   slanted as the window slants it.
//! - Fills, borders and lines are paths. A pending marker is not drawn: a
//!   PDF is written when the report is calculated.
//! - A chart's SVG goes in as vector graphics, with krilla-svg, parsed
//!   with the options and style sheet the window renders it with.
//! - The colours of the theme are those of the light theme, on white
//!   paper ([`paper_colour`]).
//!
//! Nothing here depends on egui: the application and `cliotree-cli` both
//! write their PDFs with it. [`workbook`] reads a report's values from a
//! workbook; the rest takes only what was laid out.

pub mod workbook;

use std::collections::HashMap;

use cliotree_layout::fonts::{BOLD_WEIGHT, FILES, REGULAR_WEIGHT};
use cliotree_layout::report::{Ink, Laid, Paged, Paint, Rect};
use cliotree_layout::text::{Fonts, TextStyle};
use krilla::color::rgb;
use krilla::geom::{PathBuilder, Point, Size, Transform};
use krilla::metadata::Metadata;
use krilla::num::NormalizedF32;
use krilla::page::PageSettings;
use krilla::paint::{Fill, FillRule, Stroke};
use krilla::surface::Surface;
use krilla::text::{Font, GlyphId, KrillaGlyph, Tag};
use krilla::{Document, SerializeSettings};
use krilla_svg::{SurfaceExt as _, SvgSettings};

/// How far italic text leans: the top of a glyph moves right by this much
/// of its height, as egui draws italics.
const SLANT: f32 = 0.25;

/// The colour of an ink on paper: the light theme's colours (as
/// `cliotree-app`'s `style.rs` has them), and a colour of the design as it
/// is.
pub fn paper_colour(ink: Ink) -> [u8; 4] {
    match ink {
        Ink::Text => [0x1D, 0x25, 0x2D, 255],
        Ink::Weak => [0x5E, 0x6B, 0x78, 255],
        Ink::Error => [0xC9, 0x36, 0x36, 255],
        Ink::Rule => [0xD5, 0xDC, 0xE3, 255],
        Ink::Rgba(c) => c,
    }
}

/// One page of a PDF: its size in points, and what is on it, in points from
/// its top left.
pub struct Page<'a> {
    pub width: f32,
    pub height: f32,
    pub laid: &'a Laid,
}

/// A single page report as a PDF of one page, as wide and as tall as the
/// report (DESIGN.md, "PDF export": the paper's width less its margins).
pub fn single_page(laid: &Laid, title: &str) -> Result<Vec<u8>, String> {
    write(
        title,
        &[Page {
            width: laid.width,
            height: laid.height,
            laid,
        }],
    )
}

/// A multipage report as a PDF, page by page on its paper. A report with no
/// pages is one empty page, as a PDF must have one.
pub fn pages(paged: &Paged, title: &str) -> Result<Vec<u8>, String> {
    let empty = Laid::default();
    let mut pages: Vec<Page> = paged
        .pages
        .iter()
        .map(|laid| Page {
            width: paged.width,
            height: paged.height,
            laid,
        })
        .collect();
    if pages.is_empty() {
        pages.push(Page {
            width: paged.width,
            height: paged.height,
            laid: &empty,
        });
    }
    write(title, &pages)
}

/// The pages as a PDF, with `title` as its title.
pub fn write(title: &str, pages: &[Page]) -> Result<Vec<u8>, String> {
    let mut document = Document::new_with(SerializeSettings::default());
    let mut fonts = FontCache::default();
    for page in pages {
        let settings = PageSettings::from_wh(page.width.max(1.0), page.height.max(1.0))
            .ok_or_else(|| format!("a page cannot be {} by {}", page.width, page.height))?;
        let mut pdf_page = document.start_page_with(settings);
        let mut surface = pdf_page.surface();
        draw(&mut surface, page.laid, &mut fonts);
        surface.finish();
        pdf_page.finish();
    }
    document.set_metadata(
        Metadata::new()
            .title(title.to_owned())
            .creator(format!("ClioTree {}", env!("CARGO_PKG_VERSION"))),
    );
    document
        .finish()
        .map_err(|e| format!("the PDF could not be written: {e:?}"))
}

/// The fonts of the PDF, each read once at the weight it is used in.
#[derive(Default)]
struct FontCache(HashMap<(usize, bool), Option<Font>>);

impl FontCache {
    fn get(&mut self, font: usize, bold: bool) -> Option<Font> {
        self.0
            .entry((font, bold))
            .or_insert_with(|| {
                let bytes: &'static [u8] = FILES.get(font)?.bytes;
                let weight = if bold { BOLD_WEIGHT } else { REGULAR_WEIGHT };
                Font::new_variable(bytes.into(), 0, &[(Tag::new(b"wght"), weight)])
                    .or_else(|| Font::new(bytes.into(), 0))
            })
            .clone()
    }
}

fn fill(ink: Ink) -> Fill {
    let [r, g, b, a] = paper_colour(ink);
    Fill {
        paint: rgb::Color::new(r, g, b).into(),
        opacity: NormalizedF32::new(f32::from(a) / 255.0).unwrap_or(NormalizedF32::ONE),
        rule: FillRule::default(),
    }
}

fn stroke(ink: Ink, width: f32) -> Stroke {
    let [r, g, b, a] = paper_colour(ink);
    Stroke {
        paint: rgb::Color::new(r, g, b).into(),
        width,
        opacity: NormalizedF32::new(f32::from(a) / 255.0).unwrap_or(NormalizedF32::ONE),
        ..Stroke::default()
    }
}

fn rect_path(r: &Rect) -> Option<krilla::geom::Path> {
    let mut builder = PathBuilder::new();
    builder.push_rect(krilla::geom::Rect::from_xywh(r.x, r.y, r.w, r.h)?);
    builder.finish()
}

/// Draws what was laid out.
fn draw(surface: &mut Surface, laid: &Laid, fonts: &mut FontCache) {
    for paint in &laid.paints {
        match paint {
            Paint::Text {
                x,
                y,
                text,
                style,
                ink,
            } => text_line(surface, fonts, (*x, *y), text, style, *ink),
            Paint::Fill { rect, ink } => {
                if let Some(path) = rect_path(rect) {
                    surface.set_stroke(None);
                    surface.set_fill(Some(fill(*ink)));
                    surface.draw_path(&path);
                }
            }
            Paint::Border { rect, width, ink } => {
                // Inside the rectangle, as the window draws it.
                let half = width / 2.0;
                let inner = Rect::new(
                    rect.x + half,
                    rect.y + half,
                    (rect.w - width).max(0.0),
                    (rect.h - width).max(0.0),
                );
                if let Some(path) = rect_path(&inner) {
                    surface.set_fill(None);
                    surface.set_stroke(Some(stroke(*ink, *width)));
                    surface.draw_path(&path);
                }
            }
            Paint::Line {
                from,
                to,
                width,
                ink,
            } => {
                let mut builder = PathBuilder::new();
                builder.move_to(from.0, from.1);
                builder.line_to(to.0, to.1);
                if let Some(path) = builder.finish() {
                    surface.set_fill(None);
                    surface.set_stroke(Some(stroke(*ink, *width)));
                    surface.draw_path(&path);
                }
            }
            Paint::Pending { .. } => {}
            Paint::Svg { rect, svg, style } => chart(surface, rect, svg, style.as_deref()),
        }
    }
    surface.set_fill(None);
    surface.set_stroke(None);
}

/// One line of text with its row's top left at `at`, its glyphs where the
/// layout measured them.
fn text_line(
    surface: &mut Surface,
    fonts: &mut FontCache,
    at: (f32, f32),
    text: &str,
    style: &TextStyle,
    ink: Ink,
) {
    let measure = Fonts::shared();
    let shaped = measure.shape(text, style);
    if shaped.glyphs.is_empty() || style.size <= 0.0 {
        return;
    }
    let baseline = at.1 + measure.metrics(style).ascent;
    let size = style.size;
    // Where the cluster of each glyph ends: where the next cluster starts.
    let mut starts: Vec<usize> = shaped.glyphs.iter().map(|g| g.cluster).collect();
    starts.sort_unstable();
    starts.dedup();
    let end_of = |cluster: usize| {
        starts
            .iter()
            .copied()
            .find(|&s| s > cluster)
            .unwrap_or(text.len())
    };
    surface.set_stroke(None);
    surface.set_fill(Some(fill(ink)));
    if style.italic {
        surface.push_transform(&Transform::from_row(
            1.0,
            0.0,
            -SLANT,
            1.0,
            SLANT * baseline,
            0.0,
        ));
    }
    let glyphs = &shaped.glyphs;
    let mut i = 0;
    while i < glyphs.len() {
        let font = glyphs[i].font;
        let j = glyphs[i..]
            .iter()
            .position(|g| g.font != font)
            .map_or(glyphs.len(), |n| i + n);
        let run = &glyphs[i..j];
        if let Some(pdf_font) = fonts.get(font, style.bold) {
            let run_glyphs: Vec<KrillaGlyph> = run
                .iter()
                .map(|g| {
                    KrillaGlyph::new(
                        GlyphId::new(g.id),
                        g.advance / size,
                        g.x_offset / size,
                        // The layout's offsets go down; the font's go up.
                        -g.y_offset / size,
                        0.0,
                        g.cluster..end_of(g.cluster),
                        None,
                    )
                })
                .collect();
            surface.draw_glyphs(
                Point::from_xy(at.0 + run[0].x, baseline),
                &run_glyphs,
                pdf_font,
                text,
                size,
                false,
            );
        }
        i = j;
    }
    if style.italic {
        surface.pop();
    }
}

/// A chart's SVG, fitted into its box keeping its proportions and centred,
/// as the window draws it. SVG that does not parse draws nothing: the
/// layout has already put its error there.
fn chart(surface: &mut Surface, rect: &Rect, svg: &str, style: Option<&str>) {
    let options = cliotree_layout::chart::svg_options_styled(style);
    let Ok(tree) = usvg::Tree::from_str(svg, &options) else {
        return;
    };
    let size = tree.size();
    let scale = (rect.w / size.width()).min(rect.h / size.height());
    if !(scale.is_finite() && scale > 0.0) {
        return;
    }
    let (w, h) = (size.width() * scale, size.height() * scale);
    let Some(target) = Size::from_wh(w, h) else {
        return;
    };
    surface.push_transform(&Transform::from_translate(
        rect.x + (rect.w - w) / 2.0,
        rect.y + (rect.h - h) / 2.0,
    ));
    surface.draw_svg(&tree, target, SvgSettings::default());
    surface.pop();
}

#[cfg(test)]
mod tests;
