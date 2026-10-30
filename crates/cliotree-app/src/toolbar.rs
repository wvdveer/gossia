//! The toolbar under the menu bar (DESIGN.md, "Look and feel", "Toolbar"):
//! the commands used most, as icon buttons, and the choice of a light or a
//! dark window. Each button does what its menu command does and is enabled
//! when that command is.

use cliotree_core::{Path, Segment};

use crate::app::ClioApp;
use crate::icons;
use crate::prefs::ThemeChoice;

pub const UNDO: &str = "\u{E014}";
pub const REDO: &str = "\u{E016}";
pub const SUN: &str = "\u{E472}";
pub const MOON: &str = "\u{E330}";
pub const CHECK_CIRCLE: &str = "\u{E184}";
pub const SIDE_PANEL: &str = "\u{E462}";
pub const COPY: &str = "\u{E1CA}";

impl ClioApp {
    /// Where *Add an item* adds one for the selection: the array and the
    /// position. An array selected gets an item at its end; an item gets one
    /// after it. `None` when nothing selected is in an array, or outside the
    /// data tree.
    pub fn add_item_target(&self) -> Option<(Path, Option<usize>)> {
        if !self.in_tree() || self.editing.is_some() {
            return None;
        }
        let selection = self.selection.as_ref()?;
        let spec = self.calc().spec(&self.sheet)?;
        let here = spec.resolve(selection)?;
        if matches!(spec.get(here.ty).kind, cliotree_core::Kind::Array { .. }) {
            return Some((selection.clone(), None));
        }
        // The nearest item the selection is in.
        let segments = selection.segments();
        let at = segments
            .iter()
            .rposition(|s| matches!(s, Segment::Index(_)))?;
        let Segment::Index(i) = segments[at] else {
            return None;
        };
        Some((Path::from(segments[..at].to_vec()), Some(i + 1)))
    }

    /// The bar of icon buttons.
    pub(crate) fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let idle = self.editing.is_none();
            if icons::button(ui, true, icons::FILE_PLUS, "N", "New workbook").clicked() {
                self.begin_new();
            }
            if icons::button(ui, true, icons::FOLDER_OPEN, "O", "Open workbook").clicked() {
                self.begin_open();
            }
            if icons::button(ui, idle, icons::FLOPPY, "S", "Save workbook").clicked() {
                self.begin_save();
            }
            ui.separator();
            let can_change = idle && self.session.running_procedure().is_none();
            let undo = self.calc().undo_label().map(str::to_owned);
            let redo = self.calc().redo_label().map(str::to_owned);
            let tip = |what: &str, label: &Option<String>| match label {
                Some(label) => format!("{what} {label}"),
                None => format!("Nothing to {}", what.to_lowercase()),
            };
            let response = icons::button(ui, undo.is_some() && can_change, UNDO, "<", "Undo")
                .on_hover_text(tip("Undo", &undo));
            if response.clicked() {
                self.undo();
            }
            let response = icons::button(ui, redo.is_some() && can_change, REDO, ">", "Redo")
                .on_hover_text(tip("Redo", &redo));
            if response.clicked() {
                self.redo();
            }
            ui.separator();
            let target = self.add_item_target();
            if icons::button(ui, target.is_some(), icons::PLUS, "+", "Add an item").clicked()
                && let Some((array, at)) = target
            {
                self.insert_item(&array, at);
            }
            let fx = crate::style::medium("fx").italics();
            let response = ui
                .add_enabled(self.in_tree(), egui::Button::new(fx).small())
                .on_hover_text("Insert a function");
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::Button, self.in_tree(), "Insert a function")
            });
            if response.clicked() {
                self.open_insert_function();
            }
            ui.separator();
            self.refresh_summary();
            let problems = self.summary.errors + self.summary.invalid;
            let (icon, tip) = if problems == 0 {
                (CHECK_CIRCLE, "No errors, no invalid data".to_owned())
            } else {
                (
                    icons::WARNING,
                    format!(
                        "{} errors, {} invalid",
                        self.summary.errors, self.summary.invalid
                    ),
                )
            };
            let response =
                toggle(ui, self.show_issues, icon, "I", "Show issues").on_hover_text(tip);
            if response.clicked() {
                self.show_issues = !self.show_issues;
            }
            let response = toggle(
                ui,
                self.show_inspector,
                SIDE_PANEL,
                "i",
                "Show node inspector",
            );
            if response.clicked() {
                self.show_inspector = !self.show_inspector;
            }
            // The theme, at the far end.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let theme = self.user.prefs.appearance.theme;
                for (choice, icon, letter, name) in [
                    (ThemeChoice::Dark, MOON, "D", "Dark theme"),
                    (ThemeChoice::Light, SUN, "L", "Light theme"),
                ] {
                    let on = theme == choice;
                    let tip = if on {
                        format!("{name} (click again to follow the system)")
                    } else {
                        name.to_owned()
                    };
                    if toggle(ui, on, icon, letter, name)
                        .on_hover_text(tip)
                        .clicked()
                    {
                        let next = if on { ThemeChoice::System } else { choice };
                        self.set_theme(next);
                    }
                }
            });
        });
    }

    /// Changes the theme in the preferences, as the Options dialog does.
    pub fn set_theme(&mut self, theme: ThemeChoice) {
        if let Err(why) = self.user.change_prefs(|p| p.appearance.theme = theme) {
            self.notice = Some(why);
        }
    }
}

/// An icon button that stays pressed while `on`.
fn toggle(ui: &mut egui::Ui, on: bool, icon: &str, letter: &str, name: &str) -> egui::Response {
    let text = if icons::ready(ui.ctx()) {
        egui::RichText::new(icon).family(egui::FontFamily::Name(crate::style::ICONS.into()))
    } else {
        egui::RichText::new(letter)
    };
    let response = ui
        .add(egui::Button::new(text).small().selected(on))
        .on_hover_text(name);
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, name));
    response
}
