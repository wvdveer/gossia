//! The look of the window (DESIGN.md, "Look and feel"): the colours of the
//! light and dark themes, the sizes, and the fonts. Every colour the window
//! draws with comes from a [`Palette`], so the two themes stay in step.
//!
//! The fonts are bundled in `gossia-layout` (see its `assets/fonts/FONTS.md`),
//! which lays out reports with the same files. Inter is the interface font and
//! Geist Mono the font of formulas and paths; both are variable fonts, so one
//! file gives every weight. The others (a serif, a condensed sans, a script and
//! a handwriting font) are there for forms and reports.
//!
//! The fonts and the style are installed when the first frame is drawn and
//! take effect on the next one; [`ready`] says whether they are in use.

use egui::epaint::text::{FontTweak, VariationCoords};
use egui::{
    Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Response,
    RichText, Sense, Stroke, Style, TextStyle, TextWrapMode, Theme, Visuals,
};
pub use gossia_core::form::FontFamily as Typeface;

// ------------------------------------------------------------- sizes

/// The height of a row of the tree grid, the tabular view and a form's table.
pub const ROW_HEIGHT: f32 = 28.0;
/// The height of the header row of a grid.
pub const HEADER_HEIGHT: f32 = 30.0;
/// Text sizes, in points.
pub const BODY_SIZE: f32 = 14.0;
pub const SMALL_SIZE: f32 = 12.0;
pub const BADGE_SIZE: f32 = 11.0;
pub const HEADING_SIZE: f32 = 16.0;
pub const MONOSPACE_SIZE: f32 = 13.0;
/// The badge in the Type column.
pub const BADGE_HEIGHT: f32 = 20.0;
const BADGE_PADDING: f32 = 6.0;
/// The marker of a cell waiting to be calculated.
pub const MARKER_DIAMETER: f32 = 6.0;
/// Corner radii: small for controls, larger for panels and popups.
pub const CONTROL_RADIUS: u8 = 4;
pub const PANEL_RADIUS: u8 = 6;
/// The weight of [`medium`] text.
const MEDIUM_WEIGHT: f32 = 560.0;
/// The weight of bold text in forms, as reports lay it out
/// (`gossia_layout::fonts`).
const BOLD_WEIGHT: f32 = 700.0;

// ------------------------------------------------------------ colours

/// The colours of a badge in the Type column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Badge {
    pub fill: Color32,
    pub text: Color32,
}

/// The colours of one theme.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Palette {
    /// Behind the menu, the bars and the tabs.
    pub chrome: Color32,
    /// Behind the grid and the panels.
    pub panel: Color32,
    /// Inside a text field.
    pub field: Color32,
    pub border: Color32,
    /// Every other row of a grid.
    pub stripe: Color32,
    /// A button at rest.
    pub control: Color32,
    pub hover: Color32,
    pub selection: Color32,
    pub selection_outline: Color32,
    pub accent: Color32,
    /// Behind white text: the button of the main action.
    pub primary: Color32,
    pub text: Color32,
    pub text_weak: Color32,
    /// Laid over a cell whose value a formula computes. Translucent, so it
    /// shows on a stripe, a hovered row and a selected row alike.
    pub computed: Color32,
    pub error: Color32,
    /// Behind an error value.
    pub error_fill: Color32,
    pub warning: Color32,
    pub success: Color32,
    pub pending: Color32,
    /// A value that is out of date because the workbook is not trusted.
    pub stale: Color32,
    pub banner_fill: Color32,
    pub banner_text: Color32,
    pub code_bg: Color32,
    pub string: Badge,
    pub number: Badge,
    pub integer: Badge,
    pub boolean: Badge,
    pub object: Badge,
    pub array: Badge,
    /// A type with no `type`: any value. Neutral, as it says nothing.
    pub any: Badge,
    pub date: Badge,
}

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

const fn badge(fill: u32, text: u32) -> Badge {
    Badge {
        fill: hex(fill),
        text: hex(text),
    }
}

pub const DARK: Palette = Palette {
    chrome: hex(0x15181D),
    panel: hex(0x1E232B),
    field: hex(0x13161B),
    border: hex(0x343B46),
    stripe: hex(0x1A1F26),
    control: hex(0x262D37),
    hover: hex(0x2E3642),
    selection: hex(0x214A68),
    selection_outline: hex(0x65B5FF),
    accent: hex(0x65B5FF),
    primary: hex(0x2C77B8),
    text: hex(0xE7ECF2),
    text_weak: hex(0x9AA6B2),
    // rgb(60, 140, 210) at 18%, premultiplied.
    computed: Color32::from_rgba_premultiplied(11, 25, 38, 46),
    error: hex(0xF06A6A),
    error_fill: hex(0x4A2529),
    warning: hex(0xF2B84B),
    success: hex(0x5CCB8A),
    pending: hex(0xF2B84B),
    stale: hex(0x707A86),
    banner_fill: hex(0x3B2F1A),
    banner_text: hex(0xF5C96A),
    code_bg: hex(0x262D37),
    string: badge(0x3B2A4A, 0xD7B8FF),
    number: badge(0x213B59, 0x8AC7FF),
    integer: badge(0x1E4A4D, 0x72DDD2),
    boolean: badge(0x4A3520, 0xFFC46B),
    object: badge(0x243D34, 0x86D9A7),
    array: badge(0x3B2F55, 0xC7A7FF),
    any: badge(0x33363B, 0xC9CDD3),
    date: badge(0x4A2938, 0xFF9FC0),
};

