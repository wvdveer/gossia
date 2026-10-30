//! Nodes hidden by hand in the tree view (DESIGN.md, "Tabular view of
//! arrays", "Hiding"). A hidden node still exists: only its row, and the rows
//! inside it, are not drawn. Its parent counts it, a thin marker sits where it
//! was, and a list on either puts it back.
//!
//! The state is view state, kept for each tab. A node is remembered by the way
//! to it, where a field or a key is its name and an item is its identity, so
//! an item stays hidden when a sort moves it. It is saved by path.

use std::collections::HashSet;

use cliotree_core::{Node, NodeId, Path, Segment};

use crate::app::ClioApp;

/// One step of the way to a hidden node.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Step {
    /// A field or the key of a map.
    Key(String),
    /// An item of an array, by identity.
    Item(NodeId),
}

/// The way to a node, from the root of the tab.
pub type HiddenKey = Vec<Step>;

/// The way to the node at `path` in `root`: `None` if an item on the way is
/// not there. A field need not be there, so an absent field can be hidden.
pub fn key_of(root: &Node, path: &Path) -> Option<HiddenKey> {
    let mut node = Some(root);
    let mut key = Vec::with_capacity(path.len());
    for segment in path.segments() {
        match segment {
            Segment::Key(name) => key.push(Step::Key(name.clone())),
            Segment::Index(i) => {
                let item = node?.value().as_array()?.get(*i)?;
                key.push(Step::Item(item.id()));
            }
        }
        node = node.and_then(|n| n.child(segment));
    }
    Some(key)
}

/// Where the node of `key` is now: `None` if an item on the way is gone.
pub fn path_of(root: &Node, key: &[Step]) -> Option<Path> {
    let mut node = Some(root);
    let mut path = Path::root();
    for step in key {
        let segment = match step {
            Step::Key(name) => Segment::Key(name.clone()),
            Step::Item(id) => {
                let items = node?.value().as_array()?;
                Segment::Index(items.iter().position(|item| item.id() == *id)?)
            }
        };
        node = node.and_then(|n| n.child(&segment));
        path.push(segment);
    }
    Some(path)
}

/// A hidden child, for the list that shows it again.
#[derive(Debug, Clone, PartialEq)]
pub struct HiddenChild {
    /// Its name as its row shows it: the field, the key, or `[2]`.
    pub name: String,
    pub path: Path,
}

impl ClioApp {
    fn hidden_set(&self) -> Option<&HashSet<HiddenKey>> {
        self.hidden.get(&self.sheet)
    }

    /// Hides the node at `path` of the current tab, and everything inside it.
    /// The root cannot be hidden. If the selection was in it, the selection
    /// moves to the nearest row that is shown.
    pub fn hide_node(&mut self, path: &Path) {
        if path.is_empty() {
            return;
        }
        let Some(key) = self
            .calc()
            .data(&self.sheet)
            .and_then(|root| key_of(root, path))
        else {
            return;
        };
        if !self
            .hidden
            .entry(self.sheet.clone())
            .or_default()
            .insert(key)
        {
            return;
        }
        let was = self.row_index(path);
        self.expansion_rev += 1;
        self.refresh_rows();
        let inside = self
            .selection
            .as_ref()
            .is_some_and(|s| s.segments().starts_with(path.segments()));
        if inside && self.editing.is_none() {
            let near = was
                .map(|i| i.min(self.rows.len().saturating_sub(1)))
                .and_then(|i| self.rows.get(i))
                .map(|row| row.path.clone());
            match near {
                Some(near) => {
                    self.select(&near);
                    self.scroll_to = self.row_index(&near);
                }
                None => self.selection = None,
            }
        }
    }

    /// Shows the node at `path` again, if it was hidden by hand.
    pub fn show_node(&mut self, path: &Path) {
        let Some(key) = self
            .calc()
            .data(&self.sheet)
            .and_then(|root| key_of(root, path))
        else {
            return;
        };
        if self
            .hidden
            .get_mut(&self.sheet)
            .is_some_and(|set| set.remove(&key))
        {
            self.expansion_rev += 1;
        }
    }

    /// Shows again every child of `parent` that was hidden by hand.
    pub fn show_all_nodes(&mut self, parent: &Path) {
        for child in self.hidden_children(parent) {
            self.show_node(&child.path);
        }
    }

    /// The children of `parent` hidden by hand: the items in their order, then
    /// the fields and keys by name.
    pub fn hidden_children(&self, parent: &Path) -> Vec<HiddenChild> {
        let (Some(root), Some(set)) = (self.calc().data(&self.sheet), self.hidden_set()) else {
            return Vec::new();
        };
        let mut out: Vec<HiddenChild> = set
            .iter()
            .filter(|key| key.len() == parent.len() + 1)
            .filter_map(|key| path_of(root, key))
            .filter(|path| path.parent().as_ref() == Some(parent))
            .map(|path| {
                let name = match path.segments().last() {
                    Some(Segment::Key(name)) => name.clone(),
                    Some(Segment::Index(i)) => format!("[{i}]"),
                    None => String::new(),
                };
                HiddenChild { name, path }
            })
            .collect();
        out.sort_by(|a, b| {
            let rank = |c: &HiddenChild| match c.path.segments().last() {
                Some(Segment::Index(i)) => (0, *i, String::new()),
                _ => (1, 0, c.name.clone()),
            };
            rank(a).cmp(&rank(b))
        });
        out
    }

    /// The paths of the nodes of `tab` hidden by hand, in order, for a save.
    pub(crate) fn hidden_paths(&self, tab: &str) -> Vec<String> {
        let (Some(root), Some(set)) = (self.calc().data(tab), self.hidden.get(tab)) else {
            return Vec::new();
        };
        let mut paths: Vec<String> = set
            .iter()
            .filter_map(|key| path_of(root, key))
            .map(|path| path.to_string())
            .collect();
        paths.sort();
        paths
    }

    /// Hides again the nodes read from a file. A path that no longer fits the
    /// data is dropped.
    pub(crate) fn restore_hidden(&mut self, tab: &str, paths: &[String]) {
        let Some(root) = self.calc().data(tab) else {
            return;
        };
        let keys: HashSet<HiddenKey> = paths
            .iter()
            .filter_map(|text| text.parse::<Path>().ok())
            .filter(|path| !path.is_empty())
            .filter_map(|path| key_of(root, &path))
            .collect();
        if keys.is_empty() {
            self.hidden.remove(tab);
        } else {
            self.hidden.insert(tab.to_owned(), keys);
        }
    }
}
