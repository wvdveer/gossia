use std::collections::BTreeSet;

use gossia_core::form::FontFamily;
use gossia_layout::fonts::FILES;
use gossia_layout::report::{Ink, Laid, Paint};
use gossia_layout::text::TextStyle;
use write_fonts::read::{FontRef, TableProvider};

use super::{BOLD, REGULAR, embedded, subset, used_chars};

const TEXT: &str = "Orders 2026 — Total: €1,234.56 Ærøskøbing café “quoted” AV fi";

fn chars(text: &str) -> BTreeSet<char> {
    text.chars().collect()
}

/// The glyph a font maps a character to, and its advance width.
fn glyph(data: &[u8], c: char) -> Option<(u32, i32)> {
    let font = FontRef::new(data).unwrap();
    let id = font.cmap().unwrap().map_codepoint(c)?;
    let hmtx = font.hmtx().unwrap();
    let advance = hmtx.advance(id).map_or(0, i32::from);
    Some((id.to_u32(), advance))
}

/// Every bundled font is reduced, at both weights, to a font that maps each
/// character it has to a glyph, with the tables a browser asks for, far
/// smaller than the whole font.
#[test]
fn every_bundled_font_is_reduced_to_what_a_browser_takes() {
    let wanted = chars(TEXT);
    for file in FILES {
        for weight in [REGULAR, BOLD] {
            let out = subset(file.bytes, &wanted, weight)
                .unwrap_or_else(|| panic!("{} at {weight} was not reduced", file.name));
            let font = FontRef::new(&out).unwrap();
            for tag in [
                b"cmap", b"OS/2", b"head", b"hhea", b"hmtx", b"maxp", b"name", b"post",
            ] {
                let tag = write_fonts::types::Tag::new(tag);
                assert!(
                    font.data_for_tag(tag).is_some(),
                    "{} lacks {tag}",
                    file.name
                );
            }
            for c in wanted.iter().filter(|c| !c.is_whitespace()) {
                assert_eq!(
                    glyph(file.bytes, *c).is_some(),
                    glyph(&out, *c).is_some(),
                    "{} {c}",
                    file.name
                );
            }
            assert!(
                out.len() * 4 < file.bytes.len(),
                "{}: {} of {} bytes",
                file.name,
                out.len(),
                file.bytes.len()
            );
        }
    }
}

/// Bold is the variable font instanced at 700: its letters are wider.
#[test]
fn bold_is_the_font_at_its_bold_weight() {
    let inter = FILES[0].bytes;
    let regular = subset(inter, &chars("W"), REGULAR).unwrap();
    let bold = subset(inter, &chars("W"), BOLD).unwrap();
    let (_, regular) = glyph(&regular, 'W').unwrap();
    let (_, bold) = glyph(&bold, 'W').unwrap();
    assert!(bold > regular, "{bold} > {regular}");
    // A character the report does not use is left out.
    assert!(glyph(&subset(inter, &chars("W"), REGULAR).unwrap(), 'x').is_none());
}

/// The characters are gathered by font and weight from the report's text.
#[test]
fn the_characters_are_gathered_by_font_and_weight() {
    let style = TextStyle::new(FontFamily::Sans, 12.0);
    let bold = TextStyle {
        bold: true,
        ..style
    };
    let mono = TextStyle::new(FontFamily::Monospace, 12.0);
    let text = |t: &str, style| Paint::Text {
        x: 0.0,
        y: 0.0,
        text: t.to_owned(),
        style,
        ink: Ink::Text,
    };
    let laid = Laid {
        paints: vec![text("Total", style), text("Sum", bold), text("x=1", mono)],
        ..Laid::default()
    };
    let used = used_chars(&[&laid]);
    assert_eq!(used[&(0, REGULAR)], chars("Total"));
    assert_eq!(used[&(0, BOLD)], chars("Sum"));
    assert_eq!(used[&(1, REGULAR)], chars("x=1"));
    assert_eq!(used.len(), 3);
    let fonts = embedded(&used);
    assert_eq!(fonts.len(), 3);
    assert!(fonts.iter().all(|f| f.weight.is_some()));
}
