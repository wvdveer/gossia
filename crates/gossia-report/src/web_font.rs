//! The fonts of an HTML export, reduced to the characters the report uses
//! (DESIGN.md, Reports, "Export as HTML"; 0.7.0).
//!
//! krilla's subsetter (`subsetter`, which the PDF uses) keeps the glyphs
//! wanted, their outlines and widths, and instances a variable font at a
//! weight, but writes fonts for PDF: it leaves out the tables that map
//! characters to glyphs (`cmap`) and `OS/2`, which a browser needs. So a
//! `cmap` for the characters kept is written here, with `write-fonts`, and
//! `OS/2` is copied from the bundled font. The layout tables (`GSUB`,
//! `GPOS`) are left out too, so the browser draws no kerning or ligatures:
//! each line is still given the width the layout measured (`textLength`),
//! so no line ends anywhere else.

use std::collections::{BTreeMap, BTreeSet};

use gossia_layout::fonts::FILES;
use gossia_layout::report::{Laid, Paint};
use gossia_layout::text::Fonts;
use write_fonts::FontBuilder;
use write_fonts::read::{FontRef, TableProvider};
use write_fonts::tables::cmap::Cmap;
use write_fonts::types::{GlyphId, Tag};

/// The weights of the bundled fonts the window draws with, as the PDF has
/// them: regular and bold.
pub const REGULAR: u16 = 400;
pub const BOLD: u16 = 700;

/// A font as embedded: the place in [`FILES`] of the font it comes from,
/// the weight it is for, and its bytes. With no weight it is the whole
/// font, for every weight, since it could not be reduced.
#[derive(Debug, Clone, PartialEq)]
pub struct Embedded {
    pub font: usize,
    pub weight: Option<u16>,
    pub bytes: Vec<u8>,
}

/// The characters each bundled font draws in the report, by the font's
/// place in [`FILES`] and the weight: those its glyphs were shaped for.
pub fn used_chars(pages: &[&Laid]) -> BTreeMap<(usize, u16), BTreeSet<char>> {
    let measure = Fonts::shared();
    let mut used: BTreeMap<(usize, u16), BTreeSet<char>> = BTreeMap::new();
    for laid in pages {
        for paint in &laid.paints {
            let Paint::Text { text, style, .. } = paint else {
                continue;
            };
            let weight = if style.bold { BOLD } else { REGULAR };
            let shaped = measure.shape(text, style);
            // A glyph draws the characters from its cluster to the next.
            let mut starts: Vec<usize> = shaped.glyphs.iter().map(|g| g.cluster).collect();
            starts.sort_unstable();
            starts.dedup();
            for glyph in &shaped.glyphs {
                let end = starts
                    .iter()
                    .find(|&&s| s > glyph.cluster)
                    .copied()
                    .unwrap_or(text.len());
                let Some(chars) = text.get(glyph.cluster..end) else {
                    continue;
                };
                used.entry((glyph.font, weight))
                    .or_default()
                    .extend(chars.chars().filter(|c| !c.is_control()));
            }
        }
    }
    used
}

/// The fonts to embed: each font at each weight it is used in, with only
/// the characters it draws there. A font that cannot be reduced is embedded
/// whole, once.
pub fn embedded(used: &BTreeMap<(usize, u16), BTreeSet<char>>) -> Vec<Embedded> {
    let mut out = Vec::new();
    let mut whole = BTreeSet::new();
    for (&(font, weight), chars) in used {
        let Some(file) = FILES.get(font) else {
            continue;
        };
        match subset(file.bytes, chars, weight) {
            Some(bytes) => out.push(Embedded {
                font,
                weight: Some(weight),
                bytes,
            }),
            None => {
                if whole.insert(font) {
                    out.push(Embedded {
                        font,
                        weight: None,
                        bytes: file.bytes.to_vec(),
                    });
                }
            }
        }
    }
    // A font embedded whole serves every weight.
    out.retain(|e| e.weight.is_none() || !whole.contains(&e.font));
    out
}

/// `data` reduced to `chars`, at `weight` when it is a variable font, as a
/// font a browser takes. None when the font cannot be read or reduced.
pub fn subset(data: &[u8], chars: &BTreeSet<char>, weight: u16) -> Option<Vec<u8>> {
    let font = FontRef::new(data).ok()?;
    let charmap = font.cmap().ok()?;
    let mut remapper = subsetter::GlyphRemapper::new();
    let mut mappings = Vec::new();
    for &c in chars {
        let Some(glyph) = charmap.map_codepoint(c) else {
            continue;
        };
        let old = u16::try_from(glyph.to_u32()).ok()?;
        mappings.push((c, GlyphId::new(u32::from(remapper.remap(old)))));
    }
    let wght = subsetter::Tag::new(b"wght");
    let reduced =
        subsetter::subset_with_variations(data, 0, &[(wght, f32::from(weight))], &remapper).ok()?;
    let cmap = Cmap::from_mappings(mappings).ok()?;
    let mut builder = FontBuilder::new();
    builder.add_table(&cmap).ok()?;
    let os2 = Tag::new(b"OS/2");
    if let Some(table) = font.data_for_tag(os2) {
        builder.add_raw(os2, table.as_bytes().to_vec());
    }
    builder.copy_missing_tables(FontRef::new(&reduced).ok()?);
    Some(builder.build())
}

#[cfg(test)]
#[path = "web_font_tests.rs"]
mod tests;
