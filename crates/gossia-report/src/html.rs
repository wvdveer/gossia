//! Writing a report as HTML (DESIGN.md, Reports, "Export as HTML"): one
//! self-contained file, a picture of the report with no scripts, from what
//! `gossia-layout` laid out, as the PDF is.
//!
//! - Each page is an inline SVG in points, so that everything is where the
//!   layout put it: text on the baseline the layout measured, fills,
//!   borders and lines as SVG shapes. Text stays text, so it can be
//!   selected, searched and read aloud; each line is given the width the
//!   layout measured (`textLength`), so the browser's shaping cannot move
//!   where a line ends. Italic is slanted as the window slants it.
//! - The bundled fonts a report uses are embedded, whole, as `data:` URLs:
//!   a font is only embedded when a line of the report is set in it.
//! - A chart's SVG is parsed with usvg and written again by it, with the
//!   options and style sheet the window renders it with. What comes out
//!   has only shapes and embedded images, with its text as paths: no
//!   scripts, event handlers, links or outside files, whatever the chart
//!   function returned.
//! - A content security policy forbids scripts and anything outside the
//!   file, should a browser find any.
//! - A multipage report has a page for each page, each broken onto its own
//!   sheet when printed (`@page` with the report's paper).

use std::collections::BTreeSet;
use std::fmt::Write as _;

use base64::Engine as _;
use gossia_layout::fonts::{self, FILES};
use gossia_layout::report::{Ink, Laid, Paged, Paint, Rect};
use gossia_layout::text::Fonts;

use crate::paper_colour;

/// How far italic text leans, as in the PDF.
const SLANT: f32 = 0.25;

/// A single page report as an HTML page: one page as wide and as tall as
/// the report.
pub fn single_page(laid: &Laid, title: &str) -> String {
    write(title, laid.width, laid.height, &[laid], false)
}

/// A multipage report as an HTML page, page by page on its paper, each
/// printed on a sheet of its own. A report with no pages has one empty page.
pub fn pages(paged: &Paged, title: &str) -> String {
    let empty = Laid::default();
    let mut pages: Vec<&Laid> = paged.pages.iter().collect();
    if pages.is_empty() {
        pages.push(&empty);
    }
    write(title, paged.width, paged.height, &pages, true)
}

fn write(title: &str, width: f32, height: f32, pages: &[&Laid], paper: bool) -> String {
    let (width, height) = (width.max(1.0), height.max(1.0));
    let mut out = String::new();
    out.push_str("<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n");
    out.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    out.push_str(
        "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; \
         style-src 'unsafe-inline'; font-src data:; img-src data:\">\n",
    );
    let _ = writeln!(
        out,
        "<meta name=\"generator\" content=\"Gossia {}\">",
        env!("CARGO_PKG_VERSION")
    );
    let _ = writeln!(out, "<title>{}</title>", escape(title));
    out.push_str("<style>\n");
    let used = used_fonts(pages);
    for &font in &used {
        let file = &FILES[font];
        let _ = writeln!(
            out,
            "@font-face {{ font-family: \"{}\"; src: url(data:font/ttf;base64,{}) \
             format(\"truetype\"); font-weight: 100 900; }}",
            family(font),
            base64::engine::general_purpose::STANDARD.encode(file.bytes)
        );
    }
    if paper {
        let _ = writeln!(
            out,
            "@page {{ size: {}pt {}pt; margin: 0; }}",
            num(width),
            num(height)
        );
    }
    out.push_str(
        "html { background: #e6e9ed; }\n\
         body { margin: 0; padding: 24px 16px; display: flex; flex-direction: column; \
         align-items: center; gap: 24px; }\n\
         .page { display: block; background: #fff; max-width: 100%; height: auto; \
         box-shadow: 0 1px 4px rgba(0, 0, 0, 0.25); }\n\
         .page text { white-space: pre; }\n\
         @media print {\n\
         html { background: none; }\n\
         body { display: block; padding: 0; }\n\
         .page { box-shadow: none; max-width: none; break-after: page; }\n\
         .page:last-child { break-after: auto; }\n\
         }\n",
    );
    out.push_str("</style>\n</head>\n<body>\n");
    let mut charts = 0;
    for (n, laid) in pages.iter().enumerate() {
        let label = if pages.len() > 1 {
            format!("{title}, page {} of {}", n + 1, pages.len())
        } else {
            title.to_owned()
        };
        let _ = writeln!(
            out,
            "<svg class=\"page\" xmlns=\"http://www.w3.org/2000/svg\" \
             xmlns:xlink=\"http://www.w3.org/1999/xlink\" width=\"{w}pt\" height=\"{h}pt\" \
             viewBox=\"0 0 {w} {h}\" aria-label=\"{label}\">",
            w = num(width),
            h = num(height),
            label = escape(&label),
        );
        draw(&mut out, laid, &used, &mut charts);
        out.push_str("</svg>\n");
    }
    out.push_str("</body>\n</html>\n");
    out
}

