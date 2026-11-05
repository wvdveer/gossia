//! The HTML of a report (DESIGN.md, Reports, "Export as HTML"): one file with
//! its pages, its text as text, only the fonts it uses, and no scripts.

use std::sync::Arc;

use gossia_core::form::FontFamily;
use gossia_layout::report::{Ink, Laid, Paged, Paint, Rect};
use gossia_layout::text::TextStyle;

use super::{pages, single_page};

fn text(x: f32, y: f32, text: &str, style: TextStyle) -> Paint {
    Paint::Text {
        x,
        y,
        text: text.to_owned(),
        style,
        ink: Ink::Text,
    }
}

fn laid(width: f32, height: f32, paints: Vec<Paint>) -> Laid {
    Laid {
        width,
        height,
        paints,
        ..Laid::default()
    }
}

fn count(html: &str, what: &str) -> usize {
    html.matches(what).count()
}

#[test]
fn a_single_page_report_is_one_page_with_its_text_as_text() {
    let body = TextStyle::new(FontFamily::Sans, 12.0);
    let report = laid(
        300.0,
        120.0,
        vec![
            Paint::Fill {
                rect: Rect::new(0.0, 0.0, 300.0, 30.0),
                ink: Ink::Rgba([0x33, 0x66, 0x99, 128]),
            },
            text(8.0, 8.0, "R&D <north>", body.bold()),
            text(8.0, 40.0, "North 12,500", body),
            Paint::Line {
                from: (0.0, 60.0),
                to: (300.0, 60.0),
                width: 0.75,
                ink: Ink::Rule,
            },
            Paint::Border {
                rect: Rect::new(0.0, 70.0, 100.0, 20.0),
                width: 1.0,
                ink: Ink::Rule,
            },
            Paint::Pending { x: 300.0, y: 0.0 },
        ],
    );
    let html = single_page(&report, "Sales & costs");
    assert!(html.starts_with("<!DOCTYPE html>"));
    assert_eq!(count(&html, "<svg class=\"page\""), 1);
    assert_eq!(
        count(
            &html,
            "width=\"300pt\" height=\"120pt\" viewBox=\"0 0 300 120\""
        ),
        1
    );
    assert!(html.contains("<title>Sales &amp; costs</title>"));
    assert!(html.contains(">R&amp;D &lt;north&gt;</text>"));
    assert!(html.contains(">North 12,500</text>"));
    assert!(html.contains("font-weight=\"700\""));
    // Each line keeps the width the layout measured.
    assert_eq!(count(&html, "lengthAdjust=\"spacing\""), 2);
    // The fill, half transparent; the line; the border inside its box.
    assert!(html.contains("fill=\"#336699\" fill-opacity=\"0.5\""));
    assert!(html.contains("<line x1=\"0\" y1=\"60\" x2=\"300\" y2=\"60\""));
    assert!(html.contains("<rect x=\"0.5\" y=\"70.5\" width=\"99\" height=\"19\" fill=\"none\""));
    // Only Inter, the sans serif typeface's font, is embedded; there is no
    // paper to print on, and nothing runs.
    assert_eq!(count(&html, "@font-face"), 1);
    assert!(html.contains("font-family: \"gossia-0\"; src: url(data:font/ttf;base64,"));
    assert!(!html.contains("@page"));
    assert!(html.contains("default-src 'none'"));
    assert!(!html.contains("<script"));
    let generator = format!("Gossia {}", env!("CARGO_PKG_VERSION"));
    assert!(html.contains(&generator));
}

#[test]
fn a_multipage_report_has_each_page_printed_on_its_paper() {
    let body = TextStyle::new(FontFamily::Sans, 12.0);
    let page = |n: usize| {
        laid(
            595.0,
            842.0,
            vec![text(40.0, 40.0, &format!("Page {n}"), body)],
        )
    };
    let paged = Paged {
        pages: vec![page(1), page(2), page(3)],
        width: 595.0,
        height: 842.0,
        ..Paged::default()
    };
    let html = pages(&paged, "Orders");
    assert_eq!(count(&html, "<svg class=\"page\""), 3);
    assert!(html.contains("@page { size: 595pt 842pt; margin: 0; }"));
    assert!(html.contains("break-after: page"));
    assert!(html.contains("aria-label=\"Orders, page 2 of 3\""));
    for n in 1..=3 {
        assert!(html.contains(&format!(">Page {n}</text>")));
    }
    // A report with no pages still has one, empty.
    let empty = Paged {
        width: 595.0,
        height: 842.0,
        ..Paged::default()
    };
    let html = pages(&empty, "Orders");
    assert_eq!(count(&html, "<svg class=\"page\""), 1);
    assert_eq!(count(&html, "@font-face"), 0);
}

#[test]
fn each_typeface_embeds_its_own_font_and_italic_leans() {
    let mono = TextStyle::new(FontFamily::Monospace, 10.0);
    let serif = TextStyle {
        italic: true,
        ..TextStyle::new(FontFamily::Serif, 10.0)
    };
    let report = laid(
        200.0,
        60.0,
        vec![
            text(0.0, 0.0, "x = 1", mono),
            text(0.0, 20.0, "Note", serif),
        ],
    );
    let html = single_page(&report, "Fonts");
    // Geist Mono and Source Serif 4, not Inter, which no character needed.
    assert_eq!(count(&html, "@font-face"), 2);
    assert!(html.contains("\"gossia-1\""));
    assert!(html.contains("\"gossia-2\""));
    assert!(!html.contains("\"gossia-0\""));
    assert!(html.contains("font-family=\"'gossia-1'\""));
    assert!(html.contains("transform=\"matrix(1 0 -0.25 1 "));
}

#[test]
fn a_chart_is_drawn_again_without_scripts_links_or_text() {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="50" onload="alert(1)">
        <script>alert(2)</script>
        <defs><linearGradient id="g"><stop offset="0" stop-color="red"/><stop offset="1" stop-color="blue"/></linearGradient></defs>
        <a href="https://example.com"><rect width="100" height="50" fill="url(#g)"/></a>
        <text x="10" y="30" font-size="12">Total</text>
        <image href="https://example.com/x.png" width="10" height="10"/>
    </svg>"##;
    let chart = |x: f32| Paint::Svg {
        rect: Rect::new(x, 0.0, 200.0, 50.0),
        svg: Arc::from(svg),
        style: None,
    };
    let html = single_page(&laid(400.0, 50.0, vec![chart(0.0), chart(200.0)]), "Chart");
    for banned in ["script", "onload", "alert", "example.com", "<a ", "<text"] {
        assert!(!html.contains(banned), "{banned} is in the HTML");
    }
    // Fitted into its box keeping its proportions, each chart's ids its own.
    assert!(
        html.contains("<svg x=\"0\" y=\"0\" width=\"200\" height=\"50\" viewBox=\"0 0 100 50\">")
    );
    assert!(html.contains("url(#chart1-"));
    assert!(html.contains("url(#chart2-"));
    // The text is drawn as paths, with the application's fonts.
    assert!(count(&html, "<path") >= 2);
}

#[test]
fn a_chart_that_does_not_parse_draws_nothing() {
    let paint = Paint::Svg {
        rect: Rect::new(0.0, 0.0, 100.0, 50.0),
        svg: Arc::from("<svg"),
        style: None,
    };
    let html = single_page(&laid(100.0, 50.0, vec![paint]), "Broken");
    assert_eq!(count(&html, "<svg"), 1);
}
