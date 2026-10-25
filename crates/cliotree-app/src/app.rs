//! The application window: state, and the way a frame is put together.
//!
//! The window is a menu, the formula bar, the grid of the current tab, the tabs
//! along the bottom, and the status bar (DESIGN.md, "Application structure",
//! "UI"). The drawing of each part is in its own module.

use std::collections::{HashMap, HashSet};

use cliotree_calc::python::PythonSettings;
use cliotree_calc::{Calc, EvalSettings, Evaluator, Session, python};
use cliotree_core::validate::validate;
use cliotree_core::{Node, Path, Value};

use crate::form::{FormState, View};
use crate::raw::RawState;
use crate::rows::{self, Row};
use crate::schema::SchemaDialog;
use crate::tables::Tables;

/// What is being typed into a cell, in the formula bar or in the cell itself.
/// They are two views of the same text (DESIGN.md, "Entering formulas").
#[derive(Debug, Clone)]
pub struct Editing {
    pub sheet: String,
    pub path: Path,
    pub text: String,
    /// Where the text cursor is, in characters.
    pub cursor: usize,
    /// Which of the two should take the keyboard when the frame is drawn.
    pub focus: Focus,
    /// The text was changed by typing since editing began.
    pub changed: bool,
    /// The cell, not the bar, is where it is being typed.
    pub in_cell: bool,
    /// A cursor position to set in the next frame, after the text was changed
    /// by the program (accepting a completion, inserting a function).
    pub set_cursor: Option<usize>,
    /// What is being typed is the formula of a whole column: the array and the
    /// field of its items. `path` is then that field of the first item, which
    /// is what completion reads names relative to.
    pub column: Option<(Path, String)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Bar,
    Cell,
    None,
}

/// The popup of names under the formula bar.
#[derive(Debug, Clone, Default)]
pub struct CompletionUi {
    pub start: usize,
    pub items: Vec<cliotree_calc::python::completion::Completion>,
    pub selected: usize,
    /// The text for which the popup was dismissed with Esc.
    pub dismissed_for: Option<String>,
}

/// The Insert Function dialog.
#[derive(Debug, Clone, Default)]
pub struct InsertFunction {
    pub search: String,
    pub selected: Option<&'static str>,
}

/// Counts for the status bar, kept for one version of the workbook.
#[derive(Debug, Clone, Default)]
pub struct Summary {
    pub key: Option<(String, u64)>,
    pub errors: usize,
    pub invalid: usize,
}

pub struct ClioApp {
    pub session: Session,
    pub sheet: String,
    pub selection: Option<Path>,
    pub expanded: HashMap<String, HashSet<Path>>,
    /// Bumped whenever a node is opened or closed.
    pub expansion_rev: u64,
    pub rows: Vec<Row>,
    pub rows_key: Option<(String, u64, u64)>,
    pub editing: Option<Editing>,
    /// Why an entry was refused, shown under the formula bar.
    pub notice: Option<String>,
    pub completion: CompletionUi,
    pub insert_function: Option<InsertFunction>,
    pub summary: Summary,
    pub show_issues: bool,
    /// The tabular view of arrays: which arrays, and the view state of each.
    pub tables: Tables,
    /// A field command waiting for a name or a confirmation.
    pub schema_dialog: Option<SchemaDialog>,
    /// Scroll the grid so that this row is in view.
    pub scroll_to: Option<usize>,
    /// What the trust banner shows while the workbook is not trusted.
    pub trust: crate::trust_ui::TrustBanner,
    /// The view each tab shows. A tab not in the map shows its data tree.
    pub views: HashMap<String, View>,
    /// What the form views keep between frames.
    pub form: FormState,
    /// What the Raw view keeps between frames.
    pub raw: RawState,
    /// The file the workbook is saved in, and what goes with it.
    pub doc: crate::workbook_file::Document,
    /// The dialogs of the file commands.
    pub files: crate::workbook_file::FileUi,
}

impl ClioApp {
    /// An application showing `calc`, evaluated by the Python engine.
    pub fn new(calc: Calc) -> ClioApp {
        let evaluator = Evaluator::new(
            python::factory(PythonSettings::default()),
            EvalSettings::default(),
        );
        ClioApp::with_session(Session::new(calc, evaluator))
    }

    pub fn with_session(session: Session) -> ClioApp {
        // A trusted workbook is calculated when it is opened.
        ClioApp::build(session, true)
    }

