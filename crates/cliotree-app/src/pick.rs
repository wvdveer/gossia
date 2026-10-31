//! Picking a place for a formula (DESIGN.md, "Decisions made for picking a
//! place"). The button right of the formula bar sets the text being typed
//! aside; the user goes to a node, on this tab or another, and clicks it, and
//! its address goes into the text where the cursor was.

use std::time::{Duration, Instant};

use cliotree_core::Path;

use crate::app::{ClioApp, CompletionUi, Editing, Focus};
use crate::tables::ColumnSel;

/// What is set aside while a place is being picked.
#[derive(Debug, Clone)]
pub struct Pick {
    pub editing: Editing,
    /// The selection on the tab of the formula, to go back to.
    pub selection: Option<Path>,
    pub column: Option<ColumnSel>,
}

/// What the formula bar says while a place is being picked.
pub const PICK_HINT: &str =
    "Click the place whose address goes into the formula; it can be on another tab. Esc cancels.";

/// How long after a pick a click that would start editing is ignored: the
/// second click of a double click on the place.
const AFTER_PICK: Duration = Duration::from_millis(600);

impl ClioApp {
    pub fn picking(&self) -> bool {
        self.pick.is_some()
    }

    /// Starts picking a place for the text being typed. Returns false if
    /// nothing is being typed.
    pub fn begin_pick(&mut self) -> bool {
        if self.pick.is_some() {
            return true;
        }
        let Some(editing) = self.editing.take() else {
            return false;
        };
        self.completion = CompletionUi::default();
        self.notice = None;
        self.pick = Some(Pick {
            editing,
            selection: self.selection.clone(),
            column: self.tables.column.clone(),
        });
        true
    }

    /// Puts the address of `path`, on the tab shown, into the text where the
    /// cursor was, and goes back to typing it in the formula bar.
    pub fn finish_pick(&mut self, path: &Path) {
        let Some(pick) = self.pick.take() else {
            return;
        };
        let address = self.name_of(path);
        let mut editing = pick.editing.clone();
        let at = editing
            .text
            .char_indices()
            .nth(editing.cursor)
            .map_or(editing.text.len(), |(i, _)| i);
        editing.text.insert_str(at, &address);
        let cursor = editing.cursor + address.chars().count();
        editing.cursor = cursor;
        editing.set_cursor = Some(cursor);
        editing.changed = true;
        self.go_back(pick, editing);
        self.picked_at = Some(Instant::now());
    }

    /// Stops picking and goes back to the text as it was.
    pub fn cancel_pick(&mut self) {
        if let Some(pick) = self.pick.take() {
            let editing = pick.editing.clone();
            self.go_back(pick, editing);
        }
    }

    /// Whether a place was picked a moment ago, so that the rest of a double
    /// click on it does not start editing it.
    pub fn just_picked(&self) -> bool {
        self.picked_at.is_some_and(|t| t.elapsed() < AFTER_PICK)
    }

    fn go_back(&mut self, pick: Pick, mut editing: Editing) {
        let exists = self.calc().sheet_names().any(|s| s == editing.sheet);
        if !exists {
            // The tab was deleted or renamed meanwhile.
            self.notice = Some(format!("The tab {} is no longer there", editing.sheet));
            return;
        }
        if self.sheet != editing.sheet {
            let sheet = editing.sheet.clone();
            self.switch_to(&sheet);
        }
        self.selection = pick.selection;
        self.tables.column = pick.column;
        // Typing goes on in the formula bar, which keeps the cursor.
        editing.in_cell = false;
        editing.focus = Focus::Bar;
        self.notice = None;
        self.editing = Some(editing);
    }
}