pub const LIGHT: Palette = Palette {
    chrome: hex(0xF3F5F7),
    panel: hex(0xFFFFFF),
    field: hex(0xFFFFFF),
    border: hex(0xD5DCE3),
    stripe: hex(0xF7F9FB),
    control: hex(0xE9EDF1),
    hover: hex(0xE2EBF3),
    // Stronger than the computed tint, so that a computed cell does not look
    // selected.
    selection: hex(0xCCE5FA),
    selection_outline: hex(0x1D74B5),
    accent: hex(0x247BC1),
    primary: hex(0x247BC1),
    text: hex(0x1D252D),
    text_weak: hex(0x5E6B78),
    // rgb(90, 140, 170) at 10%, premultiplied: a grey blue, not the
    // selection's blue.
    computed: Color32::from_rgba_premultiplied(9, 14, 17, 25),
    error: hex(0xC93636),
    error_fill: hex(0xFDEAEA),
    warning: hex(0xA86800),
    success: hex(0x237A4A),
    pending: hex(0xB87700),
    // Darker than the first design (#8B949E), which was faint on white.
    stale: hex(0x7A838D),
    banner_fill: hex(0xFFF2D6),
    banner_text: hex(0x7A4D00),
    code_bg: hex(0xEEF1F4),
    string: badge(0xF1E8FA, 0x6E3AA0),
    number: badge(0xE5F1FC, 0x17639A),
    integer: badge(0xE2F6F3, 0x14736D),
    boolean: badge(0xFFF0D7, 0x945A00),
    object: badge(0xE3F4E9, 0x267347),
    array: badge(0xEEE8FC, 0x6240A0),
    any: badge(0xECEEF1, 0x4A5058),
    date: badge(0xFBE7EF, 0xA33F68),
};

impl Palette {
    pub fn of(theme: Theme) -> &'static Palette {
        match theme {
            Theme::Dark => &DARK,
            Theme::Light => &LIGHT,
        }
    }

    /// The palette of the theme a `Ui` is drawn in.
    pub fn of_ui(ui: &egui::Ui) -> &'static Palette {
        Self::of_visuals(ui.visuals())
    }

    pub fn of_visuals(visuals: &Visuals) -> &'static Palette {
        if visuals.dark_mode { &DARK } else { &LIGHT }
    }

    /// The badge of a type, by its kind.
    pub fn badge(&self, kind: &gossia_core::Kind) -> Badge {
        use gossia_core::Kind;
        match kind {
            Kind::String(_) | Kind::Enum { .. } => self.string,
            Kind::Integer(_) => self.integer,
            Kind::Number(_) => self.number,
            Kind::Bool => self.boolean,
            Kind::Date | Kind::DateTime => self.date,
            Kind::Array { .. } => self.array,
            Kind::Object { .. } | Kind::Map { .. } => self.object,
            Kind::Any => self.any,
        }
    }
}

// -------------------------------------------------------------- fonts

/// The font families the window knows, besides egui's `Proportional` (Inter)
/// and `Monospace` (Geist Mono).
pub const MEDIUM: &str = "medium";
pub const SERIF: &str = "serif";
pub const CONDENSED: &str = "condensed";
pub const SCRIPT: &str = "script";
pub const HANDWRITING: &str = "handwriting";
/// The Phosphor icons (see [`crate::icons`]).
pub const ICONS: &str = "phosphor";

/// The egui family a typeface of a document is drawn with.
pub fn family(typeface: Typeface) -> FontFamily {
    match typeface {
        Typeface::Sans => FontFamily::Proportional,
        Typeface::Monospace => FontFamily::Monospace,
        Typeface::Serif => FontFamily::Name(SERIF.into()),
        Typeface::Condensed => FontFamily::Name(CONDENSED.into()),
        Typeface::Script => FontFamily::Name(SCRIPT.into()),
        Typeface::Handwriting => FontFamily::Name(HANDWRITING.into()),
    }
}

/// The egui family of a typeface in bold: the same font file at the bold
/// weight, as a family of its own (egui has no weight in a font id).
pub fn bold_family(typeface: Typeface) -> FontFamily {
    FontFamily::Name(format!("{}-bold", typeface.key()).into())
}

/// The bundled font of each typeface, and the name it is registered under.
fn typeface_font(typeface: Typeface) -> (&'static [u8], &'static str) {
    use gossia_layout::fonts::{CAVEAT, DANCING_SCRIPT, GEIST_MONO, INTER, OSWALD, SOURCE_SERIF};
    match typeface {
        Typeface::Sans => (INTER, "Inter"),
        Typeface::Serif => (SOURCE_SERIF, "Source Serif 4"),
        Typeface::Monospace => (GEIST_MONO, "Geist Mono"),
        Typeface::Condensed => (OSWALD, "Oswald"),
        Typeface::Script => (DANCING_SCRIPT, "Dancing Script"),
        Typeface::Handwriting => (CAVEAT, "Caveat"),
    }
}

