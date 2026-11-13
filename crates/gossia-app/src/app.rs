//! The application window: state, and the way a frame is put together.
//!
//! The window is a menu, the formula bar, the grid of the current tab, the tabs
//! along the bottom, and the status bar (DESIGN.md, "Application structure",
//! "UI"). The drawing of each part is in its own module.

use gossia_i18n::tr;
use std::collections::{HashMap, HashSet};

use gossia_calc::{Calc, Ceilings, Effective, Session, Setting};
use gossia_core::validate::validate;
use gossia_core::{Node, Path, Value};

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
    pub items: Vec<gossia_calc::python::completion::Completion>,
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

pub struct GossiaApp {
    pub session: Session,
    pub sheet: String,
    pub selection: Option<Path>,
    pub expanded: HashMap<String, HashSet<Path>>,
    /// Bumped whenever a node is opened or closed, hidden or shown.
    pub expansion_rev: u64,
    /// The nodes hidden by hand in the tree view, for each tab.
    pub hidden: HashMap<String, HashSet<crate::tree_hidden::HiddenKey>>,
    pub rows: Vec<Row>,
    pub rows_key: Option<(String, u64, u64)>,
    pub editing: Option<Editing>,
    /// Why an entry was refused, shown under the formula bar.
    pub notice: Option<String>,
    pub completion: CompletionUi,
    /// Picking a place for the formula being typed (`pick.rs`).
    pub pick: Option<crate::pick::Pick>,
    /// When the last place was picked.
    pub picked_at: Option<web_time::Instant>,
    /// When the Beancount journals that formulas read were last checked for
    /// changes (`sources_ui::watch_journals`).
    pub journals_checked: Option<web_time::Instant>,
    pub insert_function: Option<InsertFunction>,
    pub summary: Summary,
    pub show_issues: bool,
    /// The toolbar under the menu bar (View > Toolbar).
    pub show_toolbar: bool,
    /// The node inspector on the right of the data tree.
    pub show_inspector: bool,
    /// Whether the window is used with a finger, and the style last
    /// installed for it (DESIGN.md, 17d).
    pub touch: bool,
    pub touch_applied: Option<bool>,
    /// Whether the side panels are unfolded on a narrow window.
    pub side_unfolded: bool,
    /// A formula waiting for the answer to whether it may erase what is
    /// inside its node.
    pub replace_inside: Option<crate::replace_inside::ReplaceInside>,
    /// What Tools > Disconnect from Data Sources would change, waiting for
    /// the answer.
    pub disconnect: Option<gossia_calc::Disconnect>,
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
    /// The font of the Data tree and Raw views of every tab (View > Font).
    pub data_font: crate::style::DataFont,
    /// What the form views keep between frames.
    pub form: FormState,
    /// What the Raw view keeps between frames.
    pub raw: RawState,
    /// The file the workbook is saved in, and what goes with it.
    pub doc: crate::workbook_file::Document,
    /// The dialogs of the file commands.
    pub files: crate::workbook_file::FileUi,
    /// The user's preferences and trust record, kept apart from any workbook.
    pub user: crate::prefs::User,
    /// The dialog of *Rename tab...*, while it is open.
    pub rename_tab: Option<crate::tab_rename::RenameTab>,
    /// The dialog of *Add entry...* or *Rename key...* of a map, while it is
    /// open.
    pub key_dialog: Option<crate::map_keys::KeyDialog>,
    /// The dialogs of *New tab...*, *Delete tab...* and *Change schema...*.
    pub tab_dialogs: crate::tab_commands::TabDialogs,
    /// Files read for importing, each waiting for its dialog in turn.
    pub imports: crate::import::Imports,
    /// The export dialog, while it is open.
    pub export: Option<crate::export::ExportPlan>,
    /// The PDF export of a report, while its dialog is open.
    pub pdf_export: Option<crate::report_pdf::PdfExport>,
    /// How a file is opened in the system's program for it: really, or
    /// (in tests) only noted.
    pub outside: crate::report_print::Outside,
    /// The PDF *Print...* last opened.
    pub last_printed: Option<std::path::PathBuf>,
    /// The language whose font for Japanese or Chinese the window has (none
    /// inside: no such font), once it has been given one or none.
    pub cjk_font_for: Option<Option<gossia_i18n::Language>>,
    /// The height the formula bar was dragged or expanded to, in points;
    /// None for one line, growing while a formula is typed.
    pub formula_bar_height: Option<f32>,
    /// The Options dialog, while it is open, and the page it was last left on.
    pub options: Option<crate::options::OptionsDialog>,
    pub options_page: crate::options::Page,
    /// The theme and scale the window was last given.
    pub appearance_applied: Option<crate::prefs::Appearance>,
    /// The settings the evaluation thread was last given.
    pub applied: Option<Effective>,
    /// The code editor's window and the modules open in it.
    pub code: crate::code::CodeEditor,
    /// The box a comment is written in, while it is open.
    pub comment_edit: Option<crate::comments::CommentEdit>,
    /// *Tools > Data sources...*, while it is open.
    pub sources_panel: Option<crate::sources_ui::SourcesPanel>,
    /// The report shown, as it was last laid out.
    pub report: crate::report_view::ReportView,
}

