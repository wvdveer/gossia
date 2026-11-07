//! Tabs of calendar entries and contacts (DESIGN.md, Calendars and contacts,
//! 0.4.0): which kind a tab holds, and adding the entries of a file to it.

use std::collections::HashMap;

use gossia_core::calendar::{EntryKind, node_key};
use gossia_core::{Node, Path};

use super::{Calc, EditError};

impl Calc {
    /// The kind of entry a data tab holds: the one whose title its schema
    /// tab's root has.
    pub fn entry_kind(&self, tab: &str) -> Option<EntryKind> {
        let schema = self.schema_of(tab)?;
        let title = self
            .data(schema)?
            .value()
            .as_object()?
            .get("title")?
            .value()
            .as_str()?;
        EntryKind::from_title(title)
    }

    /// The data tabs holding entries of a kind, in tab order.
    pub fn tabs_of_kind(&self, kind: EntryKind) -> Vec<String> {
        self.sheet_names()
            .filter(|t| self.entry_kind(t) == Some(kind))
            .map(str::to_owned)
            .collect()
    }

    /// Adds entries to a tab of their kind, as one entry in the history
    /// called `label`: an entry with the key of one already in the tab
    /// replaces it in its place, and the others go at the end in their
    /// order. Gives how many were replaced and how many added. On a failure
    /// nothing is changed.
    pub fn add_entries(
        &mut self,
        tab: &str,
        kind: EntryKind,
        entries: &[Node],
        label: &str,
    ) -> Result<(usize, usize), EditError> {
        let items = self
            .data(tab)
            .ok_or_else(|| EditError::NoSheet(tab.to_owned()))?
            .value()
            .as_array()
            .ok_or_else(|| EditError::Path(format!("{tab} does not hold a list")))?;
        let mut at: HashMap<String, usize> = items
            .iter()
            .enumerate()
            .filter_map(|(i, n)| node_key(n, kind).map(|k| (k, i)))
            .collect();
        let mut len = items.len();
        let before = self.history_len().0;
        let (mut replaced, mut added) = (0, 0);
        self.begin_entry(label);
        let mut done = Ok(());
        for entry in entries {
            let key = node_key(entry, kind);
            done = match key.as_ref().and_then(|k| at.get(k)).copied() {
                Some(i) => self
                    .set_value(tab, &Path::root().index(i), entry.value().clone())
                    .map(|()| replaced += 1),
                None => self
                    .insert_item(tab, &Path::root(), len, entry.value().clone())
                    .map(|()| {
                        if let Some(k) = key {
                            at.insert(k, len);
                        }
                        len += 1;
                        added += 1;
                    }),
            };
            if done.is_err() {
                break;
            }
        }
        self.end_entry();
        if let Err(e) = done {
            if self.history_len().0 > before {
                let _ = self.undo();
            }
            return Err(e);
        }
        Ok((replaced, added))
    }
}