/// The name of a typeface in a menu: what it is, and the font it is.
pub fn typeface_label(typeface: Typeface) -> String {
    let font = match typeface {
        Typeface::Sans => "Inter",
        Typeface::Serif => "Source Serif 4",
        Typeface::Monospace => "Geist Mono",
        Typeface::Condensed => "Oswald",
        Typeface::Script => "Dancing Script",
        Typeface::Handwriting => "Caveat",
    };
    format!("{} ({font})", typeface.label())
}

/// The sizes the View menu offers for the data font, in points.
pub const DATA_FONT_SIZES: [u32; 10] = [10, 11, 12, 13, 14, 16, 18, 20, 24, 28];

/// The font of the Data tree and Raw views of every tab of a workbook (View >
/// Font). It is kept in the workbook (`view.json`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DataFont {
    pub family: Typeface,
    pub size: u32,
}

impl Default for DataFont {
    fn default() -> DataFont {
        DataFont {
            family: Typeface::Sans,
            size: BODY_SIZE as u32,
        }
    }
}

/// The text sizes of the window, scaled by `scale`, in `body` (the family of
/// everything but code) and the monospace family.
fn text_styles(body: FontFamily, scale: f32) -> std::collections::BTreeMap<TextStyle, FontId> {
    let heading = if body == FontFamily::Proportional {
        FontFamily::Name(MEDIUM.into())
    } else {
        body.clone()
    };
    // Sans is the interface pairing: Inter for text, Geist Mono for code. Any
    // other typeface is used for code too.
    let code = if body == FontFamily::Proportional {
        FontFamily::Monospace
    } else {
        body.clone()
    };
    [
        (
            TextStyle::Small,
            FontId::new(SMALL_SIZE * scale, body.clone()),
        ),
        (
            TextStyle::Body,
            FontId::new(BODY_SIZE * scale, body.clone()),
        ),
        (TextStyle::Button, FontId::new(BODY_SIZE * scale, body)),
        (
            TextStyle::Heading,
            FontId::new(HEADING_SIZE * scale, heading),
        ),
        (
            TextStyle::Monospace,
            FontId::new(MONOSPACE_SIZE * scale, code),
        ),
    ]
    .into()
}

/// Makes what is drawn with `style` use a typeface and a size: every text
/// style takes the typeface and is scaled so that the body text has the size.
/// Under Sans, code (formulas, the Raw view) stays in Geist Mono; under any
/// other typeface it takes that one too. None keeps the interface font, or
/// its size. Type badges follow through [`badge_label`].
pub fn use_font(style: &mut Style, typeface: Option<Typeface>, size: Option<u32>) {
    use_font_in(style, typeface, size, false);
}

/// As [`use_font`], in bold when `bold` is set: every text style, headings
/// and code included, takes the bold weight of its typeface.
pub fn use_font_in(style: &mut Style, typeface: Option<Typeface>, size: Option<u32>, bold: bool) {
    let scale = size.map_or(1.0, |s| s as f32 / BODY_SIZE);
    if !bold {
        let body = typeface.map_or(FontFamily::Proportional, family);
        style.text_styles = text_styles(body, scale);
        return;
    }
    let typeface = typeface.unwrap_or(Typeface::Sans);
    let mut styles = text_styles(bold_family(typeface), scale);
    if typeface == Typeface::Sans
        && let Some(code) = styles.get_mut(&TextStyle::Monospace)
    {
        code.family = bold_family(Typeface::Monospace);
    }
    style.text_styles = styles;
}

/// The height of a row of a grid for the body text of this `Ui`: the standard
/// height, or more for a larger font.
pub fn row_height(ui: &egui::Ui) -> f32 {
    (ui.text_style_height(&TextStyle::Body) + 11.0).max(ROW_HEIGHT)
}

/// The height of a row of the tree grid or the tabular view, at the row
/// density of the Display options.
pub fn grid_row_height(ui: &egui::Ui, density: crate::prefs::Density) -> f32 {
    let (padding, least) = density.padding();
    (ui.text_style_height(&TextStyle::Body) + padding).max(least)
}

/// The height of a row whose text is in a typeface and a size (each None for
/// the interface's), as [`row_height`] gives it.
pub fn row_height_in(ui: &egui::Ui, typeface: Option<Typeface>, size: Option<u32>) -> f32 {
    let mut style = (**ui.style()).clone();
    use_font(&mut style, typeface, size);
    let body = TextStyle::Body.resolve(&style);
    (ui.fonts_mut(|f| f.row_height(&body)) + 11.0).max(ROW_HEIGHT)
}

fn font(bytes: &'static [u8]) -> FontData {
    FontData::from_static(bytes)
}

/// A text font at a weight, without its private use characters (see
/// [`without_private_use`]).
fn weighted(bytes: &'static [u8], weight: f32) -> FontData {
    FontData::from_owned(without_private_use(bytes)).tweak(FontTweak {
        coords: VariationCoords::new([(b"wght", weight)]),
        ..FontTweak::default()
    })
}

/// The private use area of the Basic Multilingual Plane, where the icons are.
const PRIVATE_USE: std::ops::RangeInclusive<u32> = 0xE000..=0xF8FF;

