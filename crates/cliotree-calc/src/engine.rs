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
use std::sync::Arc;

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
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
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

/// What a formula on a node gave: a value, or bytes, which are read as the
/// node (DESIGN.md, "Raw memory"). Only a formula on a node can give bytes.
#[derive(Debug, Clone, PartialEq)]
pub enum Answer {
    Value(Value),
    Bytes(Vec<u8>),
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
    /// The bytes of the node at `path` in the tab called `sheet`, as the tab
    /// holds them (`RAW`): the read is recorded as a deep one.
    fn raw(&self, sheet: &str, path: &Path) -> Result<Vec<u8>, ErrorValue> {
        let _ = (sheet, path);
        Err(ErrorValue {
            kind: ErrorKind::Name,
            message: "RAW cannot be used here".to_owned(),
            origin: None,
        })
    }
    /// Whether `this` is the value of the node when it holds a single value,
    /// rather than the node: for the expressions of form controls and table
    /// cells, which compare it (`this > 0`). A formula keeps the node, since
    /// reading its own value would make it depend on itself.
    fn this_by_value(&self) -> bool {
        false
    }
    /// Names an expression of a control is given besides `this`, `parent`
    /// and `root`, with their values: the item and the list of each repeater
    /// around it whose items an expression gave, by the names the repeater
    /// gives them, and `this` for such an item, in place of a node.
    fn bindings(&self) -> &[(String, Value)] {
        &[]
    }
}

/// What the source of a module says about itself, found without running it.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ModuleInfo {
    /// The first part of each name imported at the top level, in order.
    pub imports: Vec<String>,
    /// Where each of `imports` is: its line and column, 1-based.
    pub import_at: Vec<(usize, usize)>,
    /// The functions defined at the top level, in order, private ones too.
    pub functions: Vec<String>,
    /// The chart types: the top-level functions marked `@chart("Name")`.
    pub charts: Vec<ChartInfo>,
}

/// A chart type, read from its function without running it (DESIGN.md,
/// Reports, "Chart functions").
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ChartInfo {
    /// The name `@chart` gives it, as the designer shows it.
    pub name: String,
    /// The function that draws it.
    pub function: String,
    /// The parameters of the function, less `width` and `height`, in order.
    pub params: Vec<ChartParam>,
    /// Where the decorator is: its line and column, 1-based.
    pub at: (usize, usize),
}

/// A parameter of a chart function: a box on the chart's property page.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ChartParam {
    pub name: String,
    /// It has no default, so the chart cannot be drawn without it.
    pub required: bool,
}

/// A module as the engine is asked to have it loaded.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ModuleSpec {
    pub name: String,
    pub source: Arc<str>,
    /// Goes up with each change; the engine loads a module again when it has
    /// changed.
    pub generation: u64,
    pub info: ModuleInfo,
    /// Why the module is not to be loaded, if that is known without running it:
    /// it does not parse, or it is on a cycle of imports.
    pub refused: Option<String>,
}

/// What loading a module came to.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ModuleReport {
    pub name: String,
    pub generation: u64,
    pub result: Result<(), String>,
}

/// Checks that formula text parses, without running it, so it is also safe in a
/// workbook that is not trusted. Formulas are single expressions without the
/// leading `=`.
pub trait Syntax: Send + Sync {
    fn check(&self, formula: &str) -> Result<(), SyntaxError>;

    /// Reads the source of a module without running it.
    fn module_info(&self, _source: &str) -> Result<ModuleInfo, SyntaxError> {
        Ok(ModuleInfo::default())
    }

    /// The standard library modules a workbook module may import, if the
    /// language knows them without running anything; `None` if it does not,
    /// and then an import is not checked before the module is loaded.
    fn standard_modules(&self) -> Option<&'static [&'static str]> {
        None
    }

    /// The reads of a data source written out in an expression, for
    /// Disconnect from Data Sources (DESIGN.md, "Data sources"). Nothing is
    /// found if the language cannot tell.
    fn source_reads(&self, _text: &str) -> SourceScan {
        SourceScan::default()
    }
}

/// The reads of data sources in an expression, and the uses of a source that
/// are not reads that can be replaced by what they give.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SourceScan {
    pub reads: Vec<SourceRead>,
    /// Each a use that stays, said in a few words.
    pub others: Vec<String>,
}

/// A read such as `source('shop').query(...)`: the byte ranges of the text
/// that could be replaced by a literal, shortest first. The first is the read
/// call itself; each next one adds an attribute, a call or a subscript on the
/// one before (`.json()`, `[0]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRead {
    pub chains: Vec<std::ops::Range<usize>>,
}

