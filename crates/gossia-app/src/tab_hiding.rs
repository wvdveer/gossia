//! Hidden tabs (DESIGN.md, "Managing tabs", *Hide tab*): a tab can be taken
//! off the tab bar from its menu and brought back by name from *View >
//! Unhide tab*. It is view state, kept in `view.json`: not an edit, not
//! undoable, and not part of the trust hash. The tab is still there for
//! formulas, the code editor and the control channel; going to it (from the
//! Issues panel, say) shows it again.

use gossia_i18n::tr;

use crate::app::GossiaApp;

impl GossiaApp {
    /// The tabs on the tab bar, in their order.
    pub fn visible_tabs(&self) -> Vec<String> {
        self.calc()
            .tab_names()
            .into_iter()
            .filter(|n| !self.hidden_tabs.contains(n))
            .collect()
    }

    /// The hidden tabs, in the order of the tabs.
    pub fn hidden_tab_names(&self) -> Vec<String> {
        self.calc()
            .tab_names()
            .into_iter()
            .filter(|n| self.hidden_tabs.contains(n))
            .collect()
    }

    /// Whether a tab can be hidden: one tab stays on the bar.
    pub fn can_hide_tab(&self, name: &str) -> bool {
        let visible = self.visible_tabs();
        visible.len() > 1 && visible.iter().any(|n| n == name)
    }

    /// Hides a tab. When it is the one shown, the tab after it on the bar is
    /// shown, or the one before for the last.
    pub fn hide_tab(&mut self, name: &str) {
        if !self.can_hide_tab(name) {
            return;
        }
        let visible = self.visible_tabs();
        let at = visible.iter().position(|n| n == name).unwrap_or(0);
        self.hidden_tabs.insert(name.to_owned());
        if self.sheet == name {
            let next = visible
                .get(at + 1)
                .or_else(|| at.checked_sub(1).and_then(|i| visible.get(i)))
                .cloned();
            if let Some(next) = next {
                self.switch_to(&next);
            }
        }
    }

    /// Puts a hidden tab back on the bar and shows it.
    pub fn unhide_tab(&mut self, name: &str) {
        self.hidden_tabs.remove(name);
        self.switch_to(name);
    }

    /// Keeps the hidden tabs to tabs that are there, with one on the bar, and
    /// the tab shown on the bar. After a file is read.
    pub(crate) fn tidy_hidden_tabs(&mut self) {
        let names = self.calc().tab_names();
        self.hidden_tabs.retain(|n| names.contains(n));
        if names.iter().all(|n| self.hidden_tabs.contains(n)) {
            self.hidden_tabs.clear();
        }
        if self.hidden_tabs.contains(&self.sheet)
            && let Some(first) = self.visible_tabs().first().cloned()
        {
            self.show_tab(&first);
        }
    }

    /// *Hide tab* on a tab's menu.
    pub(crate) fn hide_tab_item(&mut self, ui: &mut egui::Ui, tab: &str) {
        let button = ui
            .add_enabled(self.can_hide_tab(tab), egui::Button::new(tr!("Hide tab")))
            .on_hover_text(tr!("View > Unhide tab shows it again"))
            .on_disabled_hover_text(tr!("One tab stays on the tab bar"));
        if button.clicked() {
            self.hide_tab(tab);
            ui.close();
        }
    }

    /// *View > Unhide tab*: the hidden tabs by name.
    pub(crate) fn unhide_menu(&mut self, ui: &mut egui::Ui) {
        let hidden = self.hidden_tab_names();
        ui.add_enabled_ui(!hidden.is_empty(), |ui| {
            ui.menu_button(tr!("Unhide tab"), |ui| {
                for name in hidden {
                    if ui.button(&name).clicked() {
                        self.unhide_tab(&name);
                        ui.close();
                    }
                }
            });
        });
    }
}