/// The font with no character of the private use area in its character map.
/// The icons are there, and some text fonts (Inter has 745) have glyphs of
/// their own there, which would be drawn instead of an icon in text that is
/// not in the icon font, as the file dialog's is. The glyphs stay in the font;
/// only the characters that lead to them are taken out. A font that cannot be
/// read is returned as it is.
fn without_private_use(bytes: &[u8]) -> Vec<u8> {
    use skrifa::MetadataProvider;
    let Ok(font) = skrifa::FontRef::new(bytes) else {
        return bytes.to_vec();
    };
    let mut mappings: Vec<(u32, u32)> = font
        .charmap()
        .mappings()
        .filter(|(c, _)| !PRIVATE_USE.contains(c))
        .map(|(c, glyph)| (c, glyph.to_u32()))
        .collect();
    mappings.sort_unstable();
    replace_table(bytes, *b"cmap", &character_map(&mappings))
}

/// A `cmap` table of one Unicode subtable of format 12 (any plane), for
/// mappings sorted by character.
fn character_map(mappings: &[(u32, u32)]) -> Vec<u8> {
    // Runs of characters whose glyphs are also consecutive.
    let mut groups: Vec<(u32, u32, u32)> = Vec::new();
    for &(c, glyph) in mappings {
        match groups.last_mut() {
            Some((start, end, first)) if c == *end + 1 && glyph == *first + (c - *start) => {
                *end = c;
            }
            _ => groups.push((c, c, glyph)),
        }
    }
    let mut table = Vec::new();
    // The header: version 0, one subtable, Windows Unicode full repertoire.
    for word in [0u16, 1, 3, 10] {
        table.extend(word.to_be_bytes());
    }
    table.extend(12u32.to_be_bytes());
    // The subtable.
    table.extend(12u16.to_be_bytes());
    table.extend(0u16.to_be_bytes());
    table.extend((16 + 12 * groups.len() as u32).to_be_bytes());
    table.extend(0u32.to_be_bytes());
    table.extend((groups.len() as u32).to_be_bytes());
    for (start, end, glyph) in groups {
        table.extend(start.to_be_bytes());
        table.extend(end.to_be_bytes());
        table.extend(glyph.to_be_bytes());
    }
    table
}

/// The font file with the table `tag` replaced by `data`, laid out again
/// (tables in the order of the directory, each on four bytes). The
/// directory's own fields and the other tables are unchanged.
fn replace_table(font: &[u8], tag: [u8; 4], data: &[u8]) -> Vec<u8> {
    let be32 = |at: usize| u32::from_be_bytes(font[at..at + 4].try_into().expect("4 bytes"));
    let count = u16::from_be_bytes([font[4], font[5]]) as usize;
    let tables: Vec<([u8; 4], &[u8])> = (0..count)
        .map(|i| {
            let record = 12 + 16 * i;
            let name: [u8; 4] = font[record..record + 4].try_into().expect("4 bytes");
            let offset = be32(record + 8) as usize;
            let length = be32(record + 12) as usize;
            let body = if name == tag {
                data
            } else {
                &font[offset..offset + length]
            };
            (name, body)
        })
        .collect();
    let mut out = font[..12].to_vec();
    let mut offset = 12 + 16 * count;
    for (name, body) in &tables {
        out.extend(name);
        out.extend(checksum(body).to_be_bytes());
        out.extend((offset as u32).to_be_bytes());
        out.extend((body.len() as u32).to_be_bytes());
        offset += body.len().next_multiple_of(4);
    }
    for (_, body) in &tables {
        out.extend(*body);
        out.resize(out.len().next_multiple_of(4), 0);
    }
    out
}

/// The checksum of a table: its 32-bit words summed, the last one padded.
fn checksum(body: &[u8]) -> u32 {
    body.chunks(4).fold(0u32, |sum, chunk| {
        let mut word = [0u8; 4];
        word[..chunk.len()].copy_from_slice(chunk);
        sum.wrapping_add(u32::from_be_bytes(word))
    })
}

/// egui's own fonts, with the bundled ones in front of them. What a font lacks
/// is looked for in the fonts after it: egui's emoji fonts, then the icons.
pub fn fonts() -> FontDefinitions {
    fonts_with(None)
}

/// The name of the system's font for Japanese or Chinese, when the
/// interface's language needs one (see `i18n`).
const CJK: &str = "CJK";

/// How far the font for Japanese or Chinese is lowered, as a fraction of
/// the size: checked by eye against Inter with Yu Gothic and Microsoft YaHei.
const CJK_Y_OFFSET: f32 = 0.15;

/// Gives the window its fonts, with the system's font for Japanese or
/// Chinese behind each family's own when there is one.
pub fn set_fonts_with(ctx: &egui::Context, cjk: Option<crate::i18n::SystemFont>) {
    ctx.set_fonts(fonts_with(cjk.as_ref()));
}

