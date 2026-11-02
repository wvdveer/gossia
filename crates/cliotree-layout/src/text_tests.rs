use super::*;

fn sans(size: f32) -> TextStyle {
    TextStyle::new(Typeface::Sans, size)
}

fn texts(text: &str, layout: &TextLayout) -> Vec<String> {
    layout
        .lines
        .iter()
        .map(|l| text[l.range.clone()].to_owned())
        .collect()
}

#[test]
fn every_bundled_font_reads_and_every_typeface_has_a_chain() {
    let fonts = Fonts::new();
    assert_eq!(fonts.faces.len(), FILES.len());
    for typeface in Typeface::ALL {
        let chain = fonts::chain(typeface);
        assert!(chain.iter().all(|&i| i < FILES.len()), "{typeface:?}");
        assert!(
            fonts.width("Hello", &TextStyle::new(typeface, 14.0)) > 10.0,
            "{typeface:?}"
        );
    }
}

#[test]
fn the_row_height_is_worked_out_from_the_first_font_as_egui_does() {
    let fonts = Fonts::shared();
    let m = fonts.metrics(&sans(14.0));
    assert_eq!(m.row_height, m.ascent - m.descent + m.line_gap);
    assert!(m.ascent > 10.0 && m.ascent < 16.0, "{m:?}");
    assert!(m.descent < 0.0, "{m:?}");
    for x in [m.ascent, m.descent, m.line_gap] {
        assert_eq!(x * 32.0, (x * 32.0).round(), "{m:?}");
    }
    // Twice the size is (nearly) twice the height.
    let big = fonts.metrics(&sans(28.0));
    assert!(
        (big.row_height - 2.0 * m.row_height).abs() <= 0.1,
        "{big:?} {m:?}"
    );
}

#[test]
fn widths_grow_with_the_text_the_size_and_the_weight() {
    let fonts = Fonts::shared();
    let w = |t: &str, s: &TextStyle| fonts.width(t, s);
    assert_eq!(w("", &sans(14.0)), 0.0);
    assert!(w("Hello world", &sans(14.0)) > w("Hello", &sans(14.0)));
    let small = w("Hello world", &sans(14.0));
    let big = w("Hello world", &sans(28.0));
    assert!((big - 2.0 * small).abs() < 0.01, "{small} {big}");
    assert!(w("Hello world", &sans(14.0).bold()) > small);
    assert_eq!(w("Hello world", &sans(14.0).italic()), small);
    // The monospace font gives every character one width.
    let mono = TextStyle::new(Typeface::Monospace, 14.0);
    assert!((w("iiii", &mono) - w("WWWW", &mono)).abs() < 0.01);
}

#[test]
fn text_is_kerned() {
    let fonts = Fonts::shared();
    let style = sans(40.0);
    let together = fonts.width("AV", &style);
    let apart = fonts.width("A", &style) + fonts.width("V", &style);
    assert!(together < apart, "{together} {apart}");
}

#[test]
fn a_character_the_font_lacks_comes_from_the_next_one() {
    let fonts = Fonts::shared();
    // Oswald has no Greek; Inter does.
    let shaped = fonts.shape("aλ", &TextStyle::new(Typeface::Condensed, 14.0));
    assert_eq!(shaped.glyphs.len(), 2);
    assert_eq!(FILES[shaped.glyphs[0].font].name, "Oswald");
    assert_eq!(FILES[shaped.glyphs[1].font].name, "Inter");
    assert_eq!(shaped.glyphs[1].cluster, 1);
    assert!(shaped.glyphs.iter().all(|g| g.id != 0));
}

#[test]
fn a_tab_is_four_spaces_wide() {
    let fonts = Fonts::shared();
    let style = sans(14.0);
    let tab = fonts.width("a\tb", &style);
    let spaces =
        fonts.width("a", &style) + 4.0 * fonts.width(" ", &style) + fonts.width("b", &style);
    assert!((tab - spaces).abs() < 0.5, "{tab} {spaces}");
}

#[test]
fn without_a_width_only_line_breaks_start_lines() {
    let fonts = Fonts::shared();
    let text = "one two\nthree\r\n\nfour";
    let layout = fonts.layout(text, &sans(14.0), None);
    assert_eq!(texts(text, &layout), ["one two", "three", "", "four"]);
    let m = layout.metrics;
    assert_eq!(layout.height(), 4.0 * m.row_height);
    let empty = fonts.layout("", &sans(14.0), None);
    assert_eq!(empty.lines.len(), 1);
    assert_eq!(empty.width, 0.0);
}

#[test]
fn lines_break_after_spaces_and_fit_the_width() {
    let fonts = Fonts::shared();
    let style = sans(14.0);
    let text = "The quick brown fox jumps over the lazy dog and runs away";
    let max = fonts.width("The quick brown fox", &style) + 1.0;
    let layout = fonts.layout(text, &style, Some(max));
    let lines = texts(text, &layout);
    assert_eq!(lines[0], "The quick brown fox");
    assert!(lines.len() >= 3, "{lines:?}");
    for line in &layout.lines {
        assert!(line.width <= max, "{line:?}");
        // No line starts or ends with a space.
        let t = &text[line.range.clone()];
        assert_eq!(t.trim(), t);
    }
    // Every word is kept, in order.
    assert_eq!(lines.join(" "), text);
    // Glyphs are placed from the start of their line.
    for line in &layout.lines {
        assert_eq!(line.glyphs[0].x, 0.0);
        assert!(line.glyphs.iter().all(|g| line.range.contains(&g.cluster)));
    }
}

#[test]
fn a_hyphen_between_letters_is_a_place_to_break() {
    let fonts = Fonts::shared();
    let style = sans(14.0);
    let text = "well-known";
    let max = fonts.width("well-kn", &style);
    let lines = texts(text, &fonts.layout(text, &style, Some(max)));
    assert_eq!(lines, ["well-", "known"]);
}

#[test]
fn a_word_wider_than_the_line_is_broken_between_its_characters() {
    let fonts = Fonts::shared();
    let style = sans(14.0);
    let text = "Supercalifragilistic go";
    let max = fonts.width("Superca", &style) + 0.5;
    let layout = fonts.layout(text, &style, Some(max));
    let lines = texts(text, &layout);
    assert_eq!(lines[0], "Superca");
    assert_eq!(lines.concat().replace(' ', ""), text.replace(' ', ""));
    assert!(layout.lines.iter().all(|l| l.width <= max), "{lines:?}");
    // Narrower than one character still puts one character on each line.
    let narrow = fonts.layout("abc", &style, Some(1.0));
    assert_eq!(texts("abc", &narrow), ["a", "b", "c"]);
}
