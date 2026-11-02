//! The bundled fonts (see `assets/fonts/FONTS.md`), and which fonts each
//! typeface of a document is set in.
//!
//! The application draws its window with these files too (`cliotree-app`,
//! `style.rs`), so a report and the window use the same fonts, and the chains
//! below follow the window's: the typeface's own font, then Inter, then egui's
//! Ubuntu Light for the symbols some fonts lack. The window has egui's icons
//! and the Phosphor icons after those; report text never needs them.

use cliotree_core::form::FontFamily as Typeface;

pub const INTER: &[u8] = include_bytes!("../assets/fonts/Inter.ttf");
pub const GEIST_MONO: &[u8] = include_bytes!("../assets/fonts/GeistMono.ttf");
pub const SOURCE_SERIF: &[u8] = include_bytes!("../assets/fonts/SourceSerif4.ttf");
pub const OSWALD: &[u8] = include_bytes!("../assets/fonts/Oswald.ttf");
pub const DANCING_SCRIPT: &[u8] = include_bytes!("../assets/fonts/DancingScript.ttf");
pub const CAVEAT: &[u8] = include_bytes!("../assets/fonts/Caveat.ttf");

/// egui's fonts, behind the bundled ones.
pub const UBUNTU_LIGHT: &[u8] = epaint_default_fonts::UBUNTU_LIGHT;
pub const HACK: &[u8] = epaint_default_fonts::HACK_REGULAR;

/// The weight of ordinary text, and of bold text. The bundled fonts are
/// variable fonts with a weight axis (`wght`).
pub const REGULAR_WEIGHT: f32 = 400.0;
pub const BOLD_WEIGHT: f32 = 700.0;

/// One font file: its name (as the window names it) and its bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FontFile {
    pub name: &'static str,
    pub bytes: &'static [u8],
}

/// Every font a report can be set in, in a fixed order; [`chain`] gives
/// places in this list.
pub const FILES: [FontFile; 8] = [
    FontFile {
        name: "Inter",
        bytes: INTER,
    },
    FontFile {
        name: "Geist Mono",
        bytes: GEIST_MONO,
    },
    FontFile {
        name: "Source Serif 4",
        bytes: SOURCE_SERIF,
    },
    FontFile {
        name: "Oswald",
        bytes: OSWALD,
    },
    FontFile {
        name: "Dancing Script",
        bytes: DANCING_SCRIPT,
    },
    FontFile {
        name: "Caveat",
        bytes: CAVEAT,
    },
    FontFile {
        name: "Ubuntu-Light",
        bytes: UBUNTU_LIGHT,
    },
    FontFile {
        name: "Hack",
        bytes: HACK,
    },
];

const I_INTER: usize = 0;
const I_GEIST_MONO: usize = 1;
const I_SOURCE_SERIF: usize = 2;
const I_OSWALD: usize = 3;
const I_DANCING_SCRIPT: usize = 4;
const I_CAVEAT: usize = 5;
const I_UBUNTU: usize = 6;
const I_HACK: usize = 7;

/// The fonts a typeface is set in, first to last, as places in [`FILES`]. A
/// character is taken from the first font that has it; one that none has is
/// drawn from the first (its "missing glyph" box).
pub fn chain(typeface: Typeface) -> &'static [usize] {
    match typeface {
        Typeface::Sans => &[I_INTER, I_UBUNTU],
        Typeface::Monospace => &[I_GEIST_MONO, I_HACK, I_INTER, I_UBUNTU],
        Typeface::Serif => &[I_SOURCE_SERIF, I_INTER, I_UBUNTU],
        Typeface::Condensed => &[I_OSWALD, I_INTER, I_UBUNTU],
        Typeface::Script => &[I_DANCING_SCRIPT, I_INTER, I_UBUNTU],
        Typeface::Handwriting => &[I_CAVEAT, I_INTER, I_UBUNTU],
    }
}