/// [`fonts`], with a font for Japanese or Chinese after Inter and Ubuntu in
/// every family, so that Latin text keeps the bundled fonts.
pub fn fonts_with(cjk: Option<&crate::i18n::SystemFont>) -> FontDefinitions {
    use gossia_layout::fonts::{CAVEAT, DANCING_SCRIPT, GEIST_MONO, INTER, OSWALD, SOURCE_SERIF};
    let mut fonts = FontDefinitions::default();
    let mut add = |name: &str, data: FontData| {
        fonts.font_data.insert(name.to_owned(), data.into());
    };
    if let Some(cjk) = cjk {
        let mut data = FontData::from_owned(cjk.bytes.clone()).tweak(FontTweak {
            // egui places a fallback font's glyphs by its own ascent, which
            // in these fonts is taller than Inter's: lowered a little, kana
            // and Han characters sit on Inter's baseline.
            y_offset_factor: CJK_Y_OFFSET,
            ..FontTweak::default()
        });
        data.index = cjk.index;
        add(CJK, data);
    }
    add("Inter", weighted(INTER, 400.0));
    add("Inter Medium", weighted(INTER, MEDIUM_WEIGHT));
    add("Geist Mono", weighted(GEIST_MONO, 400.0));
    add("Source Serif 4", weighted(SOURCE_SERIF, 400.0));
    add("Oswald", weighted(OSWALD, 400.0));
    add("Dancing Script", weighted(DANCING_SCRIPT, 400.0));
    add("Caveat", weighted(CAVEAT, 400.0));
    add(ICONS, font(include_bytes!("../assets/Phosphor.ttf")));
    for typeface in Typeface::ALL {
        let (bytes, name) = typeface_font(typeface);
        add(&format!("{name} Bold"), weighted(bytes, BOLD_WEIGHT));
    }

    // egui's last resorts (its icons and emoji), from its own families, then
    // the Phosphor icons, which are in the private use area.
    let mut fallbacks: Vec<String> = fonts
        .families
        .get(&FontFamily::Proportional)
        .into_iter()
        .flatten()
        .filter(|name| *name != "Ubuntu-Light")
        .cloned()
        .collect();
    fallbacks.push(ICONS.to_owned());
    let family = |first: &[&str]| -> Vec<String> {
        first
            .iter()
            .map(|name| (*name).to_owned())
            // Ubuntu has symbols (√ and others) that some of the fonts lack.
            .chain(["Inter".to_owned(), "Ubuntu-Light".to_owned()])
            .chain(cjk.map(|_| CJK.to_owned()))
            .chain(fallbacks.iter().cloned())
            .fold(Vec::new(), |mut list, name| {
                if !list.contains(&name) {
                    list.push(name);
                }
                list
            })
    };
    let families = [
        (FontFamily::Proportional, family(&["Inter"])),
        (FontFamily::Monospace, family(&["Geist Mono", "Hack"])),
        (FontFamily::Name(MEDIUM.into()), family(&["Inter Medium"])),
        (FontFamily::Name(SERIF.into()), family(&["Source Serif 4"])),
        (FontFamily::Name(CONDENSED.into()), family(&["Oswald"])),
        (FontFamily::Name(SCRIPT.into()), family(&["Dancing Script"])),
        (FontFamily::Name(HANDWRITING.into()), family(&["Caveat"])),
        (FontFamily::Name(ICONS.into()), vec![ICONS.to_owned()]),
    ];
    for (name, list) in families {
        fonts.families.insert(name, list);
    }
    for typeface in Typeface::ALL {
        let (_, name) = typeface_font(typeface);
        let bold = format!("{name} Bold");
        let mut list = vec![bold];
        if typeface == Typeface::Monospace {
            list.push("Hack".to_owned());
        }
        let list: Vec<&str> = list.iter().map(String::as_str).collect();
        fonts.families.insert(bold_family(typeface), family(&list));
    }
    fonts
}

/// Text in the interface font, a little heavier than the body: headers, the
/// active tab, badges.
pub fn medium(text: impl Into<String>) -> RichText {
    RichText::new(text).family(FontFamily::Name(MEDIUM.into()))
}

// -------------------------------------------------------------- style

fn corner(radius: u8) -> CornerRadius {
    CornerRadius::same(radius)
}

/// The visuals of a theme, from egui's own (for what is not set here, like the
/// shadows) with the palette's colours.
pub fn visuals(theme: Theme) -> Visuals {
    let p = Palette::of(theme);
    let mut v = match theme {
        Theme::Dark => Visuals::dark(),
        Theme::Light => Visuals::light(),
    };
    v.panel_fill = p.panel;
    v.window_fill = p.panel;
    v.window_stroke = Stroke::new(1.0, p.border);
    v.window_corner_radius = corner(PANEL_RADIUS);
    v.menu_corner_radius = corner(PANEL_RADIUS);
    v.faint_bg_color = p.stripe;
    v.extreme_bg_color = p.field;
    v.text_edit_bg_color = Some(p.field);
    v.code_bg_color = p.code_bg;
    v.hyperlink_color = p.accent;
    v.warn_fg_color = p.warning;
    v.error_fg_color = p.error;
    v.weak_text_color = Some(p.text_weak);
    v.selection.bg_fill = p.selection;
    v.selection.stroke = Stroke::new(1.0, p.selection_outline);

    let w = &mut v.widgets;
    let radius = corner(CONTROL_RADIUS);
    w.noninteractive.bg_fill = p.panel;
    w.noninteractive.weak_bg_fill = p.panel;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    w.noninteractive.corner_radius = radius;

    w.inactive.bg_fill = p.control;
    w.inactive.weak_bg_fill = p.control;
    // Text fields draw this outline; without it a field is lost on a bar.
    w.inactive.bg_stroke = Stroke::new(1.0, p.border);
    w.inactive.fg_stroke = Stroke::new(1.0, p.text);
    w.inactive.corner_radius = radius;

    w.hovered.bg_fill = p.hover;
    w.hovered.weak_bg_fill = p.hover;
    w.hovered.bg_stroke = Stroke::new(1.0, p.border);
    w.hovered.fg_stroke = Stroke::new(1.5, p.text);
    w.hovered.corner_radius = radius;
    w.hovered.expansion = 0.0;

    w.active.bg_fill = p.selection;
    w.active.weak_bg_fill = p.selection;
    w.active.bg_stroke = Stroke::new(1.0, p.accent);
    w.active.fg_stroke = Stroke::new(2.0, p.text);
    w.active.corner_radius = radius;
    w.active.expansion = 0.0;

    w.open.bg_fill = p.hover;
    w.open.weak_bg_fill = p.hover;
    w.open.bg_stroke = Stroke::new(1.0, p.border);
    w.open.fg_stroke = Stroke::new(1.0, p.text);
    w.open.corner_radius = radius;
    v
}

