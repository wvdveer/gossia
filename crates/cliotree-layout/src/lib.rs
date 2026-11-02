//! Lays out reports (DESIGN.md, Reports): it measures text with the bundled
//! fonts, breaks it into lines, and (in later steps) places the controls of a
//! report and breaks it into pages. It does not depend on egui, so the PDF
//! and `cliotree-cli` use it as the window does, and a report breaks lines and
//! pages in the same places on screen and on paper. Forms are laid out by egui
//! and do not use it.

pub mod fonts;
pub mod text;

pub use text::{Fonts, Glyph, Line, Metrics, Shaped, TextLayout, TextStyle};
