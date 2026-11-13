//! Touch and small screens (DESIGN.md, The web version on GitHub Pages,
//! 17d): larger targets while the window is used with a finger, and on a
//! narrow window a single menu button and side panels that fold away. There
//! is no separate layout; the designers and the code editor are not
//! reworked.

use crate::app::GossiaApp;
use gossia_i18n::tr;

use crate::icons;

/// Below this width (in points) the window is narrow: a phone, or a tablet
/// upright.
pub const NARROW: f32 = 700.0;

/// The width of a folded side panel's strip.
const FOLDED: f32 = 30.0;
const FOLDED_TOUCH: f32 = 44.0;

/// The most of the window an unfolded side panel takes on a narrow window.
const UNFOLDED_SHARE: f32 = 0.85;

/// Whether the window is narrow.
pub fn narrow(ctx: &egui::Context) -> bool {
    ctx.content_rect().width() < NARROW
}

impl GossiaApp {
    /// Follows the way the window is used: a touch turns the larger targets
    /// on, a press of a mouse button off again. At the start a host may say
    /// that its pointer is a finger (a phone's browser). Called each frame,
    /// after the style is installed.
    pub(crate) fn follow_touch(&mut self, ctx: &egui::Context) {
        if self.touch_applied.is_none() {
            self.touch = crate::host::current().is_some_and(|h| h.coarse_pointer());
        }
        let (touched, pressed) = ctx.input(|i| {
            let touched = i
                .events
                .iter()
                .any(|e| matches!(e, egui::Event::Touch { .. }));
            let pressed = i
                .events
                .iter()
                .any(|e| matches!(e, egui::Event::PointerButton { pressed: true, .. }));
            (touched, pressed)
        });
        // A touch also presses the pointer's button, in the same frame.
        if touched {
            self.touch = true;
        } else if pressed {
            self.touch = false;
        }
        if self.touch_applied != Some(self.touch) {
            crate::style::set_touch(ctx, self.touch);
            self.touch_applied = Some(self.touch);
        }
    }

    /// The menu bar's menus: in one button on a narrow window, where they
    /// would not fit side by side.
    pub(crate) fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            if narrow(ui.ctx()) {
                let label = if icons::ready(ui.ctx()) {
                    icons::MENU.to_owned()
                } else {
                    tr!("Menu").to_owned()
                };
                // The menu bar's buttons are small; the one button is not,
                // with a finger.
                let label = if self.touch {
                    egui::RichText::new(label).size(22.0)
                } else {
                    egui::RichText::new(label)
                };
                let response = ui.menu_button(label, |ui| self.menus(ui)).response;
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::Role::Button, true, tr!("Menu"))
                });
            } else {
                self.menus(ui);
            }
        });
    }

    /// A panel on the right of the window. On a narrow window it is folded
    /// to a strip with a button, which unfolds it over most of the window;
    /// the side panels share whether they are unfolded.
    pub(crate) fn side_panel(
        &mut self,
        ui: &mut egui::Ui,
        panel: egui::Panel,
        folded_id: &'static str,
        add_contents: impl FnOnce(&mut Self, &mut egui::Ui),
    ) {
        if !narrow(ui.ctx()) {
            panel.show(ui, |ui| add_contents(self, ui));
            return;
        }
        let width = ui.ctx().content_rect().width();
        let strip = if self.touch { FOLDED_TOUCH } else { FOLDED };
        let folded = egui::Panel::right(folded_id)
            .resizable(false)
            .exact_size(strip);
        // A set share of the window: a finger does not drag its edge.
        let unfolded = panel.resizable(false).exact_size(width * UNFOLDED_SHARE);
        let mut open = self.side_unfolded;
        let mut toggled = false;
        egui::Panel::show_switched(ui, &mut open, folded, unfolded, |ui, expanded| {
            let (icon, letter, name) = if expanded {
                (icons::CARET_RIGHT, ">", tr!("Fold the side panel"))
            } else {
                (icons::CARET_LEFT, "<", tr!("Unfold the side panel"))
            };
            if icons::button(ui, true, icon, letter, name).clicked() {
                toggled = true;
            }
            if expanded {
                add_contents(self, ui);
            }
        });
        self.side_unfolded = if toggled { !self.side_unfolded } else { open };
    }
}
