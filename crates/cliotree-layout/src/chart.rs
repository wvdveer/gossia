//! The SVG of charts (DESIGN.md, Reports, "Chart functions"): what a chart
//! function returns is checked here before it is painted, with the options
//! every part of the application parses it with: the bundled fonts, the
//! interface font for a family the application does not have, and nothing
//! loaded from outside the SVG (no files or URLs; images only as `data:`
//! URIs).

use std::sync::{Arc, OnceLock};

use crate::fonts;

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