impl GossiaApp {
    /// An application showing `calc`, evaluated by the Python engine.
    pub fn new(calc: Calc) -> GossiaApp {
        let user = crate::prefs::User::in_memory();
        let effective = calc.settings().effective(&Ceilings::default());
        let evaluator =
            crate::host::evaluator(user.python_settings(&effective), effective.eval_settings());
        let mut app = GossiaApp::with_session(Session::new(calc, evaluator));
        // The engine already has this user's data sources.
        app.user = user;
        app
    }

    /// The window for a session whose evaluator is the caller's: it is kept
    /// until the settings in force change.
    pub fn with_session(session: Session) -> GossiaApp {
        // A trusted workbook is calculated when it is opened.
        GossiaApp::build(session, true)
    }

    /// The window for a session. `recalc` makes every formula out of date, as
    /// opening a trusted workbook does; an untrusted one runs nothing.
    pub(crate) fn build(mut session: Session, recalc: bool) -> GossiaApp {
        let sheet = session
            .calc()
            .tab_names()
            .into_iter()
            .next()
            .unwrap_or_default();
        if recalc {
            session.calc_mut().recalc_all();
        }
        let applied = Some(session.calc().settings().effective(&Ceilings::default()));
        let mut expanded: HashMap<String, HashSet<Path>> = HashMap::new();
        for name in session.calc().sheet_names() {
            expanded
                .entry(name.to_owned())
                .or_default()
                .insert(Path::root());
        }
        GossiaApp {
            session,
            sheet,
            selection: None,
            expanded,
            expansion_rev: 0,
            hidden: HashMap::new(),
            rows: Vec::new(),
            rows_key: None,
            editing: None,
            notice: None,
            completion: CompletionUi::default(),
            pick: None,
            picked_at: None,
            journals_checked: None,
            insert_function: None,
            summary: Summary::default(),
            show_issues: false,
            show_toolbar: true,
            show_inspector: false,
            touch: false,
            touch_applied: None,
            side_unfolded: false,
            replace_inside: None,
            disconnect: None,
            tables: Tables::default(),
            schema_dialog: None,
            scroll_to: None,
            trust: crate::trust_ui::TrustBanner::default(),
            views: HashMap::new(),
            data_font: crate::style::DataFont::default(),
            form: FormState::default(),
            raw: RawState::default(),
            doc: crate::workbook_file::Document::default(),
            files: crate::workbook_file::FileUi::default(),
            user: crate::prefs::User::in_memory(),
            rename_tab: None,
            key_dialog: None,
            tab_dialogs: crate::tab_commands::TabDialogs::default(),
            imports: crate::import::Imports::new(),
            export: None,
            pdf_export: None,
            outside: crate::report_print::Outside::System,
            last_printed: None,
            cjk_font_for: None,
            formula_bar_height: None,
            options: None,
            options_page: crate::options::Page::General,
            appearance_applied: None,
            applied,
            code: crate::code::CodeEditor::default(),
            comment_edit: None,
            sources_panel: None,
            report: Default::default(),
        }
    }

