//! Icons: the Phosphor Icons font (MIT, see `assets/PHOSPHOR-NOTICE.txt`),
//! which has the arrows and the cross the bundled text font lacks.
//!
//! The font is added to egui when the first frame is drawn and takes effect
//! on the next one, so [`ready`] says whether the icons can be drawn yet; an
//! icon button draws a letter until then.

use egui::{FontFamily, RichText};

pub const ARROW_UP: &str = "\u{E08E}";
pub const ARROW_DOWN: &str = "\u{E03E}";
pub const CARET_LEFT: &str = "\u{E138}";
pub const CARET_RIGHT: &str = "\u{E13A}";
pub const X: &str = "\u{E4F6}";

// What the file browser draws (its own symbols are not in the bundled font).
pub const HOUSE: &str = "\u{E2C2}";
pub const FOLDER: &str = "\u{E24A}";
pub const FOLDER_OPEN: &str = "\u{E256}";
pub const FILE: &str = "\u{E230}";
pub const FILE_TEXT: &str = "\u{E23A}";
pub const DESKTOP: &str = "\u{E560}";
pub const DOWNLOAD: &str = "\u{E20C}";
pub const MUSIC: &str = "\u{E33C}";
pub const IMAGE: &str = "\u{E2CA}";
pub const FILM: &str = "\u{E792}";
pub const PIN: &str = "\u{E3E2}";
pub const PENCIL: &str = "\u{E3B4}";
pub const RELOAD: &str = "\u{E036}";
pub const ARROW_UP_RIGHT: &str = "\u{E092}";
pub const CHECK_SQUARE: &str = "\u{E186}";
pub const PROHIBIT: &str = "\u{E3DE}";
pub const DRIVE: &str = "\u{E29E}";
pub const DRIVES: &str = "\u{E2A0}";
pub const WARNING: &str = "\u{E4E0}";
pub const CARET_UP: &str = "\u{E13C}";
pub const MENU: &str = "\u{E2F0}";
pub const SEARCH: &str = "\u{E30C}";
pub const CHECK: &str = "\u{E182}";
pub const PLUS: &str = "\u{E3D4}";
pub const FILE_PLUS: &str = "\u{E236}";
pub const FLOPPY: &str = "\u{E248}";
pub const USB: &str = "\u{E956}";

const FAMILY: &str = "phosphor";

fn state_id() -> egui::Id {
    egui::Id::unique("cliotree icons installed")
}

fn ready_id() -> egui::Id {
    egui::Id::unique("cliotree icons ready")
}

/// Adds the font on the first frame. Called at the start of each frame.
pub fn prepare(ctx: &egui::Context) {
    let installed = ctx
        .data(|d| d.get_temp::<bool>(state_id()))
        .unwrap_or(false);
    if installed {
        ctx.data_mut(|d| d.insert_temp(ready_id(), true));
        return;
    }
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        FAMILY.to_owned(),
        egui::FontData::from_static(include_bytes!("../assets/Phosphor.ttf")).into(),
    );
    fonts
        .families
        .insert(FontFamily::Name(FAMILY.into()), vec![FAMILY.to_owned()]);
    // The icons are in the private use area, so adding the font to the end of
    // the text families lets a label carry one (the file browser's do).
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        if let Some(list) = fonts.families.get_mut(&family) {
            list.push(FAMILY.to_owned());
        }
    }
    ctx.set_fonts(fonts);
    // A symbol that no font has is drawn as a box, not a reason to stop.
    ctx.set_missing_glyph_policy(egui::MissingGlyphPolicy::Tofu);
    ctx.data_mut(|d| {
        d.insert_temp(state_id(), true);
        d.insert_temp(ready_id(), false);
    });
    ctx.request_repaint();
}

/// Whether the icon font is in use in this frame.
pub fn ready(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(ready_id()))
        .unwrap_or(false)
}

/// A small button with an icon. `letter` is what is drawn until the font is
/// there; `name` is what a screen reader (and a test) calls it, and its tip.
pub fn button(
    ui: &mut egui::Ui,
    enabled: bool,
    icon: &str,
    letter: &str,
    name: &'static str,
) -> egui::Response {
    let text = if ready(ui.ctx()) {
        RichText::new(icon).family(FontFamily::Name(FAMILY.into()))
    } else {
        RichText::new(letter)
    };
    let response = ui
        .add_enabled(enabled, egui::Button::new(text).small())
        .on_hover_text(name);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, name));
    response
}
