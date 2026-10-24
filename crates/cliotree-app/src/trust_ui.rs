//! The trust banner and the words of untrusted mode (DESIGN.md, "Trust model").
//!
//! Deciding whether a file is trusted belongs to the layer that opens files,
//! with the record of past decisions. This is what the window does with the
//! answer: it shows that nothing is being run, why, and lets the user trust the
//! version in front of them.

use cliotree_calc::{Change, ChangeKind};
use egui::{Color32, RichText};

use crate::app::ClioApp;

/// What the banner shows.
#[derive(Debug, Default)]
pub struct TrustBanner {
    /// What changed since the workbook was trusted, when it was changed outside
    /// the application. Empty for a file that was never trusted.
    pub changes: Vec<Change>,
    /// The user chose to continue read-only, so the banner is put away.
    pub dismissed: bool,
}

/// One line of the list of changes.
pub fn change_text(change: &Change) -> String {
    let what = match change.kind {
        ChangeKind::Added => "added",
        ChangeKind::Edited => "changed",
        ChangeKind::Removed => "removed",
    };
    format!("{}: {what}", change.item)
}

/// The sentence that says what trusting allows.
pub const WARNING: &str = "This workbook has not been trusted, so its formulas and modules are \
     not run. Trusting it allows it to run code.";

impl ClioApp {
    /// Trusts the workbook as it is now.
    pub fn trust_workbook(&mut self) {
        self.session.calc_mut().set_trusted(true);
        self.trust = TrustBanner::default();
        self.notice = None;
        self.session.tick();
    }

    /// Stops running the workbook's code. `changes` is what the file changed
    /// since it was trusted, if it was.
    pub fn withhold_trust(&mut self, changes: Vec<Change>) {
        self.session.calc_mut().set_trusted(false);
        self.trust = TrustBanner {
            changes,
            dismissed: false,
        };
    }

    /// Puts the banner away and carries on without running anything.
    pub fn continue_read_only(&mut self) {
        self.trust.dismissed = true;
    }

    /// Whether the banner is on the screen.
    pub fn banner_shown(&self) -> bool {
        !self.calc().is_trusted() && !self.trust.dismissed
    }

    /// What to tell the user after they have entered a formula, if it will not
    /// be calculated. Brings the banner back, which is where trust is asked.
    pub(crate) fn note_unevaluated_formula(&mut self, text: &str) {
        if self.calc().is_trusted() || !text.trim_start().starts_with('=') {
            return;
        }
        self.trust.dismissed = false;
        self.notice = Some(
            "The formula is stored but not calculated: this workbook is not trusted.".to_owned(),
        );
    }

    pub(crate) fn trust_banner(&mut self, ui: &mut egui::Ui) {
        let fill = if ui.visuals().dark_mode {
            Color32::from_rgb(80, 62, 20)
        } else {
            Color32::from_rgb(255, 240, 200)
        };
        let mut trust = false;
        let mut dismiss = false;
        egui::Frame::new()
            .fill(fill)
            .inner_margin(6.0)
            .show(ui, |ui| {
                ui.label(RichText::new(WARNING).strong());
                if !self.trust.changes.is_empty() {
                    ui.label("It was trusted before, but the file has changed since:");
                    egui::ScrollArea::vertical()
                        .max_height(90.0)
                        .id_salt("trust_changes")
                        .show(ui, |ui| {
                            for change in &self.trust.changes {
                                ui.label(change_text(change));
                            }
                        });
                }
                ui.horizontal(|ui| {
                    trust = ui.button("Trust this version").clicked();
                    dismiss = ui.button("Continue read-only").clicked();
                });
            });
        if trust {
            self.trust_workbook();
        } else if dismiss {
            self.continue_read_only();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_change_is_worded_by_what_happened_to_the_item() {
        let change = |kind| Change {
            kind,
            item: "formula S!a".to_owned(),
        };
        assert_eq!(
            change_text(&change(ChangeKind::Edited)),
            "formula S!a: changed"
        );
        assert_eq!(
            change_text(&change(ChangeKind::Added)),
            "formula S!a: added"
        );
        assert_eq!(
            change_text(&change(ChangeKind::Removed)),
            "formula S!a: removed"
        );
    }
}
