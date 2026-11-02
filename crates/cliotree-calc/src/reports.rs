//! Report tabs (DESIGN.md, "Reports" and "Managing tabs"): a report is a tab
//! in the order of the bar, among the data tabs, whose content is its design.
//!
//! Making, duplicating, deleting, renaming and moving a report tab are edits,
//! one entry each in the history. Changing a design is not, as for forms: the
//! report designer has its own undo while layout mode is on. Nothing in the
//! workbook names a report, so a rename or a delete recalculates nothing.

use cliotree_core::report::ReportDesign;

use super::{Calc, EditError};
use crate::history::Op;

/// A report tab as the history keeps it: its design and its place.
#[derive(Debug, Clone)]
pub(crate) struct SavedReport {
    pub index: usize,
    pub name: String,
    pub design: ReportDesign,
}

/// What kind of tab a name is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabKind {
    Data,
    Report,
}

impl Calc {
    /// Every tab in the order of the bar, data tabs and report tabs.
    pub fn tab_names(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .order
            .iter()
            .filter(|n| self.sheets.contains_key(*n) || self.reports.contains_key(*n))
            .cloned()
            .collect();
        // A tab made since the order was last written goes at the end.
        for name in self.sheets.keys().chain(self.reports.keys()) {
            if !out.contains(name) {
                out.push(name.clone());
            }
        }
        out
    }

    pub fn tab_kind(&self, name: &str) -> Option<TabKind> {
        if self.sheets.contains_key(name) {
            Some(TabKind::Data)
        } else if self.reports.contains_key(name) {
            Some(TabKind::Report)
        } else {
            None
        }
    }

    pub fn is_report(&self, name: &str) -> bool {
        self.reports.contains_key(name)
    }

    /// The report tabs, in the order of the bar.
    pub fn report_names(&self) -> Vec<String> {
        self.tab_names()
            .into_iter()
            .filter(|n| self.reports.contains_key(n))
            .collect()
    }

    pub fn report(&self, name: &str) -> Option<&ReportDesign> {
        self.reports.get(name)
    }

    /// Every report and its design, in the order they were made.
    pub fn reports(&self) -> impl Iterator<Item = (&str, &ReportDesign)> {
        self.reports.iter().map(|(n, d)| (n.as_str(), d))
    }

    /// Gives a report another design. As for forms, this is not an edit of
    /// the workbook: it is not in the history and recalculates nothing, but it
    /// is an unsaved change (see [`Calc::forms_version`]).
    pub fn set_report(&mut self, name: &str, design: ReportDesign) -> Result<(), EditError> {
        let Some(own) = self.reports.get_mut(name) else {
            return Err(EditError::NoSheet(name.to_owned()));
        };
        *own = design;
        self.forms_version += 1;
        self.exprs.resync = true;
        Ok(())
    }

    /// Makes a report tab at the end of the tabs: one entry in the history,
    /// *New report Sales*.
    pub fn create_report(&mut self, name: &str, design: ReportDesign) -> Result<(), EditError> {
        if let Some(why) = self.tab_name_problem(name, None) {
            return Err(EditError::BadName(why));
        }
        let saved = SavedReport {
            index: self.tab_names().len(),
            name: name.to_owned(),
            design,
        };
        self.store_insert_report(&saved);
        self.history.record(
            format!("New report {name}"),
            vec![Op::AddReport(Box::new(saved))],
            None,
        );
        Ok(())
    }

    /// The name a duplicate of `name` gets: `<name>_2`, or the next number
    /// that is free.
    pub fn duplicate_name(&self, name: &str) -> String {
        (2..)
            .map(|n| format!("{name}_{n}"))
            .find(|candidate| self.tab_name_problem(candidate, None).is_none())
            .expect("some number is free")
    }

    /// Copies a report's design to a new report tab placed after it: one
    /// entry in the history, *Duplicate report Sales*. Returns the new name.
    pub fn duplicate_report(&mut self, name: &str) -> Result<String, EditError> {
        let design = self
            .reports
            .get(name)
            .cloned()
            .ok_or_else(|| EditError::NoSheet(name.to_owned()))?;
        let copy = self.duplicate_name(name);
        let index = self
            .tab_names()
            .iter()
            .position(|n| n == name)
            .map_or(0, |i| i + 1);
        let saved = SavedReport {
            index,
            name: copy.clone(),
            design,
        };
        self.store_insert_report(&saved);
        self.history.record(
            format!("Duplicate report {name}"),
            vec![Op::AddReport(Box::new(saved))],
            None,
        );
        Ok(copy)
    }

    /// Deletes a report tab: one entry in the history, which puts it back
    /// with its design in its place.
    pub(super) fn delete_report(&mut self, name: &str) -> Result<(), EditError> {
        let saved = self
            .store_remove_report(name)
            .ok_or_else(|| EditError::NoSheet(name.to_owned()))?;
        self.history.record(
            format!("Delete tab {name}"),
            vec![Op::RemoveReport(Box::new(saved))],
            None,
        );
        Ok(())
    }

    /// Puts a report in at its place, without checking or recording.
    pub(super) fn store_insert_report(&mut self, saved: &SavedReport) {
        let mut order = self.tab_names();
        self.reports
            .insert(saved.name.clone(), saved.design.clone());
        order.insert(saved.index.min(order.len()), saved.name.clone());
        self.sync_order(order);
        self.forms_version += 1;
        self.exprs.resync = true;
        self.revision += 1;
    }

    /// Takes a report out, without checking or recording.
    pub(super) fn store_remove_report(&mut self, name: &str) -> Option<SavedReport> {
        let index = self.tab_names().iter().position(|n| n == name)?;
        let design = self.reports.shift_remove(name)?;
        self.order.retain(|n| n != name);
        self.removed.push(name.to_owned());
        self.forms_version += 1;
        self.exprs.resync = true;
        self.revision += 1;
        Some(SavedReport {
            index,
            name: name.to_owned(),
            design,
        })
    }

    /// Renames a report, which keeps its place, without checking or
    /// recording.
    pub(super) fn store_rename_report(&mut self, from: &str, to: &str) {
        let Some(at) = self.reports.get_index_of(from) else {
            return;
        };
        let (_, design) = self
            .reports
            .shift_remove_index(at)
            .expect("the index was found");
        self.reports.insert(to.to_owned(), design);
        let last = self.reports.len() - 1;
        self.reports.move_index(last, at);
        self.rename_in_order(from, to);
        self.renames.push((from.to_owned(), to.to_owned()));
        self.forms_version += 1;
        self.exprs.resync = true;
        self.revision += 1;
    }

    /// Writes down the order of the tabs, and keeps the data tabs in the
    /// same order among themselves.
    pub(super) fn sync_order(&mut self, order: Vec<String>) {
        let place = |name: &str| order.iter().position(|n| n == name).unwrap_or(usize::MAX);
        self.sheets
            .sort_by(|a, _, b, _| place(a.as_str()).cmp(&place(b.as_str())));
        self.reports
            .sort_by(|a, _, b, _| place(a.as_str()).cmp(&place(b.as_str())));
        self.order = order;
    }

    pub(super) fn rename_in_order(&mut self, from: &str, to: &str) {
        for name in &mut self.order {
            if name == from {
                *name = to.to_owned();
            }
        }
    }
}