/// The whole style of a theme: its visuals, the spacing and the text sizes.
pub fn style(theme: Theme) -> Style {
    let mut style = Style {
        visuals: visuals(theme),
        ..Style::default()
    };
    style.text_styles = text_styles(FontFamily::Proportional, 1.0);
    let s = &mut style.spacing;
    s.item_spacing = egui::vec2(8.0, 5.0);
    s.button_padding = egui::vec2(8.0, 3.0);
    s.interact_size.y = 22.0;
    s.window_margin = Margin::same(10);
    s.menu_margin = Margin::same(6);
    s.indent = 16.0;
    // Scroll bars that are always there and take their own room, as in a
    // desktop utility: egui's floating ones are hidden until the pointer is
    // near, so a form or table wider than the window did not look scrollable.
    s.scroll = egui::style::ScrollStyle::solid();
    style
}

fn installed_id() -> egui::Id {
    egui::Id::unique("gossia style installed")
}

fn ready_id() -> egui::Id {
    egui::Id::unique("gossia style ready")
}

/// Installs the fonts and the styles of both themes on the first frame. Called
/// at the start of each frame.
pub fn prepare(ctx: &egui::Context) {
    let installed = ctx
        .data(|d| d.get_temp::<bool>(installed_id()))
        .unwrap_or(false);
    if installed {
        ctx.data_mut(|d| d.insert_temp(ready_id(), true));
        return;
    }
    ctx.set_fonts(fonts());
    for theme in [Theme::Dark, Theme::Light] {
        ctx.set_style_of(theme, style(theme));
    }
    // A symbol that no font has is drawn as a box, not a reason to stop.
    ctx.set_missing_glyph_policy(egui::MissingGlyphPolicy::Tofu);
    ctx.data_mut(|d| {
        d.insert_temp(installed_id(), true);
        d.insert_temp(ready_id(), false);
    });
    ctx.request_repaint();
}

/// Whether the bundled fonts are in use in this frame.
pub fn ready(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(ready_id()))
        .unwrap_or(false)
}

// ---------------------------------------------------- drawing helpers

/// A frame for a bar of the window (the menu, the formula bar, the tabs, the
/// status bar), in the chrome colour.
pub fn bar_frame(ui: &egui::Ui) -> egui::Frame {
    egui::Frame::side_top_panel(ui.style())
        .fill(Palette::of_ui(ui).chrome)
        .inner_margin(Margin::symmetric(8, 4))
}

/// A type badge: the text on a rounded fill, cut short with an ellipsis when
/// the column is narrow. Clicking it is reported, as for a label.
pub fn badge_label(ui: &mut egui::Ui, text: &str, colours: Badge) -> Response {
    // The badge follows the font of its `Ui` (the data font in the grid):
    // its typeface, medium for Inter, and its size relative to the default.
    let body = ui.style().text_styles[&TextStyle::Body].clone();
    let scale = body.size / BODY_SIZE;
    let family = if body.family == FontFamily::Proportional {
        FontFamily::Name(MEDIUM.into())
    } else {
        body.family
    };
    let padding = BADGE_PADDING * scale;
    let room = (ui.available_width() - 2.0 * padding).max(0.0);
    let galley = egui::WidgetText::from(
        RichText::new(text)
            .font(FontId::new(BADGE_SIZE * scale, family))
            .color(colours.text),
    )
    .into_galley(
        ui,
        Some(TextWrapMode::Truncate),
        room,
        egui::FontSelection::Default,
    );
    let size = egui::vec2(
        galley.size().x + 2.0 * padding,
        (BADGE_HEIGHT * scale).max(galley.size().y + 2.0),
    );
    let (rect, response) = ui.allocate_exact_size(size, Sense::CLICK);
    if ui.is_rect_visible(rect) {
        ui.painter()
            .rect_filled(rect, corner(CONTROL_RADIUS), colours.fill);
        let at = rect.center() - galley.size() / 2.0;
        ui.painter().galley(at, galley, colours.text);
    }
    let text = text.to_owned();
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, &text));
    response
}

