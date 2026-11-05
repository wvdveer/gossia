//! Where formulas live and what is kept about them.

use std::fmt;

use gossia_core::{ErrorValue, Path};
use indexmap::IndexMap;

use crate::trie::PathTrie;

/// A place in the workbook: a node in one tab. A formula belongs to a `Loc`,
/// not to the item that happens to be there (DESIGN.md, "References are
/// literal").
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Loc {
    pub sheet: String,
    pub path: Path,
}

impl Loc {
    pub fn new(sheet: impl Into<String>, path: Path) -> Loc {
        Loc {
            sheet: sheet.into(),
            path,
        }
    }
}

impl fmt::Display for Loc {
    /// `Orders.items[2].price`, as a formula would write it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let path = self.path.to_string();
        if path.is_empty() {
            f.write_str(&self.sheet)
        } else if path.starts_with('[') {
            write!(f, "{}{path}", self.sheet)
        } else {
            write!(f, "{}.{path}", self.sheet)
        }
    }
}

/// Something a formula read. A deep read is of the node and everything in it;
/// a shallow read is only of the node's own structure: whether it exists, what
/// kind it is, and how many items an array has. Moving through a proxy such as
/// `Orders.items[2]` reads shallowly, so that a formula on one item can look at
/// a sibling without depending on the whole array.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Dep {
    pub loc: Loc,
    pub shallow: bool,
}

/// Whether one path is the other or lies inside it.
pub(crate) fn overlaps(a: &Path, b: &Path) -> bool {
    a.segments().starts_with(b.segments()) || b.segments().starts_with(a.segments())
}

/// A formula and what the last evaluation of it found out.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct Formula {
    /// The expression, without the leading `=`.
    pub text: String,
    /// Everything it read the last time it ran, whether or not it found a node.
    pub deps: Vec<Dep>,
    /// The error the node holds, kept here too because a data file cannot hold
    /// one (DESIGN.md, "Stored values").
    pub error: Option<ErrorValue>,
    pub traceback: Option<String>,
    /// The text did not parse. Such a formula is kept but never run.
    pub syntax_error: bool,
    /// What it returned the last time, when its node's type refused it
    /// (`#TYPE!`): kept for *Force object definition*.
    pub rejected: Option<std::sync::Arc<gossia_core::Value>>,
    /// The nodes of its result that did not fit their type, when the rest
    /// did (`Computed::Partial`): the tree shows `#TYPE!` on the way to them.
    #[serde(default)]
    pub misfits: Vec<Path>,
}

/// The formulas of one tab, by the path of the node they are attached to.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct Formulas {
    pub by_path: IndexMap<Path, Formula>,
    pub index: PathTrie<Path>,
}

impl Formulas {
    /// Inserts a formula, returning the one it replaced.
    pub fn insert(&mut self, path: Path, formula: Formula) -> Option<Formula> {
        let old = self.by_path.insert(path.clone(), formula);
        if old.is_none() {
            let segments = path.segments().to_vec();
            self.index.insert(&segments, path);
        }
        old
    }

    pub fn remove(&mut self, path: &Path) -> Option<Formula> {
        let removed = self.by_path.shift_remove(path)?;
        self.index.remove(path.segments(), path);
        Some(removed)
    }

    /// The outermost formula strictly above `path`, which makes the node at
    /// `path` part of a computed subtree.
    pub fn computed_by(&self, path: &Path) -> Option<Path> {
        let (depth, _) = self.index.first_above(path.segments())?;
        Some(Path::from(path.segments()[..depth].to_vec()))
    }

    /// Formulas strictly below `path`.
    pub fn below(&self, path: &Path) -> Vec<Path> {
        let mut out = Vec::new();
        self.index.below(path.segments(), &mut out);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(text: &str) -> Path {
        text.parse().unwrap()
    }

    fn formula(text: &str) -> Formula {
        Formula {
            text: text.to_string(),
            deps: Vec::new(),
            error: None,
            traceback: None,
            syntax_error: false,
            rejected: None,
            misfits: Vec::new(),
        }
    }

    #[test]
    fn loc_prints_as_a_formula_would_write_it() {
        assert_eq!(
            Loc::new("Orders", p("items[2].price")).to_string(),
            "Orders.items[2].price"
        );
        assert_eq!(Loc::new("Orders", Path::root()).to_string(), "Orders");
    }

    #[test]
    fn overlap_is_prefix_either_way() {
        assert!(overlaps(&p("a.b"), &p("a")));
        assert!(overlaps(&p("a"), &p("a.b")));
        assert!(overlaps(&p("a"), &Path::root()));
        assert!(!overlaps(&p("a.b"), &p("a.c")));
        assert!(!overlaps(&p("a[1]"), &p("a[2]")));
    }

    #[test]
    fn computed_by_finds_the_formula_above() {
        let mut formulas = Formulas::default();
        formulas.insert(p("items"), formula("1"));
        formulas.insert(p("total"), formula("2"));
        assert_eq!(formulas.computed_by(&p("items[3].price")), Some(p("items")));
        assert_eq!(formulas.computed_by(&p("items")), None);
        assert_eq!(formulas.computed_by(&p("total")), None);
        assert_eq!(formulas.below(&Path::root()).len(), 2);
        formulas.remove(&p("items"));
        assert_eq!(formulas.computed_by(&p("items[3]")), None);
    }
}