/// An expression typed in the Immediate window of the code editor (DESIGN.md,
/// "Python modules"). It runs as a formula on the node `path` of `sheet`
/// would, so `this` is that node, but nothing it reads is recorded and its
/// result is shown, not stored.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Immediate {
    /// Tells the answers apart.
    pub id: u64,
    pub text: String,
    pub sheet: String,
    pub path: Path,
    /// The longest it may run: the formula time limit.
    pub time_limit: Option<std::time::Duration>,
}

/// What an expression of the Immediate window came to.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ImmediateResult {
    pub id: u64,
    pub text: String,
    /// The result in its Python form (`repr`), or the error.
    pub result: Result<String, String>,
}

/// What a button procedure does to a tab, collected while it runs and applied
/// afterwards, all or nothing (DESIGN.md, "Button procedures").
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum WriteOp {
    /// Puts a value at the path: replaces what is there, or makes the node.
    Set(Value),
    /// Removes the node, shifting later array items down.
    Remove,
    /// Adds an item at the end of the array at the path.
    Append(Value),
}

/// One write of a procedure, in the order it was made.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Write {
    pub loc: crate::Loc,
    pub op: WriteOp,
}

/// A button procedure to run.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Call {
    /// The function, as the button names it: `name`, or `module.name`.
    pub procedure: String,
    /// The tab of the form, which is what `form` is a view of.
    pub sheet: String,
    /// The name of the form, and the label of the button (`ctx.form` and
    /// `ctx.button`).
    pub form: String,
    pub button: String,
    /// For the write procedure of a field: the text typed into it, passed as
    /// the third argument, `name(form, ctx, value)`. `button` is then the
    /// label of the field.
    pub value: Option<String>,
    /// For a control inside the items of repeaters: the item of each,
    /// outermost first. `ctx.index` is the last, the item the control is in.
    pub rows: Vec<usize>,
    /// The longest the procedure may run, if it is limited.
    pub time_limit: Option<std::time::Duration>,
}

/// Runs formulas. Not required to be `Send`.
pub trait Engine {
    /// How long one formula may run before it is stopped with `#TIMEOUT!`.
    /// Engines that cannot stop a formula ignore it.
    fn set_time_limit(&self, _limit: Option<std::time::Duration>) {}

    /// Runs the formula. An engine reports a value, or a [`Failure`] that
    /// carries the error kind. A result that is an error value found in the
    /// data (a formula such as `=Orders.total` whose source has failed) is
    /// returned as `Ok(Value::Error(..))`.
    fn eval(&self, formula: &str, ctx: Rc<dyn Context>) -> Result<Value, Failure>;

    /// Runs a formula on a node, which may give bytes as well as a value.
    fn eval_answer(&self, formula: &str, ctx: Rc<dyn Context>) -> Result<Answer, Failure> {
        self.eval(formula, ctx).map(Answer::Value)
    }

    /// Runs an expression of the Immediate window against what `ctx` reads,
    /// within `limit`, and gives its result as text. Reads are not recorded.
    fn immediate(
        &self,
        text: &str,
        ctx: Rc<dyn Context>,
        limit: Option<std::time::Duration>,
    ) -> Result<String, Failure> {
        self.set_time_limit(limit);
        self.eval(text, ctx).map(|value| format!("{value:?}"))
    }

    /// Runs a button procedure against what `ctx` reads, and gives the writes
    /// it made, which are not applied here. Reads are not recorded as
    /// dependencies of anything. An engine that has no procedures fails.
    fn call(&self, call: &Call, ctx: Rc<dyn Context>) -> Result<Vec<Write>, Failure> {
        let _ = (call, ctx);
        Err(Failure::new(
            ErrorKind::Name,
            "this engine cannot run button procedures",
        ))
    }

    /// Starts a recalculation pass. What a formula read from a data source
    /// during the previous pass is forgotten, so that the pass reads it again
    /// (DESIGN.md, "Data sources": reads are cached for one pass).
    fn begin_pass(&self) {}

    /// Makes the engine's modules the ones given: loads those that are new or
    /// have changed (and those that use them), in the order their imports
    /// need, and takes away those that are gone. Called between evaluations,
    /// never during one. `tabs` are the names of the tabs, which no function of
    /// a module may take the place of. An engine without modules reports each
    /// as loaded.
    fn sync_modules(&self, modules: &[ModuleSpec], _tabs: &[String]) -> Vec<ModuleReport> {
        modules
            .iter()
            .map(|m| ModuleReport {
                name: m.name.clone(),
                generation: m.generation,
                result: match &m.refused {
                    Some(why) => Err(why.clone()),
                    None => Ok(()),
                },
            })
            .collect()
    }
}
