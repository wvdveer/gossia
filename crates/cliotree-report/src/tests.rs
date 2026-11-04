//! The PDF of a report (DESIGN.md, Testing, "PDF output"): its pages and
//! their sizes, the text of each page read back, and the fonts embedded.

use cliotree_core::form::FontFamily;
use cliotree_layout::report::{Ink, Laid, Paged, Paint, Rect};
use cliotree_layout::text::TextStyle;

use crate::{pages, paper_colour, single_page};

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

/// The text of each page, as words.
fn words(pdf: &[u8]) -> Vec<Vec<String>> {
    pdf_extract::extract_text_from_mem_by_pages(pdf)
        .expect("the PDF reads back")
        .iter()
        .map(|page| page.split_whitespace().map(str::to_owned).collect())
        .collect()
}

fn count(pdf: &[u8], what: &str) -> usize {
    let bytes = what.as_bytes();
    pdf.windows(bytes.len()).filter(|w| *w == bytes).count()
}

#[test]
fn a_single_page_report_is_one_page_as_big_as_the_report() {
    let body = TextStyle::new(FontFamily::Sans, 12.0);
    let report = laid(
        300.0,
        120.0,
        vec![
            Paint::Fill {
                rect: Rect::new(0.0, 0.0, 300.0, 30.0),
                ink: Ink::Rgba([0x33, 0x66, 0x99, 255]),
            },
            text(8.0, 8.0, "Quarterly sales", body.bold()),
            text(8.0, 40.0, "North 12,500", body),
            Paint::Line {
                from: (0.0, 60.0),
                to: (300.0, 60.0),
                width: 0.75,
                ink: Ink::Rule,
            },
            Paint::Pending { x: 300.0, y: 0.0 },
        ],
    );
    let pdf = single_page(&report, "Sales").unwrap();
    assert!(pdf.starts_with(b"%PDF-"));
    assert_eq!(count(&pdf, "/MediaBox[0 0 300 120]"), 1);
    assert_eq!(words(&pdf), [vec!["Quarterly", "sales", "North", "12,500"]]);
    // The fonts are embedded, as subsets.
    assert!(count(&pdf, "/FontFile") >= 1);
    assert_eq!(
        count(&pdf, r#"<rdf:li xml:lang="x-default">Sales</rdf:li>"#),
        1
    );
    assert!(count(&pdf, "ClioTree 0.2.0") >= 1);
}

#[test]
fn a_multipage_report_has_each_page_on_its_paper() {
    let body = TextStyle::new(FontFamily::Serif, 10.0);
    let paged = Paged {
        pages: vec![
            laid(595.0, 842.0, vec![text(56.0, 56.0, "Page one", body)]),
            laid(
                595.0,
                842.0,
                vec![text(56.0, 56.0, "Page two", body.italic())],
            ),
            laid(
                595.0,
                842.0,
                vec![text(
                    56.0,
                    56.0,
                    "Mono 1\t2",
                    TextStyle::new(FontFamily::Monospace, 9.0),
                )],
            ),
        ],
        width: 595.0,
        height: 842.0,
        warnings: Vec::new(),
    };
    let pdf = pages(&paged, "Report").unwrap();
    assert_eq!(count(&pdf, "/MediaBox[0 0 595 842]"), 3);
    assert_eq!(
        words(&pdf),
        [
            vec!["Page", "one"],
            vec!["Page", "two"],
            vec!["Mono", "1", "2"]
        ]
    );
}

#[test]
fn a_report_without_pages_is_one_empty_page() {
    let paged = Paged {
        width: 612.0,
        height: 792.0,
        ..Paged::default()
    };
    let pdf = pages(&paged, "Empty").unwrap();
    assert_eq!(count(&pdf, "/MediaBox[0 0 612 792]"), 1);
}

#[test]
fn a_chart_goes_in_as_vector_graphics_with_its_text() {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100" color="#0b0b0b">
        <rect width="100%" height="100%" fill="#fcfcfb" class="background"/>
        <rect x="10" y="10" width="40" height="80" fill="#2a78d6"/>
        <text x="60" y="50" font-size="12" fill="currentColor">Sales chart</text>
    </svg>"##;
    let report = laid(
        200.0,
        100.0,
        vec![Paint::Svg {
            rect: Rect::new(0.0, 0.0, 200.0, 100.0),
            svg: svg.into(),
            style: cliotree_layout::chart::style_sheet(
                Some(Ink::Rgba([255, 255, 255, 255])),
                Some(Ink::Rgba([0, 0, 0, 255])),
            )
            .map(Into::into),
        }],
    );
    let pdf = single_page(&report, "Chart").unwrap();
    // Text in SVG is placed glyph by glyph, so it reads back without its
    // spaces.
    assert_eq!(words(&pdf).concat().concat(), "Saleschart");
    // Not a picture of the chart.
    assert_eq!(count(&pdf, "/Subtype /Image"), 0);
}

#[test]
fn the_colours_on_paper_are_the_light_themes() {
    assert_eq!(paper_colour(Ink::Text), [0x1D, 0x25, 0x2D, 255]);
    assert_eq!(paper_colour(Ink::Rgba([1, 2, 3, 4])), [1, 2, 3, 4]);
}
