//! Renaming a tab from the window (DESIGN.md, "Renaming a tab"): the dialog,
//! and moving what the window keeps for a tab to its new name, which an undo
//! or a redo of a rename needs as much as the rename itself.

use gossia_i18n::tr;
use std::collections::HashMap;
use std::hash::Hash;

use gossia_core::Path;

use crate::app::GossiaApp;
use crate::view::{error_color, named};

/// The dialog of *Rename tab...*, while it is open.
#[derive(Debug, Clone)]
pub struct RenameTab {
    /// The tab being renamed.
    pub from: String,
    /// The name being typed.
    pub name: String,
    /// Why the last attempt was refused, when it was.
    pub error: Option<String>,
    /// Whether the name field has been given the keyboard yet.
    focused: bool,
}

impl GossiaApp {
    /// Opens the dialog for `tab`. Typing in progress is entered first, as
    /// switching tab does; if it is refused, nothing opens.
    pub fn begin_rename_tab(&mut self, tab: &str) {
        if self.session.running_procedure().is_some() || !self.commit_editing() {
            return;
        }
        self.rename_tab = Some(RenameTab {
            from: tab.to_owned(),
            name: tab.to_owned(),
            error: None,
            focused: false,
        });
    }

    /// Why `name` cannot be what the dialog's tab is renamed to, or None.
    pub fn rename_problem(&self, from: &str, name: &str) -> Option<String> {
        self.calc().tab_name_problem(name.trim(), Some(from))
    }

    /// Renames a tab, as the dialog does on *Rename*.
    pub fn rename_tab(&mut self, from: &str, to: &str) -> Result<(), String> {
        if self.session.running_procedure().is_some() {
            return Err(tr!("a button procedure is running; wait for it to finish").to_owned());
        }
        if !self.commit_editing() {
            return Err(self.notice.clone().unwrap_or_default());
        }
        self.session
            .calc_mut()
            .rename_sheet(from, to.trim())
            .map_err(|e| e.to_string())?;
        self.follow_tabs();
        self.session.tick();
        Ok(())
    }

    /// Brings what the window keeps for each tab up to date with the tabs
    /// there are: renamed tabs take their state with them, deleted ones lose
    /// it, new ones get their root opened, and if the current tab has gone the
    /// last tab is shown instead. Edits made here call it, and so do undoing
    /// and redoing them.
    pub fn follow_tabs(&mut self) {
        for (from, to) in self.session.calc_mut().take_renames() {
            self.move_tab_state(&from, &to);
        }
        for name in self.session.calc_mut().take_removed() {
            // A tab deleted and put back by an undo starts afresh.
            if self.calc().tab_kind(&name).is_none() {
                self.forget_tab_state(&name);
            }
        }
        let names = self.calc().tab_names();
        for name in &names {
            if !self.calc().is_report(name) && !self.expanded.contains_key(name) {
                self.expanded
                    .entry(name.clone())
                    .or_default()
                    .insert(Path::root());
            }
        }
        if !names.contains(&self.sheet)
            && let Some(last) = names.last()
        {
            self.show_tab(last);
        }
    }

    /// Shows a tab, even when the current one no longer exists.
    pub(crate) fn show_tab(&mut self, name: &str) {
        if self.calc().tab_kind(&self.sheet).is_none() {
            self.editing = None;
            self.sheet = String::new();
        }
        self.switch_to(name);
    }

    /// Forgets what the window kept for a tab that is gone.
    fn forget_tab_state(&mut self, name: &str) {
        if self.editing.as_ref().is_some_and(|e| e.sheet == name) {
            self.editing = None;
        }
        self.expanded.remove(name);
        self.hidden.remove(name);
        self.views.remove(name);
        self.doc.views.remove(name);
        let tables = &mut self.tables;
        tables.tabular.retain(|(sheet, _)| sheet != name);
        tables.views.retain(|(sheet, _), _| sheet != name);
        tables.active.remove(name);
        if tables
            .last_sort
            .as_ref()
            .is_some_and(|(sheet, ..)| sheet == name)
        {
            tables.last_sort = None;
        }
        if tables.column.as_ref().is_some_and(|c| c.sheet == name) {
            tables.column = None;
        }
        tables.layout = None;
        tables.rev += 1;
        self.form.drafts.retain(|(sheet, _), _| sheet != name);
        self.form.steps.retain(|(sheet, _), _| sheet != name);
        self.form.new_keys.retain(|(sheet, _), _| sheet != name);
        if self.form.layout.as_ref().is_some_and(|l| l.sheet == name) {
            self.form.layout = None;
        }
        if self.rename_tab.as_ref().is_some_and(|d| d.from == name) {
            self.rename_tab = None;
        }
        self.tab_dialogs.forget(name);
        self.rows_key = None;
        self.summary.key = None;
        self.raw.key = None;
    }

