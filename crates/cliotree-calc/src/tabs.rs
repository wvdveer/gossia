//! Renaming a tab (DESIGN.md, "Renaming a tab").
//!
//! What belongs to the tab goes with it: its data, formulas, column formulas,
//! forms, its place in the order, and the bindings of the tabs that use it as
//! their schema. What only names it does not: formula text and module source
//! are never rewritten (DESIGN.md, "Renames do not rewrite formulas"), so they
//! are calculated again and find the tab, or not, by the name they say.

use std::sync::Arc;

use cliotree_core::Path;

use super::{Calc, EditError, SchemaRole};
use crate::history::Op;
use crate::model::Loc;
use crate::names::{self, Subject};

/// How much of the workbook mentions a name, for the warning of the rename
/// dialog.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Mentions {
    /// Formulas, a column formula counting once.
    pub formulas: usize,
    pub modules: usize,
}

impl Mentions {
    pub fn is_empty(&self) -> bool {
        self.formulas == 0 && self.modules == 0
    }
}

impl Calc {
    /// Why `name` cannot be the name of a new tab, or of the tab `renaming`
    /// renamed to it, or None. A tab may keep its own name in other letters.
    pub fn tab_name_problem(&self, name: &str, renaming: Option<&str>) -> Option<String> {
        if let Some(why) = names::problem(name, Subject::Tab) {
            return Some(why);
        }
        let lower = name.to_lowercase();
        if let Some(other) = self
            .sheets
            .keys()
            .find(|s| Some(s.as_str()) != renaming && s.to_lowercase() == lower)
        {
            return Some(format!(
                "a tab called {other} exists already (names ignore case)"
            ));
        }
        if let Some(module) = self.modules.keys().find(|m| m.to_lowercase() == lower) {
            return Some(format!(
                "{module} is the name of a module, and a tab cannot share it"
            ));
        }
        None
    }

    /// Gives a tab another name: one entry in the history. A name equal to the
    /// current one is no edit.
    pub fn rename_sheet(&mut self, from: &str, to: &str) -> Result<(), EditError> {
        if !self.sheets.contains_key(from) {
            return Err(EditError::NoSheet(from.to_owned()));
        }
        if from == to {
            return Ok(());
        }
        if let Some(why) = self.tab_name_problem(to, Some(from)) {
            return Err(EditError::BadName(why));
        }
        self.store_rename(from, to);
        self.history.record(
            format!("Rename tab {from} to {to}"),
            vec![Op::RenameSheet {
                from: from.to_owned(),
                to: to.to_owned(),
            }],
            None,
        );
        Ok(())
    }

    /// The renames done since the last call, oldest first, undoing and redoing
    /// included, so that the window can move what it keeps for a tab.
    pub fn take_renames(&mut self) -> Vec<(String, String)> {
        std::mem::take(&mut self.renames)
    }

    /// How many formulas and modules mention `name` as an identifier. It may
    /// count too many (a field of that name, a word in a string), never too
    /// few.
    pub fn mentions(&self, name: &str) -> Mentions {
        let mut mentions = Mentions::default();
        for sheet in self.sheets.values() {
            for (path, formula) in &sheet.formulas.by_path {
                if sheet.columns.iter().any(|c| c.holds(path)) {
                    continue;
                }
                if names::identifiers(&formula.text).contains(name) {
                    mentions.formulas += 1;
                }
            }
            mentions.formulas += sheet
                .columns
                .iter()
                .filter(|c| names::identifiers(&c.text).contains(name))
                .count();
        }
        mentions.modules = self
            .modules
            .values()
            .filter(|m| names::identifiers(&m.source).contains(name))
            .count();
        mentions
    }

    /// Renames without checking or recording, for an edit and for undoing and
    /// redoing one.
    pub(super) fn store_rename(&mut self, from: &str, to: &str) {
        let Some(at) = self.sheets.get_index_of(from) else {
            return;
        };
        let (_, sheet) = self
            .sheets
            .shift_remove_index(at)
            .expect("the index was found");
        // The formulas of the tab are known by their place, which has the tab's
        // name in it: what they read and whether they are out of date.
        let formulas = Arc::clone(&sheet.formulas);
        for (path, formula) in &formulas.by_path {
            let old = Loc::new(from, path.clone());
            let new = Loc::new(to, path.clone());
            self.forget_deps(&old, &formula.deps);
            self.remember_deps(&new, &formula.deps);
            if self.dirty.swap_remove(&old) {
                self.dirty.insert(new);
            }
        }
        self.sheets.insert(to.to_owned(), sheet);
        let last = self.sheets.len() - 1;
        self.sheets.move_index(last, at);
        for s in self.sheets.values_mut() {
            if let SchemaRole::Bound(schema) = &mut s.schema
                && schema == from
            {
                *schema = to.to_owned();
            }
        }
        self.forms.rename_tab(from, to);
        self.forms_version += 1;
        self.renames.push((from.to_owned(), to.to_owned()));
        // What read the old name now finds nothing, what looked for the new
        // one finds the tab, and the tab's own formulas may name it.
        self.changed(from, &Path::root());
        self.changed(to, &Path::root());
    }
}
