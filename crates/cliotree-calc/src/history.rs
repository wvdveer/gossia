//! The edit log: undo and redo (DESIGN.md, "Undo and redo").
//!
//! One linear history for the whole workbook. An entry is one user gesture and
//! holds the operations that make it up, each with what it replaced, so that it
//! can be run backwards and forwards. Removed subtrees are held by reference
//! (a [`Node`] is shared, not copied), and putting one back restores its
//! original `NodeId`. Computed results are not logged: after an undo the
//! formulas that read the changed nodes are simply calculated again.

use std::collections::VecDeque;

use cliotree_core::{Node, Path, Value};

/// A change, with enough in it to be reversed.
#[derive(Debug, Clone)]
pub(crate) enum Op {
    /// The existing node at `path` is replaced by another.
    Replace {
        sheet: String,
        path: Path,
        before: Node,
        after: Node,
    },
    /// A node comes into being at `path`, at `position` among its parent's
    /// children (the index of an array item, or the place of a field).
    Add {
        sheet: String,
        path: Path,
        position: usize,
        node: Node,
        /// Items that moved to a place where a formula is, and so were
        /// overwritten by its result, with the place each had before this
        /// edit, which undoing it puts them back to.
        moved: Vec<(Path, Node)>,
    },
    /// A node goes away. The reverse of [`Op::Add`].
    Remove {
        sheet: String,
        path: Path,
        position: usize,
        node: Node,
        /// As for [`Op::Add`].
        moved: Vec<(Path, Node)>,
    },
    /// The source of a module changes.
    ModuleSource {
        name: String,
        before: String,
        after: String,
    },
    /// The auto-update switch of a schema tab is turned on or off.
    AutoUpdate {
        sheet: String,
        before: bool,
        after: bool,
    },
    /// The formula at `path` changes. `value` is what the node held before a
    /// formula was put on it, with its position (`Some(None)` for no node),
    /// which running this backwards puts back, because the formula's results
    /// overwrote it.
    Formula {
        sheet: String,
        path: Path,
        before: Option<String>,
        after: Option<String>,
        value: Option<Option<(Node, usize)>>,
    },
}

impl Op {
    /// A rough size, for the bound on how much history is kept: the node, and
    /// the children of what it holds.
    fn weight(&self) -> usize {
        fn size(node: &Node) -> usize {
            1 + match node.value() {
                Value::Array(items) => items.len(),
                Value::Object(fields) => fields.len(),
                _ => 0,
            }
        }
        match self {
            Op::Replace { before, after, .. } => size(before) + size(after),
            Op::Add { node, moved, .. } | Op::Remove { node, moved, .. } => {
                size(node) + moved.iter().map(|(_, n)| size(n)).sum::<usize>()
            }
            Op::AutoUpdate { .. } => 1,
            Op::ModuleSource { before, after, .. } => 1 + (before.len() + after.len()) / 64,
            Op::Formula { value, .. } => {
                1 + value
                    .as_ref()
                    .and_then(|v| v.as_ref())
                    .map_or(0, |(node, _)| size(node))
            }
        }
    }
}

/// Edits made one after another to the same cell while typing are one entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MergeKey {
    pub sheet: String,
    pub path: Path,
}

#[derive(Debug, Clone)]
pub(crate) struct Entry {
    pub description: String,
    pub ops: Vec<Op>,
    merge: Option<MergeKey>,
    weight: usize,
}

/// An entry being built from several edits (a paste, a button procedure).
#[derive(Debug)]
struct Group {
    description: String,
    ops: Vec<Op>,
    depth: usize,
}

/// The default bounds on the history.
pub const DEFAULT_MAX_ENTRIES: usize = 1_000;
pub const DEFAULT_MAX_WEIGHT: usize = 5_000_000;

#[derive(Debug)]
pub(crate) struct History {
    undo: VecDeque<Entry>,
    redo: Vec<Entry>,
    group: Option<Group>,
    /// How many entries have been dropped from the old end.
    dropped: u64,
    /// The position in the history when the workbook was last saved.
    saved: Option<u64>,
    /// The last entry may take the next edit to the same cell into itself.
    open_for_merge: bool,
    max_entries: usize,
    max_weight: usize,
    weight: usize,
}

impl Default for History {
    fn default() -> Self {
        History {
            undo: VecDeque::new(),
            redo: Vec::new(),
            group: None,
            dropped: 0,
            // A workbook that was just made is as saved as it will be.
            saved: Some(0),
            open_for_merge: false,
            max_entries: DEFAULT_MAX_ENTRIES,
            max_weight: DEFAULT_MAX_WEIGHT,
            weight: 0,
        }
    }
}