    /// The sample workbook.
    pub fn sample() -> GossiaApp {
        GossiaApp::new(crate::sample::workbook())
    }

    /// Uses the user's configuration: their limits, ceilings and trust record.
    pub fn with_user(mut self, user: crate::prefs::User) -> GossiaApp {
        self.user = user;
        let (entries, weight) = (
            self.user.prefs.history_entries,
            self.user.prefs.history_weight,
        );
        self.session.calc_mut().set_history_limits(entries, weight);
        self.applied = None;
        self.apply_settings();
        self
    }

    /// The settings in force: the workbook's, held down by the user's ceilings.
    pub fn effective_settings(&self) -> Effective {
        self.calc().settings().effective(&self.user.prefs.ceilings)
    }

    /// Gives the evaluation thread the settings in force, when they have
    /// changed since it was last given them (an undo, a file, a preference).
    pub fn apply_settings(&mut self) {
        let effective = self.effective_settings();
        self.session.calc_mut().set_report_limits(
            effective.get(Setting::MaxDetailRows) as usize,
            effective.get(Setting::MaxChartSvg) as usize,
        );
        if self.applied.as_ref() == Some(&effective) {
            return;
        }
        crate::host::reconfigure(
            self.session.evaluator_mut(),
            self.user.python_settings(&effective),
            effective.eval_settings(),
        );
        self.applied = Some(effective);
    }

    pub fn calc(&self) -> &Calc {
        self.session.calc()
    }

    /// *Tools > Recalculate* (F9): every formula runs again, so that those
    /// reading a data source read it afresh (DESIGN.md, "Volatile
    /// functions"). An untrusted workbook runs nothing, and says so.
    pub fn recalculate(&mut self) {
        if !self.calc().is_trusted() {
            self.notice = Some(
                tr!(
                    "This workbook is not trusted, so its formulas are not run. Trust it first to \
                 recalculate."
                )
                .to_owned(),
            );
            return;
        }
        self.session.calc_mut().recalc_all();
    }

    /// *Recalculate this formula* on a cell's menu: only the formula at
    /// `path` of the tab shown runs again, sending its requests afresh, and
    /// what reads its result follows (DESIGN.md, "Data sources").
    pub fn recalculate_formula(&mut self, path: &Path) {
        if !self.calc().is_trusted() {
            self.notice = Some(
                tr!(
                    "This workbook is not trusted, so its formulas are not run. Trust it first to \
                 recalculate."
                )
                .to_owned(),
            );
            return;
        }
        let sheet = self.sheet.clone();
        self.session.calc_mut().recalc_one(&sheet, path);
    }

    /// *Convert formula to values* on a cell's menu: the formula at `path` of
    /// the tab shown is removed and what it last gave stays as data, to be
    /// edited by hand.
    pub fn convert_formula_to_values(&mut self, path: &Path) {
        let sheet = self.sheet.clone();
        match self.session.calc_mut().formula_to_values(&sheet, path) {
            Ok(_) => {
                self.notice = None;
                self.session.tick();
            }
            Err(e) => self.notice = Some(e.to_string()),
        }
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
                let no_hidden = HashSet::new();
                let hidden = self.hidden.get(&self.sheet).unwrap_or(&no_hidden);
                rows::build(spec, &self.sheet, data, open, &tabular, hidden)
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
        gossia_calc::Loc::new(self.sheet.clone(), path.clone()).to_string()
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