/// The marker of a cell that is waiting to be calculated: a small dot.
pub fn pending_marker(ui: &mut egui::Ui) -> Response {
    let side = MARKER_DIAMETER + 4.0;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(side, side), Sense::hover());
    ui.painter().circle_filled(
        rect.center(),
        MARKER_DIAMETER / 2.0,
        Palette::of_ui(ui).pending,
    );
    response
}

/// The theme for what the system draws around a window: its title bar.
pub fn window_theme(choice: crate::prefs::ThemeChoice) -> egui::SystemTheme {
    use crate::prefs::ThemeChoice;
    match choice {
        ThemeChoice::System => egui::SystemTheme::SystemDefault,
        ThemeChoice::Light => egui::SystemTheme::Light,
        ThemeChoice::Dark => egui::SystemTheme::Dark,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reports are measured by `gossia-layout`, without egui, and drawn by
    /// egui on screen, line by line; the two must agree on the width of a
    /// line and the height of a row, in every typeface (DESIGN.md, The report
    /// designer).
    #[test]
    fn reports_measure_text_as_egui_does() {
        use gossia_layout::{Fonts, TextStyle};
        let ctx = egui::Context::default();
        ctx.set_fonts(fonts());
        let mut out = ctx.run_ui(egui::RawInput::default(), |_| {});
        out.textures_delta.clear();
        let texts = [
            "Hello, world",
            "AVA WAVE Tô",
            "Quarterly revenue by org unit: 1,234,567.89",
            "Grüße aus Köln — λόγος",
            "√2 ≈ 1.414",
        ];
        let layout = Fonts::shared();
        ctx.fonts_mut(|f| {
            for typeface in Typeface::ALL {
                for size in [10.0, 14.0, 22.0] {
                    let font = FontId::new(size, family(typeface));
                    let style = TextStyle::new(typeface, size);
                    assert_eq!(
                        layout.metrics(&style).row_height,
                        f.row_height(&font),
                        "row height of {typeface:?} at {size}"
                    );
                    for text in texts {
                        let ours = layout.width(text, &style);
                        let egui = f
                            .layout_no_wrap(text.to_owned(), font.clone(), Color32::WHITE)
                            .size()
                            .x;
                        assert!(
                            (ours - egui).abs() <= 0.25,
                            "{typeface:?} at {size}: {text:?} is {ours} here and {egui} in egui"
                        );
                    }
                }
            }
        });
    }

    /// An icon is drawn from the icon font in every family, as the file
    /// dialog draws its icons in the text font. (Inter has glyphs of its own
    /// in the private use area, where the icons are.)
    #[test]
    fn an_icon_is_drawn_from_the_icon_font_in_every_family() {
        let ctx = egui::Context::default();
        ctx.set_fonts(fonts());
        let mut out = ctx.run_ui(egui::RawInput::default(), |_| {});
        out.textures_delta.clear();
        let icons = [
            crate::icons::FOLDER,
            crate::icons::FOLDER_OPEN,
            crate::icons::HOUSE,
            crate::icons::DESKTOP,
            crate::icons::FILE,
            crate::icons::FILE_TEXT,
            crate::icons::DOWNLOAD,
            crate::icons::MUSIC,
            crate::icons::IMAGE,
            crate::icons::FILM,
            crate::icons::PIN,
            crate::icons::PENCIL,
            crate::icons::RELOAD,
            crate::icons::DRIVE,
            crate::icons::WARNING,
        ];
        let icon_font = FontId::new(16.0, FontFamily::Name(ICONS.into()));
        ctx.fonts_mut(|f| {
            // The icon font has no letters, so it can come first.
            assert!(!f.has_glyph(&icon_font, 'A'));
            assert!(!f.has_glyph(&icon_font, '1'));
            for family in fonts().families.keys() {
                let font = FontId::new(16.0, family.clone());
                for icon in icons {
                    let c = icon.chars().next().unwrap();
                    assert_eq!(
                        f.glyph_width(&font, c),
                        f.glyph_width(&icon_font, c),
                        "{family:?} draws {:X} from another font",
                        c as u32
                    );
                }
            }
        });
    }

    #[test]
    fn taking_out_the_private_use_area_keeps_every_other_character() {
        use skrifa::MetadataProvider;
        let original = gossia_layout::fonts::INTER;
        let cleaned = without_private_use(original);
        let map = |bytes: &[u8]| -> Vec<(u32, u32)> {
            skrifa::FontRef::new(bytes)
                .unwrap()
                .charmap()
                .mappings()
                .map(|(c, g)| (c, g.to_u32()))
                .collect()
        };
        let before = map(original);
        let after = map(&cleaned);
        let kept: Vec<(u32, u32)> = before
            .iter()
            .copied()
            .filter(|(c, _)| !PRIVATE_USE.contains(c))
            .collect();
        assert!(
            before.len() > kept.len(),
            "Inter has private use characters"
        );
        assert_eq!(after, kept);
        // The other tables are as they were.
        let font = skrifa::FontRef::new(&cleaned).unwrap();
        assert_eq!(
            font.table_data(skrifa::Tag::new(b"glyf")).map(|d| d.len()),
            skrifa::FontRef::new(original)
                .unwrap()
                .table_data(skrifa::Tag::new(b"glyf"))
                .map(|d| d.len())
        );
    }

    #[test]
    fn every_family_starts_with_a_bundled_font_and_has_the_icons() {
        let fonts = fonts();
        for (family, list) in &fonts.families {
            for name in list {
                assert!(
                    fonts.font_data.contains_key(name),
                    "{family:?}: no font {name}"
                );
            }
            assert!(list.contains(&ICONS.to_owned()), "{family:?}");
        }
        for typeface in Typeface::ALL {
            assert!(
                fonts.families.contains_key(&family(typeface)),
                "{typeface:?}"
            );
            // Bold is the same file at the bold weight, first in its family.
            let bold = &fonts.families[&bold_family(typeface)];
            assert!(bold[0].ends_with(" Bold"), "{typeface:?}: {bold:?}");
        }
    }

    #[test]
    fn bold_text_measures_as_reports_measure_it() {
        let ctx = egui::Context::default();
        ctx.set_fonts(fonts());
        let mut out = ctx.run_ui(egui::RawInput::default(), |_| {});
        out.textures_delta.clear();
        let layout = gossia_layout::Fonts::shared();
        ctx.fonts_mut(|f| {
            for typeface in Typeface::ALL {
                let text = "Quarterly revenue: 1,234.50";
                let mut width = |family| {
                    f.layout_no_wrap(text.to_owned(), FontId::new(14.0, family), Color32::WHITE)
                        .size()
                        .x
                };
                let bold = width(bold_family(typeface));
                // Wider, except in a monospaced font, whose letters keep
                // their width at every weight.
                let regular = width(family(typeface));
                if typeface == Typeface::Monospace {
                    assert!((bold - regular).abs() < 0.01, "{typeface:?}");
                } else {
                    assert!(bold > regular, "{typeface:?}");
                }
                let ours =
                    layout.width(text, &gossia_layout::TextStyle::new(typeface, 14.0).bold());
                // Within the tolerance of the regular weight (see above).
                assert!(
                    (ours - bold).abs() <= 0.25,
                    "{typeface:?}: {ours} and {bold}"
                );
            }
        });
        let mut style = Style::default();
        use_font_in(&mut style, None, None, true);
        assert_eq!(
            TextStyle::Monospace.resolve(&style).family,
            bold_family(Typeface::Monospace)
        );
        assert_eq!(
            TextStyle::Body.resolve(&style).family,
            bold_family(Typeface::Sans)
        );
    }

    #[test]
    fn the_bundled_fonts_have_a_weight_axis() {
        let fonts = fonts();
        for name in [
            "Inter",
            "Inter Medium",
            "Geist Mono",
            "Source Serif 4",
            "Oswald",
            "Dancing Script",
            "Caveat",
        ] {
            let axes = fonts.font_data[name].variation_axes();
            assert!(
                axes.iter().any(|a| a.tag == "wght"),
                "{name} has no weight axis"
            );
        }
    }

    #[test]
    fn a_font_sets_every_text_style_and_scales_from_the_defaults() {
        let mut style = style(Theme::Dark);
        use_font(&mut style, Some(Typeface::Serif), Some(28));
        let body = &style.text_styles[&TextStyle::Body];
        assert_eq!(body.size, 28.0);
        assert_eq!(body.family, family(Typeface::Serif));
        assert_eq!(style.text_styles[&TextStyle::Small].size, SMALL_SIZE * 2.0);
        let code = &style.text_styles[&TextStyle::Monospace];
        assert_eq!(code.family, family(Typeface::Serif), "code takes the font");
        assert_eq!(code.size, MONOSPACE_SIZE * 2.0);
        // Under Sans, code stays in the monospace font.
        use_font(&mut style, Some(Typeface::Sans), None);
        assert_eq!(
            style.text_styles[&TextStyle::Monospace].family,
            FontFamily::Monospace
        );
        // Applied again (a control in a group with a font), it does not
        // compound.
        use_font(&mut style, None, Some(14));
        assert_eq!(style.text_styles[&TextStyle::Body].size, 14.0);
        assert_eq!(
            style.text_styles[&TextStyle::Body].family,
            FontFamily::Proportional
        );
    }

    #[test]
    fn a_badge_follows_the_font_of_its_ui() {
        let ctx = egui::Context::default();
        let badge = |typeface, size| {
            let mut rect = egui::Rect::NOTHING;
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                prepare(ui.ctx());
                use_font(ui.style_mut(), typeface, size);
                rect = badge_label(ui, "string", DARK.string).rect;
            });
            out.textures_delta.clear();
            rect
        };
        // The first frame installs the fonts.
        let _ = badge(None, None);
        let normal = badge(None, None);
        let large = badge(Some(Typeface::Serif), Some(28));
        assert_eq!(normal.height(), BADGE_HEIGHT);
        assert!(large.height() >= 2.0 * BADGE_HEIGHT - 0.5, "{large:?}");
        assert!(large.width() > normal.width() * 1.5, "{large:?} {normal:?}");
    }

    #[test]
    fn the_computed_tint_is_not_the_selection() {
        for p in [&DARK, &LIGHT] {
            assert_ne!(p.computed, p.selection);
            assert!(p.computed.a() < 255, "the tint must be translucent");
        }
    }
}