impl History {
    fn position(&self) -> u64 {
        self.dropped + self.undo.len() as u64
    }

    pub fn set_limits(&mut self, max_entries: usize, max_weight: usize) {
        self.max_entries = max_entries.max(1);
        self.max_weight = max_weight.max(1);
        self.trim();
    }

    pub fn clear(&mut self) {
        self.dropped += self.undo.len() as u64;
        self.undo.clear();
        self.redo.clear();
        self.weight = 0;
        self.open_for_merge = false;
        self.saved = None;
    }

    pub fn is_grouping(&self) -> bool {
        self.group.is_some()
    }

    pub fn begin(&mut self, description: &str) {
        match &mut self.group {
            Some(group) => group.depth += 1,
            None => {
                self.group = Some(Group {
                    description: description.to_owned(),
                    ops: Vec::new(),
                    depth: 1,
                });
            }
        }
    }

    pub fn end(&mut self) {
        let Some(group) = &mut self.group else {
            return;
        };
        group.depth -= 1;
        if group.depth == 0
            && let Some(group) = self.group.take()
            && !group.ops.is_empty()
        {
            self.push(group.description, group.ops, None);
        }
    }

    /// Records one edit, or adds it to the entry being built.
    pub fn record(&mut self, description: String, ops: Vec<Op>, merge: Option<MergeKey>) {
        if ops.is_empty() {
            return;
        }
        match &mut self.group {
            Some(group) => group.ops.extend(ops),
            None => self.push(description, ops, merge),
        }
    }

    fn push(&mut self, description: String, ops: Vec<Op>, merge: Option<MergeKey>) {
        // Typing in one cell is one entry.
        if let Some(key) = &merge
            && self.open_for_merge
            && self.redo.is_empty()
            && let Some(last) = self.undo.back_mut()
            && last.merge.as_ref() == Some(key)
            && let Some(merged) = merge_ops(&last.ops, &ops)
        {
            self.weight -= last.weight;
            last.ops = merged;
            last.weight = last.ops.iter().map(Op::weight).sum();
            self.weight += last.weight;
            last.description = description;
            return;
        }
        if !self.redo.is_empty() {
            // A new edit after an undo ends the redo history.
            let here = self.position();
            if self.saved.is_some_and(|s| s > here) {
                self.saved = None;
            }
            self.redo.clear();
        }
        let weight = ops.iter().map(Op::weight).sum();
        self.open_for_merge = merge.is_some();
        self.weight += weight;
        self.undo.push_back(Entry {
            description,
            ops,
            merge,
            weight,
        });
        self.trim();
    }

    fn trim(&mut self) {
        while self.undo.len() > 1
            && (self.undo.len() > self.max_entries || self.weight > self.max_weight)
        {
            if let Some(oldest) = self.undo.pop_front() {
                self.weight -= oldest.weight;
                self.dropped += 1;
            }
        }
    }

    /// Ends the entry that is open to merging, so that the next edit to the
    /// same cell is an entry of its own.
    pub fn seal(&mut self) {
        self.open_for_merge = false;
    }

    pub fn pop_undo(&mut self) -> Option<Entry> {
        self.open_for_merge = false;
        let entry = self.undo.pop_back()?;
        self.weight -= entry.weight;
        Some(entry)
    }

    pub fn finish_undo(&mut self, entry: Entry) {
        self.redo.push(entry);
    }

    pub fn pop_redo(&mut self) -> Option<Entry> {
        self.open_for_merge = false;
        self.redo.pop()
    }

    pub fn finish_redo(&mut self, entry: Entry) {
        self.weight += entry.weight;
        self.undo.push_back(entry);
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.undo.back().map(|e| e.description.as_str())
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|e| e.description.as_str())
    }

    pub fn mark_saved(&mut self) {
        self.saved = Some(self.position());
        self.open_for_merge = false;
    }

    pub fn is_modified(&self) -> bool {
        self.saved != Some(self.position())
    }

    pub fn len(&self) -> (usize, usize) {
        (self.undo.len(), self.redo.len())
    }
}

/// Two edits to the same cell in a row, as one: the first one's "before" and
/// the second one's "after". Anything more involved is not merged.
fn merge_ops(first: &[Op], second: &[Op]) -> Option<Vec<Op>> {
    match (first, second) {
        (
            [
                Op::Replace {
                    sheet,
                    path,
                    before,
                    ..
                },
            ],
            [
                Op::Replace {
                    path: p2,
                    after,
                    sheet: s2,
                    ..
                },
            ],
        ) if sheet == s2 && path == p2 => Some(vec![Op::Replace {
            sheet: sheet.clone(),
            path: path.clone(),
            before: before.clone(),
            after: after.clone(),
        }]),
        _ => None,
    }
}