    fn move_tab_state(&mut self, from: &str, to: &str) {
        let moved = |name: &mut String| {
            if name == from {
                to.clone_into(name);
            }
        };
        moved(&mut self.sheet);
        if let Some(editing) = &mut self.editing {
            moved(&mut editing.sheet);
        }
        // A report's design open in the code editor follows its name.
        for tab in &mut self.code.tabs {
            if tab.item == crate::code::CodeItem::Report(from.to_owned()) {
                tab.item = crate::code::CodeItem::Report(to.to_owned());
            }
        }
        rekey(&mut self.expanded, from, to);
        rekey(&mut self.hidden, from, to);
        rekey(&mut self.views, from, to);
        if let Some(view) = self.doc.views.remove(from) {
            self.doc.views.insert(to.to_owned(), view);
        }
        let tables = &mut self.tables;
        tables.tabular = std::mem::take(&mut tables.tabular)
            .into_iter()
            .map(|(sheet, path)| (renamed(sheet, from, to), path))
            .collect();
        rekey_pair(&mut tables.views, from, to);
        rekey(&mut tables.active, from, to);
        if let Some((sheet, ..)) = &mut tables.last_sort {
            moved(sheet);
        }
        if let Some(column) = &mut tables.column {
            moved(&mut column.sheet);
        }
        tables.layout = None;
        tables.rev += 1;
        rekey_pair(&mut self.form.drafts, from, to);
        rekey_pair(&mut self.form.steps, from, to);
        rekey_pair(&mut self.form.new_keys, from, to);
        if let Some(layout) = &mut self.form.layout {
            moved(&mut layout.sheet);
        }
        if let Some(dialog) = &mut self.rename_tab {
            moved(&mut dialog.from);
        }
        // What was made for the old name is made again.
        self.rows_key = None;
        self.summary.key = None;
        self.raw.key = None;
    }

    /// The dialog of *Rename tab...*, when it is open.
    pub(crate) fn rename_tab_window(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.rename_tab.take() else {
            return;
        };
        let problem = self.rename_problem(&dialog.from, &dialog.name);
        // Nothing in the workbook names a report, so renaming one warns of
        // nothing.
        let mentions = if self.calc().is_report(&dialog.from) {
            gossia_calc::Mentions::default()
        } else {
            self.calc().mentions(&dialog.from)
        };
        let mut open = true;
        let (mut apply, mut cancel) = (false, false);
        egui::Window::new(tr!("Rename tab {from}", from = dialog.from))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(tr!("New name"));
                let response = ui.text_edit_singleline(&mut dialog.name);
                named(&response, tr!("Tab name"));
                if !dialog.focused {
                    response.request_focus();
                    select_all(ui, &response, &dialog.name);
                    dialog.focused = true;
                }
                if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    apply = true;
                }
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    cancel = true;
                }
                if let Some(why) = problem.as_ref().or(dialog.error.as_ref()) {
                    ui.colored_label(error_color(ui), why);
                }
                if let Some(warning) = mentions_text(&dialog.from, mentions) {
                    ui.label(warning);
                }
                ui.horizontal(|ui| {
                    apply |= ui
                        .add_enabled(problem.is_none(), egui::Button::new(tr!("Rename")))
                        .clicked();
                    cancel |= ui.button(tr!("Cancel")).clicked();
                });
            });
        if apply && problem.is_none() {
            let (from, to) = (dialog.from.clone(), dialog.name.clone());
            if let Err(why) = self.rename_tab(&from, &to) {
                dialog.error = Some(why);
                self.rename_tab = Some(dialog);
            }
        } else if open && !cancel {
            self.rename_tab = Some(dialog);
        }
    }
}

/// What the dialog says about the formulas and modules that use the old name.
pub(crate) fn mentions_text(name: &str, mentions: gossia_calc::Mentions) -> Option<String> {
    let formulas = |n| gossia_i18n::counted(n, tr!("1 formula"), tr!("{n} formulas"));
    let modules = |n| gossia_i18n::counted(n, tr!("1 module"), tr!("{n} modules"));
    let things = match (mentions.formulas, mentions.modules) {
        (0, 0) => return None,
        (f, 0) => formulas(f),
        (0, m) => modules(m),
        (f, m) => gossia_i18n::and_list(&[formulas(f), modules(m)]),
    };
    Some(if mentions.formulas + mentions.modules == 1 {
        tr!(
            "{things} uses {name}. It is not changed, and will show #NAME! until it is edited.",
            things = things,
            name = name
        )
    } else {
        tr!(
            "{things} use {name}. They are not changed, and will show #NAME! until they are edited.",
            things = things,
            name = name
        )
    })
}

/// Selects the whole text of a field, so that typing replaces the name.
pub(crate) fn select_all(ui: &egui::Ui, response: &egui::Response, text: &str) {
    if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), response.id) {
        let all = egui::text::CCursorRange::two(
            egui::text::CCursor::new(0),
            egui::text::CCursor::new(text.chars().count()),
        );
        state.cursor.set_char_range(Some(all));
        state.store(ui.ctx(), response.id);
    }
}

fn renamed(name: String, from: &str, to: &str) -> String {
    if name == from { to.to_owned() } else { name }
}

fn rekey<V>(map: &mut HashMap<String, V>, from: &str, to: &str) {
    if let Some(value) = map.remove(from) {
        map.insert(to.to_owned(), value);
    }
}

fn rekey_pair<V>(map: &mut HashMap<(String, Path), V>, from: &str, to: &str)
where
    (String, Path): Hash + Eq,
{
    let keys: Vec<(String, Path)> = map.keys().filter(|(s, _)| s == from).cloned().collect();
    for key in keys {
        if let Some(value) = map.remove(&key) {
            map.insert((to.to_owned(), key.1), value);
        }
    }
}
