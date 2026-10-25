//! The trust model (DESIGN.md, "Trust model"): what a workbook's executable
//! content is, the digest of it that a decision of trust is recorded against,
//! and what the record says about a file that is opened.
//!
//! Nothing here runs anything. A [`Fingerprint`] is made from the text of the
//! formulas and the modules; a [`TrustStore`] is the part of the user's
//! configuration that remembers which workbooks were trusted, and in what state.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use sha2::{Digest, Sha256};

use super::{Calc, ModuleStatus};

/// Changes with the way the content is serialised, so that a digest made by
/// another version of the format never matches by accident.
const FORMAT: &str = "cliotree-trust-1";

/// A digest of the executable content of a workbook, and a digest for each
/// item of it, so that a workbook that has changed can say what changed.
///
/// The content is the formulas (by tab and path), the module sources and the
/// order of the modules, which decides who keeps a name, and the workbook
/// settings when they are not the defaults. Button procedure names, the
/// expressions of controls and report expressions join it when those exist.
/// Data, view state and the order of tabs are not part of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fingerprint {
    /// Hex digest of the whole.
    pub hash: String,
    /// Hex digest of each item, by a name that reads well in a list:
    /// `formula Orders!items[0].total`, `module Tax`, `order of modules`.
    pub items: BTreeMap<String, String>,
}

impl Fingerprint {
    fn from_items(items: BTreeMap<String, String>) -> Fingerprint {
        let mut whole = Sha256::new();
        whole.update(FORMAT.as_bytes());
        // `BTreeMap` iterates in key order, so the serialisation is canonical.
        for (name, digest) in &items {
            // Length-prefixed, so one item can never be read as two.
            whole.update((name.len() as u64).to_le_bytes());
            whole.update(name.as_bytes());
            whole.update(digest.as_bytes());
        }
        Fingerprint {
            hash: hex(&whole.finalize()),
            items,
        }
    }
}

fn digest_of(text: &str) -> String {
    hex(&Sha256::digest(text.as_bytes()))
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// How an item differs from the one that was trusted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Edited,
    Removed,
}

/// One thing that is not as it was when the workbook was trusted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub kind: ChangeKind,
    /// As named in [`Fingerprint::items`].
    pub item: String,
}

/// What the record says about a workbook that is opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// The id is known and the content is as it was trusted.
    Trusted,
    /// Nothing is recorded for this id: a new file, or one from somewhere else.
    Unknown,
    /// The id is known but the content is not what was trusted, so the file was
    /// changed outside the application. The list is for the user to review.
    Changed(Vec<Change>),
}

/// The workbooks the user has trusted: per workbook id, the content as it was
/// when it was trusted. The application keeps it on disk, with the user's
/// preferences.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrustStore {
    records: BTreeMap<String, Fingerprint>,
}

impl TrustStore {
    /// Records that the user trusts the workbook with this id, in this state.
    pub fn trust(&mut self, id: &str, content: Fingerprint) {
        self.records.insert(id.to_string(), content);
    }

    /// Forgets the decision. Returns whether there was one.
    pub fn revoke(&mut self, id: &str) -> bool {
        self.records.remove(id).is_some()
    }