    /// The window for a session. `recalc` makes every formula out of date, as
    /// opening a trusted workbook does; an untrusted one runs nothing.
    pub(crate) fn build(mut session: Session, recalc: bool) -> ClioApp {
        let sheet = session
            .calc()
            .sheet_names()
            .next()
            .unwrap_or_default()
            .to_owned();
        if recalc {
            session.calc_mut().recalc_all();
        }
        let mut expanded: HashMap<String, HashSet<Path>> = HashMap::new();
        for name in session.calc().sheet_names() {
            expanded
                .entry(name.to_owned())
                .or_default()
                .insert(Path::root());
        }
        ClioApp {
            session,
            sheet,
            selection: None,
            expanded,
            expansion_rev: 0,
            rows: Vec::new(),
            rows_key: None,
            editing: None,
            notice: None,
            completion: CompletionUi::default(),
            insert_function: None,
            summary: Summary::default(),
            show_issues: false,
            tables: Tables::default(),
            schema_dialog: None,
            scroll_to: None,
            trust: crate::trust_ui::TrustBanner::default(),
            views: HashMap::new(),
            form: FormState::default(),
            raw: RawState::default(),
            doc: crate::workbook_file::Document::default(),
            files: crate::workbook_file::FileUi::default(),
        }
    }

    /// The sample workbook.
    pub fn sample() -> ClioApp {
        ClioApp::new(crate::sample::workbook())
    }

    pub fn calc(&self) -> &Calc {
        self.session.calc()
    }

    // ----------------------------------------------------------- the rows

    /// Makes the rows of the current tab if the data or the open nodes have
    /// changed since they were made.
    pub fn refresh_rows(&mut self) {
        let key = (
            self.sheet.clone(),
            self.session.calc().revision(),
            self.expansion_rev,
        );
        if self.rows_key.as_ref() == Some(&key) {
            return;
        }
        let tabular = self.tabular_paths();
        let calc = self.session.calc();
        self.rows = match (calc.spec(&self.sheet), calc.data(&self.sheet)) {
            (Some(spec), Some(data)) => {
                let none = HashSet::new();
                let open = self.expanded.get(&self.sheet).unwrap_or(&none);
                rows::build(spec, &self.sheet, data, open, &tabular)
            }
            _ => Vec::new(),
        };
        self.rows_key = Some(key);
    }

    /// The counts for the status bar, made again only when the data changed.
    pub fn refresh_summary(&mut self) {
        let key = (self.sheet.clone(), self.session.calc().revision());
        if self.summary.key.as_ref() == Some(&key) {
            return;
        }
        let calc = self.session.calc();
        let (mut errors, mut invalid) = (0, 0);
        if let (Some(spec), Some(data)) = (calc.spec(&self.sheet), calc.data(&self.sheet)) {
            count_errors(data, &mut errors);
            invalid = validate(spec, data).len();
        }
        self.summary = Summary {
            key: Some(key),
            errors,
            invalid,
        };
    }

    pub fn is_open(&self, path: &Path) -> bool {
        self.expanded
            .get(&self.sheet)
            .is_some_and(|set| set.contains(path))
    }

    pub fn set_open(&mut self, path: &Path, open: bool) {
        let set = self.expanded.entry(self.sheet.clone()).or_default();
        let changed = if open {
            set.insert(path.clone())
        } else {
            set.remove(path)
        };
        if changed {
            self.expansion_rev += 1;
        }
    }

    pub fn row_index(&self, path: &Path) -> Option<usize> {
        self.rows.iter().position(|r| &r.path == path)
    }

    // ------------------------------------------------------------- tabs

    pub fn switch_to(&mut self, sheet: &str) {
        if sheet == self.sheet {
            return;
        }
        self.cancel_editing();
        self.form.layout = None;
        self.sheet = sheet.to_owned();
        self.selection = None;
        self.notice = None;
        self.rows_key = None;
        self.refresh_rows();
    }

    /// `Orders.items[2].price`, the name a formula would use.
    pub fn name_of(&self, path: &Path) -> String {
        cliotree_calc::Loc::new(self.sheet.clone(), path.clone()).to_string()
    }
}

fn count_errors(node: &Node, errors: &mut usize) {
    match node.value() {
        Value::Error(_) => *errors += 1,
        Value::Array(items) => items.iter().for_each(|n| count_errors(n, errors)),
        Value::Object(fields) => fields.values().for_each(|n| count_errors(n, errors)),
        _ => {}
    }
}
