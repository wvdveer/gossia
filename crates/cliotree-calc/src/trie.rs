//! A set of items filed under paths, queried by relation.
//!
//! Dependencies are recorded at node level (DESIGN.md, "Decisions for
//! implementation"): reading a node depends on the node, on everything below
//! it, and on every node above it that could replace it. So an edit at a path
//! concerns every item filed at a path that is a prefix of the edit's path or
//! that the edit's path is a prefix of. This trie answers that query without
//! looking at items that are unrelated.

use std::collections::HashMap;
use std::hash::Hash;

use cliotree_core::Segment;
use indexmap::IndexSet;

#[derive(Debug, Clone)]
pub(crate) struct PathTrie<T> {
    here: IndexSet<T>,
    children: HashMap<Segment, PathTrie<T>>,
}

impl<T> Default for PathTrie<T> {
    fn default() -> Self {
        PathTrie {
            here: IndexSet::new(),
            children: HashMap::new(),
        }
    }
}

impl<T: Hash + Eq + Clone> PathTrie<T> {
    pub fn insert(&mut self, path: &[Segment], item: T) {
        let mut node = self;
        for segment in path {
            node = node.children.entry(segment.clone()).or_default();
        }
        node.here.insert(item);
    }

    /// Removes `item` from `path`, and prunes branches left empty.
    pub fn remove(&mut self, path: &[Segment], item: &T) {
        match path.split_first() {
            None => {
                self.here.swap_remove(item);
            }
            Some((first, rest)) => {
                if let Some(child) = self.children.get_mut(first) {
                    child.remove(rest, item);
                    if child.is_empty() {
                        self.children.remove(first);
                    }
                }
            }
        }
    }

    fn is_empty(&self) -> bool {
        self.here.is_empty() && self.children.is_empty()
    }

    /// Adds to `out` every item filed at `path`, above it, or below it. An item
    /// filed under several related paths can appear more than once.
    pub fn related(&self, path: &[Segment], out: &mut Vec<T>) {
        let mut node = self;
        for segment in path {
            out.extend(node.here.iter().cloned());
            match node.children.get(segment) {
                Some(child) => node = child,
                None => return,
            }
        }
        node.collect_all(out);
    }

    fn collect_all(&self, out: &mut Vec<T>) {
        out.extend(self.here.iter().cloned());
        for child in self.children.values() {
            child.collect_all(out);
        }
    }

    /// Whether anything is filed at `path`, above it, or below it.
    pub fn any_related(&self, path: &[Segment]) -> bool {
        let mut node = self;
        for segment in path {
            if !node.here.is_empty() {
                return true;
            }
            match node.children.get(segment) {
                Some(child) => node = child,
                None => return false,
            }
        }
        !node.is_empty()
    }

    /// Whether anything is filed strictly above `path`.
    pub fn first_above(&self, path: &[Segment]) -> Option<(usize, &T)> {
        let mut node = self;
        for (depth, segment) in path.iter().enumerate() {
            if let Some(item) = node.here.iter().next() {
                return Some((depth, item));
            }
            node = node.children.get(segment)?;
        }
        None
    }

    /// Adds to `out` every item filed strictly below `path`.
    pub fn below(&self, path: &[Segment], out: &mut Vec<T>) {
        let mut node = self;
        for segment in path {
            match node.children.get(segment) {
                Some(child) => node = child,
                None => return,
            }
        }
        for child in node.children.values() {
            child.collect_all(out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cliotree_core::Path;

    fn p(text: &str) -> Path {
        text.parse().unwrap()
    }

    fn related(trie: &PathTrie<u32>, text: &str) -> Vec<u32> {
        let mut out = Vec::new();
        trie.related(p(text).segments(), &mut out);
        out.sort();
        out.dedup();
        out
    }

    #[test]
    fn finds_items_above_at_and_below_a_path_but_not_beside_it() {
        let mut trie = PathTrie::default();
        trie.insert(p("a").segments(), 1);
        trie.insert(p("a.b").segments(), 2);
        trie.insert(p("a.b.c").segments(), 3);
        trie.insert(p("a.x").segments(), 4);
        trie.insert(Path::root().segments(), 5);
        assert_eq!(related(&trie, "a.b"), vec![1, 2, 3, 5]);
        assert_eq!(related(&trie, "a.b.c.d"), vec![1, 2, 3, 5]);
        assert_eq!(related(&trie, "a"), vec![1, 2, 3, 4, 5]);
        assert_eq!(related(&trie, "z"), vec![5]);
    }

    #[test]
    fn remove_takes_an_item_out_and_prunes() {
        let mut trie = PathTrie::default();
        trie.insert(p("a.b.c").segments(), 1);
        trie.remove(p("a.b.c").segments(), &1);
        assert!(trie.is_empty());
        assert!(!trie.any_related(p("a").segments()));
    }

    #[test]
    fn above_and_below_exclude_the_path_itself() {
        let mut trie = PathTrie::default();
        trie.insert(p("a").segments(), 1);
        trie.insert(p("a.b").segments(), 2);
        trie.insert(p("a.b.c").segments(), 3);
        assert_eq!(trie.first_above(p("a.b").segments()), Some((1, &1)));
        assert_eq!(trie.first_above(p("a").segments()), None);
        let mut below = Vec::new();
        trie.below(p("a").segments(), &mut below);
        below.sort();
        assert_eq!(below, vec![2, 3]);
    }
}