    /// The ids that are trusted, for the list in the settings.
    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.records.keys().map(String::as_str)
    }

    /// Every decision, by workbook id, for keeping them on disk.
    pub fn records(&self) -> impl Iterator<Item = (&str, &Fingerprint)> {
        self.records.iter().map(|(id, f)| (id.as_str(), f))
    }

    pub fn is_recorded(&self, id: &str) -> bool {
        self.records.contains_key(id)
    }

    /// What to do with a workbook that has this id and this content. Both have
    /// to match: an id copied into another file does nothing by itself.
    pub fn decide(&self, id: &str, content: &Fingerprint) -> Decision {
        let Some(trusted) = self.records.get(id) else {
            return Decision::Unknown;
        };
        if trusted.hash == content.hash {
            return Decision::Trusted;
        }
        let mut changes = Vec::new();
        for (item, digest) in &content.items {
            match trusted.items.get(item) {
                None => changes.push(Change {
                    kind: ChangeKind::Added,
                    item: item.clone(),
                }),
                Some(was) if was != digest => changes.push(Change {
                    kind: ChangeKind::Edited,
                    item: item.clone(),
                }),
                Some(_) => {}
            }
        }
        for item in trusted.items.keys() {
            if !content.items.contains_key(item) {
                changes.push(Change {
                    kind: ChangeKind::Removed,
                    item: item.clone(),
                });
            }
        }
        Decision::Changed(changes)
    }

    /// An edit made inside the application to a workbook that is trusted keeps
    /// the record up to date, with no question asked. Does nothing for a
    /// workbook that is not recorded, so an untrusted one cannot become
    /// trusted by being edited.
    pub fn update(&mut self, id: &str, content: Fingerprint) -> bool {
        match self.records.get_mut(id) {
            Some(record) => {
                *record = content;
                true
            }
            None => false,
        }
    }

    /// Save As: the copy gets a new id and the trust of the original, because
    /// the user made the copy. Does nothing if the original is not recorded.
    pub fn carry(&mut self, from: &str, to: &str) -> bool {
        match self.records.get(from).cloned() {
            Some(record) => {
                self.records.insert(to.to_string(), record);
                true
            }
            None => false,
        }
    }
}

impl Calc {
    // ---------------------------------------------------------- trust

    /// Whether the workbook may run Python. A workbook made in the application
    /// is trusted; the layer that opens files makes one untrusted until the
    /// record says otherwise.
    pub fn is_trusted(&self) -> bool {
        self.trusted
    }

    /// Trusts the workbook or stops trusting it.
    ///
    /// While it is not trusted nothing is evaluated and no module is loaded:
    /// what was edited, and what depends on it, waits as stale (see
    /// [`Calc::stale`]). Trusting it loads the modules again and makes every
    /// formula out of date, as opening a trusted workbook does.
    pub fn set_trusted(&mut self, trusted: bool) {
        if self.trusted == trusted {
            return;
        }
        self.trusted = trusted;
        self.revision += 1;
        if trusted {
            // The engine may hold modules from before, or none; load them again.
            for module in self.modules.values_mut() {
                self.module_generation += 1;
                module.generation = self.module_generation;
                module.status = ModuleStatus::Pending;
            }
            self.recalc_all();
        }
    }

    /// How many formulas are waiting for a recalculation that cannot happen
    /// because the workbook is not trusted. Greyed in the grid.
    pub fn stale(&self) -> usize {
        if self.trusted { 0 } else { self.dirty.len() }
    }

    /// The executable content, as a digest to record a decision of trust
    /// against.
    pub fn fingerprint(&self) -> Fingerprint {
        let mut items = BTreeMap::new();
        for (name, sheet) in &self.sheets {
            for (path, formula) in &sheet.formulas.by_path {
                // The formulas a column formula gave the items are that one.
                if sheet.columns.iter().any(|c| c.holds(path)) {
                    continue;
                }
                items.insert(format!("formula {name}!{path}"), digest_of(&formula.text));
            }
            for column in &sheet.columns {
                items.insert(
                    format!("column formula {name}!{}", column.label()),
                    digest_of(&column.text),
                );
            }
        }
        for (name, module) in &self.modules {
            items.insert(format!("module {name}"), digest_of(&module.source));
        }
        if !self.modules.is_empty() {
            let order: Vec<&str> = self.modules.keys().map(String::as_str).collect();
            items.insert("order of modules".to_string(), digest_of(&order.join("\n")));
        }
        // The settings govern how the code runs. Default settings are no item,
        // so a workbook with no code still has no executable content.
        if let Some(text) = self.settings.to_json() {
            items.insert("workbook settings".to_string(), digest_of(&text));
        }
        Fingerprint::from_items(items)
    }
}
