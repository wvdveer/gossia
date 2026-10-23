//! The interface between the calculation layer and whatever evaluates a
//! formula's text.
//!
//! The Python engine ([`crate::python`]) sits behind this trait (DESIGN.md,
//! "Python engine"); [`crate::fake::FakeEngine`] stands in for tests that do
//! not need Python. The calculation layer owns everything that does not depend
//! on the language: the data, the dependency graph, the order of evaluation,
//! and checking a result against the schema.
//!
//! Two traits, because they live on different threads. [`Syntax`] only reads
//! text, needs no interpreter, and is used wherever a formula is entered.
//! [`Engine`] runs formulas. An engine usually owns an interpreter that cannot
//! be moved between threads, so it is made on the thread that evaluates and is
//! not required to be `Send`.

use std::rc::Rc;

use cliotree_core::{ErrorKind, ErrorValue, Node, Path, Value};

/// Why a formula's text does not parse (DESIGN.md, "Syntax errors").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxError {
    pub message: String,
    /// 1-based.
    pub line: usize,
    /// 1-based.
    pub column: usize,
}

impl std::fmt::Display for SyntaxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} (line {}, column {})",
            self.message, self.line, self.column
        )
    }
}

/// A formula that failed.
#[derive(Debug, Clone, PartialEq)]
pub struct Failure {
    pub error: ErrorValue,
    /// The engine's own account of where it went wrong, such as a Python
    /// traceback. Kept beside the error, never in the data.
    pub traceback: Option<String>,
}

impl Failure {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Failure {
        Failure {
            error: ErrorValue {
                kind,
                message: message.into(),
                origin: None,
            },
            traceback: None,
        }
    }

    /// A failure that passes on an error found in the data. The error keeps its
    /// kind and its origin, as the design requires of propagation.
    pub fn propagated(error: &ErrorValue) -> Failure {
        Failure {
            error: error.clone(),
            traceback: None,
        }
    }
}

/// What reading a node gave.
#[derive(Debug, Clone)]
pub enum Read {
    /// The node. It may hold an error value.
    Node(Node),
    /// The tab exists but has no node there.
    Missing,
    /// There is no tab of that name.
    NoSheet,
}

/// How much of a node a read depends on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Depth {
    /// The node and everything inside it.
    Deep,
    /// Only the node's own structure: that it exists, its kind, the number of
    /// items of an array, the names of the fields of an object. Edits to what
    /// is inside it do not matter. The node is still returned, but a caller
    /// that looks further in has to read what it looks at.
    Shallow,
}

/// What a formula can see while it runs. Every read is recorded as a
/// dependency of the formula, whether or not it finds anything, so no static
/// analysis of the text is needed.
///
/// A context is shared (`Rc`) so that the objects a formula works with, such as
/// Python proxies, can keep hold of it for as long as the formula runs.
pub trait Context {
    /// The tab the formula is on.
    fn sheet(&self) -> &str;
    /// The node the formula is attached to.
    fn path(&self) -> &Path;
    /// Reads the node at `path` in the tab called `sheet`.
    fn read(&self, sheet: &str, path: &Path, depth: Depth) -> Read;
    /// Records that the formula looked at a node, without handing the node
    /// over. For engines that already hold the node, for example because they
    /// read its parent.
    fn record(&self, sheet: &str, path: &Path, depth: Depth) {
        let _ = self.read(sheet, path, depth);
    }
    /// Whether a node missing at `path` is an optional field that the schema
    /// declares, as opposed to something the schema does not define.
    fn absent_ok(&self, sheet: &str, path: &Path) -> bool;
    /// The names of the tabs, for engines that make them available by name.
    fn sheets(&self) -> Vec<String>;
}

/// Checks that formula text parses, without running it, so it is also safe in a
/// workbook that is not trusted. Formulas are single expressions without the
/// leading `=`.
pub trait Syntax: Send + Sync {
    fn check(&self, formula: &str) -> Result<(), SyntaxError>;
}

/// Runs formulas. Not required to be `Send`.
pub trait Engine {
    /// Runs the formula. An engine reports a value, or a [`Failure`] that
    /// carries the error kind. A result that is an error value found in the
    /// data (a formula such as `=Orders.total` whose source has failed) is
    /// returned as `Ok(Value::Error(..))`.
    fn eval(&self, formula: &str, ctx: Rc<dyn Context>) -> Result<Value, Failure>;
}
