//! The SVG of charts (DESIGN.md, Reports, "Chart functions"): what a chart
//! function returns is checked here before it is painted, with the options
//! every part of the application parses it with: the bundled fonts, the
//! interface font for a family the application does not have, and nothing
//! loaded from outside the SVG (no files or URLs; images only as `data:`
//! URIs).

use std::sync::{Arc, OnceLock};

use crate::fonts;
use crate::report::Ink;

/// The largest SVG a chart function may return, until the Options page sets
/// it (DESIGN.md, "Settings and limits").
pub const SVG_LIMIT: usize = 5 << 20;

/// The size of a chart that has none of its own, in points.
pub const CHART_SIZE: (f32, f32) = (
    cliotree_core::form::CHART_SIZE.0 as f32,
    cliotree_core::form::CHART_SIZE.1 as f32,
);

/// The bundled fonts, for the text in charts.
fn font_database() -> Arc<usvg::fontdb::Database> {
    static DB: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = usvg::fontdb::Database::new();
        for file in fonts::FILES {
            db.load_font_data(file.bytes.to_vec());
        }
        db.set_sans_serif_family("Inter");
        db.set_serif_family("Source Serif 4");
        db.set_monospace_family("Geist Mono");
        db.set_cursive_family("Dancing Script");
        Arc::new(db)
    })
    .clone()
}

/// The options SVG is parsed with.
pub fn svg_options() -> usvg::Options<'static> {
    usvg::Options {
        font_family: "Inter".to_owned(),
        font_size: 10.0,
        // A point is a CSS pixel here: a chart's `width` and `height` are in
        // points and its SVG in user units.
        dpi: 72.0,
        fontdb: font_database(),
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: usvg::ImageHrefResolver::default_data_resolver(),
            // Never a file or a URL.
            resolve_string: Box::new(|_, _| None),
        },
        ..usvg::Options::default()
    }
}

/// The style sheet that gives a chart its colours, or None for the colours
/// it has: the root element's `color` (which `currentColor` follows) is the
/// text colour; with a fill, an element of class `background` is
/// transparent (the fill is painted behind the chart), and the classes
/// `surface` and `surface-stroke` fill and stroke with the fill colour, as
/// it shows on the white page.
pub fn style_sheet(text: Option<Ink>, fill: Option<Ink>) -> Option<String> {
    let mut css = String::new();
    if let Some(Ink::Rgba([r, g, b, a])) = text {
        let a = f32::from(a) / 255.0;
        css.push_str(&format!("svg {{ color: rgba({r}, {g}, {b}, {a}) }}\n"));
    }
    if let Some(Ink::Rgba([r, g, b, a])) = fill {
        let over_white = |c: u8| {
            let a = f32::from(a) / 255.0;
            (f32::from(c) * a + 255.0 * (1.0 - a)).round() as u8
        };
        let hex = format!(
            "#{:02x}{:02x}{:02x}",
            over_white(r),
            over_white(g),
            over_white(b)
        );
        css.push_str(&format!(
            ".background {{ fill: none }}\n.surface {{ fill: {hex} }}\n.surface-stroke {{ stroke: {hex} }}\n"
        ));
    }
    (!css.is_empty()).then_some(css)
}

/// The options SVG is parsed with, with a chart's style sheet.
pub fn svg_options_styled(style: Option<&str>) -> usvg::Options<'static> {
    usvg::Options {
        style_sheet: style.map(str::to_owned),
        ..svg_options()
    }
}

/// What a chart function returned, parsed; or why it cannot be drawn, as
/// the error value and its message.
pub fn parse_svg(text: &str, limit: usize) -> Result<usvg::Tree, String> {
    if text.len() > limit {
        return Err(format!(
            "#VALUE!: the chart's SVG is {} bytes, more than the limit of {limit}",
            text.len()
        ));
    }
    usvg::Tree::from_str(text, &svg_options()).map_err(|e| format!("#VALUE!: {e}"))
}