/// The fonts the report's text is set in, as places in [`FILES`]: those its
/// characters were shaped with.
fn used_fonts(pages: &[&Laid]) -> BTreeSet<usize> {
    let measure = Fonts::shared();
    let mut used = BTreeSet::new();
    for laid in pages {
        for paint in &laid.paints {
            if let Paint::Text { text, style, .. } = paint {
                used.extend(measure.shape(text, style).glyphs.iter().map(|g| g.font));
            }
        }
    }
    used
}

/// The name a bundled font is embedded under, so that it cannot be taken
/// for a font of the reader's system.
fn family(font: usize) -> String {
    format!("gossia-{font}")
}

/// A number as SVG and CSS take it: no more decimals than matter.
fn num(v: f32) -> String {
    let v = (v * 100.0).round() / 100.0;
    if v == v.trunc() {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
    out
}

/// A colour as SVG attributes: `name` and its opacity when not opaque.
fn colour(name: &str, ink: Ink) -> String {
    let [r, g, b, a] = paper_colour(ink);
    let mut out = format!("{name}=\"#{r:02x}{g:02x}{b:02x}\"");
    if a < 255 {
        let _ = write!(out, " {name}-opacity=\"{}\"", num(f32::from(a) / 255.0));
    }
    out
}

fn rect_attrs(r: &Rect) -> String {
    format!(
        "x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"",
        num(r.x),
        num(r.y),
        num(r.w.max(0.0)),
        num(r.h.max(0.0))
    )
}

fn draw(out: &mut String, laid: &Laid, used: &BTreeSet<usize>, charts: &mut usize) {
    let measure = Fonts::shared();
    for paint in &laid.paints {
        match paint {
            Paint::Text {
                x,
                y,
                text,
                style,
                ink,
            } => {
                let shaped = measure.shape(text, style);
                if shaped.glyphs.is_empty() || style.size <= 0.0 {
                    continue;
                }
                let baseline = y + measure.metrics(style).ascent;
                let families: Vec<String> = fonts::chain(style.typeface)
                    .iter()
                    .filter(|f| used.contains(f))
                    .map(|&f| format!("'{}'", family(f)))
                    .collect();
                let _ = write!(
                    out,
                    "<text x=\"{}\" y=\"{}\" font-family=\"{}\" font-size=\"{}\" {}",
                    num(*x),
                    num(baseline),
                    families.join(", "),
                    num(style.size),
                    colour("fill", *ink),
                );
                if style.bold {
                    out.push_str(" font-weight=\"700\"");
                }
                if style.italic {
                    let _ = write!(
                        out,
                        " transform=\"matrix(1 0 {} 1 {} 0)\"",
                        num(-SLANT),
                        num(SLANT * baseline)
                    );
                }
                if text.chars().count() > 1 && shaped.width > 0.0 {
                    let _ = write!(
                        out,
                        " textLength=\"{}\" lengthAdjust=\"spacing\"",
                        num(shaped.width)
                    );
                }
                let _ = writeln!(out, ">{}</text>", escape(text));
            }
            Paint::Fill { rect, ink } => {
                let _ = writeln!(out, "<rect {} {}/>", rect_attrs(rect), colour("fill", *ink));
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
                let _ = writeln!(
                    out,
                    "<rect {} fill=\"none\" {} stroke-width=\"{}\"/>",
                    rect_attrs(&inner),
                    colour("stroke", *ink),
                    num(*width)
                );
            }
            Paint::Line {
                from,
                to,
                width,
                ink,
            } => {
                let _ = writeln!(
                    out,
                    "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" {} stroke-width=\"{}\"/>",
                    num(from.0),
                    num(from.1),
                    num(to.0),
                    num(to.1),
                    colour("stroke", *ink),
                    num(*width)
                );
            }
            // A report is written when it is calculated.
            Paint::Pending { .. } => {}
            Paint::Svg { rect, svg, style } => {
                *charts += 1;
                chart(out, rect, svg, style.as_deref(), *charts);
            }
        }
    }
}

/// A chart's SVG, as usvg writes it again, fitted into its box keeping its
/// proportions and centred, as the window draws it. Its ids are given the
/// chart's number, so that two charts' gradients cannot be mixed up. SVG
/// that does not parse draws nothing: the layout has already put its error
/// there.
fn chart(out: &mut String, rect: &Rect, svg: &str, style: Option<&str>, n: usize) {
    let options = gossia_layout::chart::svg_options_styled(style);
    let Ok(tree) = usvg::Tree::from_str(svg, &options) else {
        return;
    };
    let size = tree.size();
    let written = tree.to_string(&usvg::WriteOptions {
        id_prefix: Some(format!("chart{n}-")),
        indent: usvg::Indent::None,
        attributes_indent: usvg::Indent::None,
        ..usvg::WriteOptions::default()
    });
    // usvg's root element gives only the size; ours places it.
    let Some(inner) = written
        .find('>')
        .map(|start| &written[start + 1..])
        .and_then(|rest| rest.trim_end().strip_suffix("</svg>"))
    else {
        return;
    };
    let _ = writeln!(
        out,
        "<svg {} viewBox=\"0 0 {} {}\">{inner}</svg>",
        rect_attrs(rect),
        num(size.width()),
        num(size.height()),
    );
}

#[cfg(test)]
#[path = "html_tests.rs"]
mod tests;
