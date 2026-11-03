//! Lays out reports (DESIGN.md, Reports): it measures text with the bundled
//! fonts, breaks it into lines, places the controls of a report, and (in
//! later steps) breaks it into pages. It does not depend on egui, so the PDF
//! and `cliotree-cli` use it as the window does, and a report breaks lines and
//! pages in the same places on screen and on paper. Forms are laid out by egui
//! and do not use it.

pub mod chart;
pub mod fonts;
pub mod report;
#[cfg(test)]
mod report_tests;
pub mod text;

pub use text::{Fonts, Glyph, Line, Metrics, Shaped, TextLayout, TextStyle};
