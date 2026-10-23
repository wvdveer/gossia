//! The interface between the calculation layer and whatever evaluates a
//! formula's text.
//!
//! RustPython arrives behind this trait in build step 3 (DESIGN.md, "Python
//! engine"); until then [`crate::fake::FakeEngine`] stands in. The calculation
//! layer owns everything that does not depend on the language: the data, the
//! dependency graph, the order of evaluation, and checking a result against
//! the schema.

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

/// What a formula can see while it runs. Every read is recorded as a
/// dependency of the formula, whether or not it finds anything, so no static
/// analysis of the text is needed.
pub trait Context {
    /// The tab the formula is on.
    fn sheet(&self) -> &str;
    /// The node the formula is attached to.
    fn path(&self) -> &Path;
    /// Reads the node at `path` in the tab called `sheet`.
    fn read(&mut self, sheet: &str, path: &Path) -> Read;
}

/// Something that can check and evaluate formula text. Formulas are single
/// expressions without the leading `=`.
pub trait Engine: Send + Sync {
    /// Reads the text without running it, so it is also safe in a workbook that
    /// is not trusted.
    fn check(&self, formula: &str) -> Result<(), SyntaxError>;

    /// Runs the formula. An engine reports a value, or a [`Failure`] that
    /// carries the error kind. A result that is an error value found in the
    /// data (a formula such as `=Orders.total` whose source has failed) is
    /// returned as `Ok(Value::Error(..))`.
    fn eval(&self, formula: &str, ctx: &mut dyn Context) -> Result<Value, Failure>;
}
