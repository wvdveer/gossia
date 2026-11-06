//! The Gossia Options dialog (DESIGN.md, "The Options dialog"): one dialog,
//! a list of pages down the left, OK and Cancel at the bottom. Nothing takes
//! effect until OK, apart from the appearance, which is previewed while the
//! dialog is open and goes back on Cancel.
//!
//! The dialog holds what is typed, as text, so that a value out of range can
//! be shown with its reason without losing what was typed. OK changes the
//! workbook settings as one undoable entry and the preferences on disk.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use gossia_calc::{Ceilings, Setting, Unit, WorkbookSettings};

use gossia_i18n::tr;

use crate::app::GossiaApp;
use crate::prefs::{
    Appearance, CODE_FONT_RANGE, COLUMN_WIDTH_RANGE, COMMENT_DELAY_RANGE, COMMENT_HEIGHT_RANGE,
    COMMENT_WIDTH_RANGE, CodePrefs, Density, DisplayPrefs, FORMULA_BAR_LINES_RANGE,
    IMMEDIATE_ENTRIES_RANGE, IMMEDIATE_OUTPUT_RANGE, INDENT_RANGE, Preferences, SCALE_RANGE,
    TREE_INDENT_RANGE, ThemeChoice,
};
use crate::view::error_color;

// `LIBRARIES`: the main third-party libraries, from Cargo.lock (build.rs).
include!(concat!(env!("OUT_DIR"), "/libraries.rs"));

/// A page of the dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    General,
    Display,
    Editing,
    CodeEditor,
    Evaluation,
    DataSources,
    Reports,
    Trust,
    Files,
    About,
}

impl Page {
    pub const ALL: [Page; 10] = [
        Page::General,
        Page::Display,
        Page::Editing,
        Page::CodeEditor,
        Page::Evaluation,
        Page::DataSources,
        Page::Reports,
        Page::Trust,
        Page::Files,
        Page::About,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Page::General => tr!("General"),
            Page::Display => tr!("Display"),
            Page::Editing => tr!("Editing"),
            Page::CodeEditor => tr!("Code editor"),
            Page::Evaluation => tr!("Evaluation"),
            Page::DataSources => tr!("Data sources"),
            Page::Reports => tr!("Reports"),
            Page::Trust => tr!("Trust and security"),
            Page::Files => tr!("Files and import"),
            Page::About => tr!("About"),
        }
    }

    /// Whether the page's settings are stored in the workbook.
    pub fn is_workbook(self) -> bool {
        matches!(self, Page::Evaluation | Page::DataSources | Page::Reports)
    }
}

/// A number of the dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Field {
    /// A workbook setting.
    Workbook(Setting),
    /// The user's ceiling for a workbook setting.
    Ceiling(Setting),
    Scale,
    TreeIndent,
    ColumnWidth,
    CommentDelay,
    CommentWidth,
    CommentHeight,
    UndoEntries,
    UndoWeight,
    FormulaBarLines,
    CodeFontSize,
    IndentWidth,
    ImmediateEntries,
    ImmediateOutput,
    ZipEntries,
    TotalBytes,
    EntryBytes,
    ImportBytes,
}

/// How a number is shown and typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shown {
    /// Milliseconds, shown in seconds.
    Seconds,
    /// Bytes, shown in megabytes (MB of 1,048,576 bytes).
    Megabytes,
    Count,
    Percent,
    /// Points, of a font.
    Points,
    /// Bytes, shown in kilobytes (KB of 1,024 bytes).
    Kilobytes,
    Pixels,
}

impl Shown {
    fn of(unit: Unit) -> Shown {
        match unit {
            Unit::Millis => Shown::Seconds,
            Unit::Bytes => Shown::Megabytes,
            Unit::Count => Shown::Count,
        }
    }

    /// How many of the stored unit make one shown unit.
    fn scale(self) -> u64 {
        match self {
            Shown::Seconds => 1_000,
            Shown::Megabytes => 1 << 20,
            Shown::Kilobytes => 1 << 10,
            Shown::Count | Shown::Percent | Shown::Points | Shown::Pixels => 1,
        }
    }

    pub fn suffix(self) -> &'static str {
        match self {
            Shown::Seconds => "s",
            Shown::Megabytes => "MB",
            Shown::Count => "",
            Shown::Percent => "%",
            Shown::Points => "pt",
            Shown::Kilobytes => "KB",
            Shown::Pixels => "px",
        }
    }

    /// The text of a stored value: whole numbers stay whole, and a part is
    /// written with as few decimals as it needs, up to three.
    pub fn text(self, value: u64) -> String {
        let scale = self.scale();
        if value.is_multiple_of(scale) {
            return (value / scale).to_string();
        }
        let text = format!("{:.3}", value as f64 / scale as f64);
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    }

    /// The stored value of a text, or why it is not a number.
    pub fn parse(self, text: &str) -> Result<u64, String> {
        let text = text.trim().replace('_', "");
        if text.is_empty() {
            return Err(tr!("Give a number.").to_owned());
        }
        if let Ok(n) = text.parse::<u64>() {
            return n
                .checked_mul(self.scale())
                .ok_or_else(|| tr!("The number is too large.").to_owned());
        }
        if self.scale() > 1
            && let Ok(x) = text.parse::<f64>()
            && x.is_finite()
            && x >= 0.0
        {
            let value = (x * self.scale() as f64).round();
            if value < u64::MAX as f64 {
                return Ok(value as u64);
            }
            return Err(tr!("The number is too large.").to_owned());
        }
        Err(if self.scale() > 1 {
            tr!("Give a number, such as 2 or 0.5.").to_owned()
        } else {
            tr!("Give a whole number.").to_owned()
        })
    }
}

impl Field {
    /// Every number, in the order of the pages.
    pub fn all() -> Vec<Field> {
        let mut fields = vec![
            Field::Scale,
            Field::TreeIndent,
            Field::ColumnWidth,
            Field::CommentDelay,
            Field::CommentWidth,
            Field::CommentHeight,
            Field::UndoEntries,
            Field::UndoWeight,
            Field::FormulaBarLines,
            Field::CodeFontSize,
            Field::IndentWidth,
            Field::ImmediateEntries,
            Field::ImmediateOutput,
        ];
        fields.extend(Setting::ALL.map(Field::Workbook));
        fields.extend(Setting::ALL.map(Field::Ceiling));
        fields.extend([
            Field::ZipEntries,
            Field::TotalBytes,
            Field::EntryBytes,
            Field::ImportBytes,
        ]);
        fields
    }

    /// The name the control channel's `set_option` uses: a workbook setting's
    /// key, `ceiling.` and a key, or the key in the preferences file.
    pub fn key(self) -> String {
        match self {
            Field::Workbook(s) => s.key().to_owned(),
            Field::Ceiling(s) => format!("ceiling.{}", s.key()),
            Field::Scale => "scale_percent".to_owned(),
            Field::TreeIndent => "display.tree_indent_px".to_owned(),
            Field::ColumnWidth => "display.table_column_width_px".to_owned(),
            Field::CommentDelay => "display.comment_hover_delay_ms".to_owned(),
            Field::CommentWidth => "display.comment_popup_width_px".to_owned(),
            Field::CommentHeight => "display.comment_popup_height_px".to_owned(),
            Field::FormulaBarLines => "editing.formula_bar_lines".to_owned(),
            Field::UndoEntries => "undo.max_entries".to_owned(),
            Field::UndoWeight => "undo.max_weight".to_owned(),
            Field::CodeFontSize => "code_editor.font_size".to_owned(),
            Field::IndentWidth => "code_editor.indent_width".to_owned(),
            Field::ImmediateEntries => "code_editor.immediate_history_entries".to_owned(),
            Field::ImmediateOutput => "code_editor.immediate_max_output_bytes".to_owned(),
            Field::ZipEntries => "files.max_zip_entries".to_owned(),
            Field::TotalBytes => "files.max_total_bytes".to_owned(),
            Field::EntryBytes => "files.max_entry_bytes".to_owned(),
            Field::ImportBytes => "files.max_import_bytes".to_owned(),
        }
    }

    pub fn from_key(key: &str) -> Option<Field> {
        Field::all().into_iter().find(|f| f.key() == key)
    }

    pub fn label(self) -> &'static str {
        match self {
            Field::Workbook(s) | Field::Ceiling(s) => s.label(),
            Field::Scale => tr!("Interface scale"),
            Field::TreeIndent => tr!("Indentation per level"),
            Field::ColumnWidth => tr!("Default column width"),
            Field::CommentDelay => tr!("Hover delay"),
            Field::CommentWidth => tr!("Largest popup width"),
            Field::CommentHeight => tr!("Largest popup height"),
            Field::FormulaBarLines => tr!("Most lines before it scrolls"),
            Field::UndoEntries => tr!("Most entries kept"),
            Field::UndoWeight => tr!("Memory budget (rough units)"),
            Field::CodeFontSize => tr!("Font size"),
            Field::IndentWidth => tr!("Indent width (spaces)"),
            Field::ImmediateEntries => tr!("History entries kept"),
            Field::ImmediateOutput => tr!("Largest result shown"),
            Field::ZipEntries => tr!("Most entries in a workbook"),
            Field::TotalBytes => tr!("Largest workbook, uncompressed"),
            Field::EntryBytes => tr!("Largest entry, uncompressed"),
            Field::ImportBytes => tr!("Largest file imported"),
        }
    }

    pub fn shown(self) -> Shown {
        match self {
            Field::Workbook(s) | Field::Ceiling(s) => Shown::of(s.unit()),
            Field::Scale => Shown::Percent,
            Field::CodeFontSize => Shown::Points,
            Field::ImmediateOutput => Shown::Kilobytes,
            Field::TreeIndent | Field::ColumnWidth | Field::CommentWidth | Field::CommentHeight => {
                Shown::Pixels
            }
            Field::CommentDelay => Shown::Seconds,
            Field::UndoEntries
            | Field::UndoWeight
            | Field::ZipEntries
            | Field::IndentWidth
            | Field::FormulaBarLines
            | Field::ImmediateEntries => Shown::Count,
            Field::TotalBytes | Field::EntryBytes | Field::ImportBytes => Shown::Megabytes,
        }
    }

    /// The smallest and largest values allowed, in the stored unit.
    pub fn range(self) -> (u64, u64) {
        match self {
            Field::Workbook(s) | Field::Ceiling(s) => (s.minimum(), u64::MAX),
            Field::Scale => SCALE_RANGE,
            Field::TreeIndent => TREE_INDENT_RANGE,
            Field::ColumnWidth => COLUMN_WIDTH_RANGE,
            Field::CommentDelay => COMMENT_DELAY_RANGE,
            Field::CommentWidth => COMMENT_WIDTH_RANGE,
            Field::CommentHeight => COMMENT_HEIGHT_RANGE,
            Field::FormulaBarLines => FORMULA_BAR_LINES_RANGE,
            Field::CodeFontSize => CODE_FONT_RANGE,
            Field::IndentWidth => INDENT_RANGE,
            Field::ImmediateEntries => IMMEDIATE_ENTRIES_RANGE,
            Field::ImmediateOutput => IMMEDIATE_OUTPUT_RANGE,
            // The history and file limits are sizes in memory.
            _ => (1, usize::MAX as u64),
        }
    }

    pub fn default_value(self) -> u64 {
        let defaults = Preferences::default();
        match self {
            Field::Workbook(s) => s.default_value(),
            Field::Ceiling(s) => s.default_ceiling(),
            Field::Scale => Appearance::default().scale_percent,
            Field::TreeIndent => defaults.display.indent,
            Field::ColumnWidth => defaults.display.column_width,
            Field::CommentDelay => defaults.display.comment_delay,
            Field::CommentWidth => defaults.display.comment_width,
            Field::CommentHeight => defaults.display.comment_height,
            Field::FormulaBarLines => defaults.formula_bar_lines,
            Field::UndoEntries => defaults.history_entries as u64,
            Field::UndoWeight => defaults.history_weight as u64,
            Field::CodeFontSize => defaults.code.font_size,
            Field::IndentWidth => defaults.code.indent,
            Field::ImmediateEntries => defaults.code.history_entries,
            Field::ImmediateOutput => defaults.code.max_output,
            Field::ZipEntries => defaults.limits.entries as u64,
            Field::TotalBytes => defaults.limits.total,
            Field::EntryBytes => defaults.limits.entry,
            Field::ImportBytes => defaults.max_import_bytes,
        }
    }

    /// A value with its unit, for the dialog's words.
    pub fn with_unit(self, value: u64) -> String {
        let shown = self.shown();
        let text = shown.text(value);
        match shown {
            Shown::Count => text,
            Shown::Percent => format!("{text}%"),
            _ => format!("{text} {}", shown.suffix()),
        }
    }

    /// The value as the user has it now.
    fn current(self, settings: &WorkbookSettings, prefs: &Preferences) -> u64 {
        match self {
            Field::Workbook(s) => settings.get(s),
            Field::Ceiling(s) => prefs.ceilings.get(s),
            Field::Scale => prefs.appearance.scale_percent,
            Field::TreeIndent => prefs.display.indent,
            Field::ColumnWidth => prefs.display.column_width,
            Field::CommentDelay => prefs.display.comment_delay,
            Field::CommentWidth => prefs.display.comment_width,
            Field::CommentHeight => prefs.display.comment_height,
            Field::FormulaBarLines => prefs.formula_bar_lines,
            Field::UndoEntries => prefs.history_entries as u64,
            Field::UndoWeight => prefs.history_weight as u64,
            Field::CodeFontSize => prefs.code.font_size,
            Field::IndentWidth => prefs.code.indent,
            Field::ImmediateEntries => prefs.code.history_entries,
            Field::ImmediateOutput => prefs.code.max_output,
            Field::ZipEntries => prefs.limits.entries as u64,
            Field::TotalBytes => prefs.limits.total,
            Field::EntryBytes => prefs.limits.entry,
            Field::ImportBytes => prefs.max_import_bytes,
        }
    }
}

/// The dialog, while it is open.
#[derive(Debug, Clone, PartialEq)]
pub struct OptionsDialog {
    pub page: Page,
    /// What is typed for each number.
    pub texts: BTreeMap<Field, String>,
    /// The value each number had when the dialog opened, so that a number
    /// whose text was not changed keeps its exact value.
    start: BTreeMap<Field, u64>,
    pub theme: ThemeChoice,
    /// The language chosen; none follows the system's.
    pub language: Option<gossia_i18n::Language>,
    /// The trusted workbooks to forget on OK.
    pub revoke: BTreeSet<String>,
    /// The Display page's choices.
    pub density: Density,
    pub type_column: bool,
    pub formula_column: bool,
    /// The code editor's yes-or-no options.
    pub line_numbers: bool,
    pub highlighting: bool,
    pub wrap: bool,
    /// Clear the Immediate window's history on OK.
    pub clear_history: bool,
    /// How *Copy* writes COBOL records.
    pub binary_copy: gossia_core::binary::BinaryText,
    /// *Keep report results in the file*, a workbook setting.
    pub keep_report_results: bool,
}

impl OptionsDialog {
    pub fn new(page: Page, settings: &WorkbookSettings, prefs: &Preferences) -> OptionsDialog {
        let start: BTreeMap<Field, u64> = Field::all()
            .into_iter()
            .map(|f| (f, f.current(settings, prefs)))
            .collect();
        let texts = start
            .iter()
            .map(|(f, v)| (*f, f.shown().text(*v)))
            .collect();
        OptionsDialog {
            page,
            texts,
            start,
            theme: prefs.appearance.theme,
            language: prefs.appearance.language,
            revoke: BTreeSet::new(),
            density: prefs.display.density,
            type_column: prefs.display.type_column,
            formula_column: prefs.display.formula_column,
            line_numbers: prefs.code.line_numbers,
            highlighting: prefs.code.highlighting,
            wrap: prefs.code.wrap,
            clear_history: false,
            binary_copy: prefs.binary_copy,
            keep_report_results: settings.keep_report_results(),
        }
    }

    /// The value of a number, or why what is typed cannot be used.
    pub fn value(&self, field: Field) -> Result<u64, String> {
        let text = self.texts.get(&field).map_or("", String::as_str);
        let start = self.start[&field];
        if text == field.shown().text(start) {
            return Ok(start);
        }
        let value = field.shown().parse(text)?;
        let (low, high) = field.range();
        if value < low {
            return Err(tr!(
                "The smallest allowed is {value}.",
                value = field.with_unit(low)
            ));
        }
        if value > high {
            return Err(tr!(
                "The largest allowed is {value}.",
                value = field.with_unit(high)
            ));
        }
        Ok(value)
    }

    /// Types the default of a number.
    pub fn reset(&mut self, field: Field) {
        self.texts
            .insert(field, field.shown().text(field.default_value()));
    }

    /// Types a value for a number, as the user would.
    pub fn set(&mut self, field: Field, value: u64) {
        self.texts.insert(field, field.shown().text(value));
    }

    /// The numbers that cannot be used, with why.
    pub fn problems(&self) -> Vec<(Field, String)> {
        Field::all()
            .into_iter()
            .filter_map(|f| self.value(f).err().map(|e| (f, e)))
            .collect()
    }

    /// The appearance to preview: what is chosen, with the scale as it was
    /// while what is typed is not a scale that can be used.
    pub fn appearance(&self) -> Appearance {
        Appearance {
            theme: self.theme,
            scale_percent: self
                .value(Field::Scale)
                .unwrap_or(self.start[&Field::Scale]),
            language: self.language,
        }
    }

    /// The ceilings as typed, the ones that cannot be used left as they were.
    fn ceilings(&self) -> Ceilings {
        let mut ceilings = Ceilings::default();
        for setting in Setting::ALL {
            let field = Field::Ceiling(setting);
            ceilings.set(setting, self.value(field).unwrap_or(self.start[&field]));
        }
        ceilings
    }

    /// The workbook settings as typed. Only called when there are no problems.
    fn settings(&self, mut settings: WorkbookSettings) -> WorkbookSettings {
        for setting in Setting::ALL {
            if let Ok(value) = self.value(Field::Workbook(setting)) {
                settings.set(setting, value);
            }
        }
        settings.set_keep_report_results(self.keep_report_results);
        settings
    }

    /// The preferences as typed. Only called when there are no problems.
    fn prefs(&self, prefs: &mut Preferences) {
        let get = |f: Field| self.value(f).unwrap_or(self.start[&f]);
        prefs.appearance = self.appearance();
        prefs.ceilings = self.ceilings();
        prefs.history_entries = get(Field::UndoEntries) as usize;
        prefs.history_weight = get(Field::UndoWeight) as usize;
        prefs.formula_bar_lines = get(Field::FormulaBarLines);
        prefs.display = DisplayPrefs {
            indent: get(Field::TreeIndent),
            density: self.density,
            type_column: self.type_column,
            formula_column: self.formula_column,
            column_width: get(Field::ColumnWidth),
            comment_delay: get(Field::CommentDelay),
            comment_width: get(Field::CommentWidth),
            comment_height: get(Field::CommentHeight),
        };
        prefs.code = CodePrefs {
            font_size: get(Field::CodeFontSize),
            indent: get(Field::IndentWidth),
            line_numbers: self.line_numbers,
            highlighting: self.highlighting,
            wrap: self.wrap,
            history_entries: get(Field::ImmediateEntries),
            max_output: get(Field::ImmediateOutput),
        };
        prefs.limits.entries = get(Field::ZipEntries) as usize;
        prefs.limits.total = get(Field::TotalBytes);
        prefs.limits.entry = get(Field::EntryBytes);
        prefs.max_import_bytes = get(Field::ImportBytes);
        prefs.binary_copy = self.binary_copy;
    }
}

impl GossiaApp {
    /// Tools > Options, on a page, or on the page it was last left on.
    pub fn begin_options(&mut self, page: Option<Page>) {
        let page = page.unwrap_or(self.options_page);
        self.options = Some(OptionsDialog::new(
            page,
            self.calc().settings(),
            &self.user.prefs,
        ));
    }

    /// OK: the workbook settings change as one undoable entry, if they
    /// changed; the preferences are written; the workbooks to forget are
    /// forgotten. Nothing changes if a number cannot be used.
    pub fn apply_options(&mut self) -> Result<(), String> {
        let Some(dialog) = self.options.clone() else {
            return Ok(());
        };
        if let Some((field, why)) = dialog.problems().into_iter().next() {
            return Err(format!("{}: {why}", field.label()));
        }
        let settings = dialog.settings(self.calc().settings().clone());
        self.session.calc_mut().set_settings(settings);
        self.change_prefs(|p| dialog.prefs(p));
        let (entries, weight) = (
            self.user.prefs.history_entries,
            self.user.prefs.history_weight,
        );
        self.session.calc_mut().set_history_limits(entries, weight);
        // The history keeps no more than it may now, and is cleared if asked.
        let kept = if dialog.clear_history {
            self.user.clear_immediate()
        } else {
            self.user.keep_immediate()
        };
        if let Err(e) = kept {
            self.notice = Some(tr!("The Immediate window's history: {error}", error = e));
        }
        if !dialog.revoke.is_empty() {
            let revoke = dialog.revoke.clone();
            self.change_trust(|t| {
                for id in &revoke {
                    t.revoke(id);
                }
            });
        }
        self.options_page = dialog.page;
        self.options = None;
        Ok(())
    }

    /// Cancel: every change in the dialog is dropped, and the preview goes.
    pub fn cancel_options(&mut self) {
        if let Some(dialog) = self.options.take() {
            self.options_page = dialog.page;
        }
    }

    /// The appearance in force: the dialog's while it is open.
    pub fn appearance(&self) -> Appearance {
        self.options
            .as_ref()
            .map_or(self.user.prefs.appearance, OptionsDialog::appearance)
    }

    /// Gives the window the theme and the scale, when they change. The first
    /// frame changes only what differs from the defaults, so that a window
    /// made with a theme of its own (the tests) keeps it.
    pub(crate) fn apply_appearance(&mut self, ctx: &egui::Context) {
        let want = self.appearance();
        let before = self.appearance_applied.unwrap_or_default();
        // In a host, the system's theme is the host's, which can change at
        // any time, so it is looked at each frame.
        let host_dark = match want.theme {
            ThemeChoice::System => crate::host::current().and_then(|h| h.dark()),
            _ => None,
        };
        let shown = egui::Id::unique("host_theme_dark");
        if let Some(dark) = host_dark {
            if ctx.data(|d| d.get_temp::<bool>(shown)) != Some(dark) {
                ctx.data_mut(|d| d.insert_temp(shown, dark));
                ctx.set_theme(if dark {
                    egui::ThemePreference::Dark
                } else {
                    egui::ThemePreference::Light
                });
            }
        } else if want.theme != before.theme {
            ctx.data_mut(|d| d.remove::<bool>(shown));
            ctx.set_theme(match want.theme {
                ThemeChoice::System => egui::ThemePreference::System,
                ThemeChoice::Light => egui::ThemePreference::Light,
                ThemeChoice::Dark => egui::ThemePreference::Dark,
            });
            // The title bar is drawn by the system: tell it the theme too, or
            // a dark window keeps a light title bar under a light system.
            ctx.send_viewport_cmd_to(
                egui::ViewportId::ROOT,
                egui::ViewportCommand::SetTheme(crate::style::window_theme(want.theme)),
            );
        }
        if want.scale_percent != before.scale_percent {
            ctx.set_zoom_factor(want.scale_percent as f32 / 100.0);
        }
        // The language at the start is set before the window opens (from
        // the preferences, or the host's); here it follows a change.
        if want.language != before.language {
            gossia_i18n::set_language(self.language_for(want.language));
            // The worker of the web version is told the language with the
            // engine's settings, so they are given again.
            if cfg!(target_family = "wasm") {
                self.applied = None;
            }
        }
        self.apply_cjk_font(ctx);
        self.appearance_applied = Some(want);
    }

    /// *Save and restart*: saves the workbook and gives the file to open in
    /// the new process. A workbook without a file is not saved.
    pub fn save_for_restart(&mut self) -> Result<PathBuf, String> {
        let path = self.doc.path.clone().ok_or(tr!(
            "the workbook has no file yet: save it with Save As first"
        ))?;
        self.save()?;
        Ok(path)
    }

    fn save_and_restart(&mut self, ctx: &egui::Context) {
        let started = self.save_for_restart().and_then(|path| {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            std::process::Command::new(exe)
                .arg(path)
                .spawn()
                .map_err(|e| tr!("Gossia could not be started again: {error}", error = e))
        });
        match started {
            Ok(_) => {
                self.options = None;
                self.files.quit_confirmed = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Err(e) => self.notice = Some(e),
        }
    }

    /// The dialog, while it is open.
    pub(crate) fn options_window(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.options.take() else {
            return;
        };
        let (mut ok, mut cancel, mut restart, mut open) = (false, false, false, true);
        let problems = dialog.problems();
        egui::Window::new(tr!("Gossia Options"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .min_width(740.0)
            .max_width(740.0)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(150.0);
                        for page in Page::ALL {
                            if ui
                                .selectable_label(dialog.page == page, page.label())
                                .clicked()
                            {
                                dialog.page = page;
                            }
                        }
                    });
                    // Space, not a separator: a vertical separator takes all the height
                    // it is offered, and the window grew by it each frame.
                    ui.add_space(12.0);
                    // The width left, not a fixed one: the window's width includes
                    // its frame, and content wider than the space inside widened the
                    // frame past the title bar.
                    let width = ui.available_width();
                    ui.vertical(|ui| {
                        ui.set_width(width);
                        // The About page has the application's name as its heading.
                        if dialog.page != Page::About {
                            ui.heading(dialog.page.label());
                            ui.weak(if dialog.page.is_workbook() {
                                tr!("Applies to this workbook, and is stored in it.")
                            } else {
                                tr!("Applies to all workbooks, and is stored for you.")
                            });
                            ui.add_space(6.0);
                        }
                        egui::ScrollArea::vertical()
                            .id_salt("options_page")
                            // A fixed size, so that the dialog does not move
                            // when the page changes.
                            .max_height(360.0)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                restart |= self.options_page_ui(ui, &mut dialog);
                            });
                    });
                });
                ui.separator();
                // The line is always there, so that the buttons stay put.
                let why = problems
                    .first()
                    .map(|(field, why)| format!("{}: {why}", field.label()))
                    .unwrap_or_default();
                ui.colored_label(error_color(ui), why);
                ui.horizontal(|ui| {
                    ok = ui
                        .add_enabled(problems.is_empty(), egui::Button::new(tr!("OK")))
                        .clicked();
                    cancel = ui.button(tr!("Cancel")).clicked();
                });
                let (enter, escape) = ui.input(|i| {
                    (
                        i.key_pressed(egui::Key::Enter),
                        i.key_pressed(egui::Key::Escape),
                    )
                });
                ok |= enter && problems.is_empty();
                cancel |= escape;
            });
        self.options = Some(dialog);
        if restart {
            if let Err(e) = self.apply_options() {
                self.notice = Some(e);
                return;
            }
            self.save_and_restart(ctx);
        } else if ok {
            if let Err(e) = self.apply_options() {
                self.notice = Some(e);
            }
        } else if cancel || !open {
            self.cancel_options();
        }
    }

    /// The settings of the dialog's page. Says whether *Save and restart* was
    /// clicked.
    fn options_page_ui(&mut self, ui: &mut egui::Ui, dialog: &mut OptionsDialog) -> bool {
        let mut restart = false;
        match dialog.page {
            Page::General => {
                section(ui, tr!("Appearance"), |ui| {
                    ui.label(tr!("Theme"));
                    ui.horizontal(|ui| {
                        for theme in ThemeChoice::ALL {
                            if ui.radio(dialog.theme == theme, theme.label()).clicked() {
                                dialog.theme = theme;
                            }
                        }
                    });
                    ui.end_row();
                    number(ui, dialog, Field::Scale, None);
                    ui.label(tr!("Language"));
                    let system = self.language_for(None);
                    let follow = tr!(
                        "Follow system ({language})",
                        language = system.native_name()
                    );
                    let shown = dialog
                        .language
                        .map_or(follow.clone(), |l| l.native_name().to_owned());
                    let combo = egui::ComboBox::from_id_salt("options_language")
                        .selected_text(shown)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut dialog.language, None, follow);
                            for language in gossia_i18n::Language::ALL {
                                ui.selectable_value(
                                    &mut dialog.language,
                                    Some(language),
                                    language.native_name(),
                                );
                            }
                        });
                    crate::view::named(&combo.response, tr!("Language"));
                    ui.end_row();
                });
                note(
                    ui,
                    tr!("The font of the data views belongs to each workbook: View > Font."),
                );
            }
            Page::Display => {
                section(ui, tr!("Tree grid"), |ui| {
                    number(ui, dialog, Field::TreeIndent, None);
                    ui.label(tr!("Row density"));
                    ui.horizontal(|ui| {
                        for density in Density::ALL {
                            if ui
                                .radio(dialog.density == density, density.label())
                                .clicked()
                            {
                                dialog.density = density;
                            }
                        }
                    });
                    ui.end_row();
                    ui.label("");
                    ui.checkbox(&mut dialog.type_column, tr!("Show the Type column"));
                    ui.end_row();
                    ui.label("");
                    ui.checkbox(&mut dialog.formula_column, tr!("Show the Formula column"));
                    ui.end_row();
                });
                section(ui, tr!("Tabular view"), |ui| {
                    number(ui, dialog, Field::ColumnWidth, None);
                });
                section(ui, tr!("Comments"), |ui| {
                    number(ui, dialog, Field::CommentDelay, None);
                    number(ui, dialog, Field::CommentWidth, None);
                    number(ui, dialog, Field::CommentHeight, None);
                });
                note(
                    ui,
                    tr!(
                        "The row density applies to the tabular view too. A column keeps the width \
                     it was dragged to."
                    ),
                );
            }
            Page::Editing => {
                section(ui, tr!("Undo history"), |ui| {
                    number(ui, dialog, Field::UndoEntries, None);
                    number(ui, dialog, Field::UndoWeight, None);
                });
                section(ui, tr!("Formula bar"), |ui| {
                    number(ui, dialog, Field::FormulaBarLines, None);
                });
            }
            Page::CodeEditor => {
                section(ui, tr!("Editor"), |ui| {
                    ui.label(tr!("Font"));
                    ui.label("Geist Mono");
                    ui.end_row();
                    number(ui, dialog, Field::CodeFontSize, None);
                    number(ui, dialog, Field::IndentWidth, None);
                    ui.label("");
                    ui.checkbox(&mut dialog.line_numbers, tr!("Show line numbers"));
                    ui.end_row();
                    ui.label("");
                    ui.checkbox(&mut dialog.highlighting, tr!("Syntax highlighting"));
                    ui.end_row();
                    ui.label("");
                    ui.checkbox(&mut dialog.wrap, tr!("Wrap long lines"));
                    ui.end_row();
                });
                section(ui, tr!("Immediate window"), |ui| {
                    number(ui, dialog, Field::ImmediateEntries, None);
                    number(ui, dialog, Field::ImmediateOutput, None);
                    ui.label(tr!("History"));
                    ui.horizontal(|ui| {
                        let kept = self.user.immediate.len();
                        if dialog.clear_history {
                            ui.label(format!("{kept} entries, cleared on OK"));
                            if ui.small_button(tr!("Keep history")).clicked() {
                                dialog.clear_history = false;
                            }
                        } else {
                            ui.label(format!("{kept} entries"));
                            if ui
                                .add_enabled(
                                    kept > 0,
                                    egui::Button::new(tr!("Clear history")).small(),
                                )
                                .clicked()
                            {
                                dialog.clear_history = true;
                            }
                        }
                    });
                    ui.end_row();
                });
                note(
                    ui,
                    tr!(
                        "The history is kept for you, not in the workbook, in immediate-history.json \
                     beside the preferences."
                    ),
                );
            }
            Page::Evaluation => {
                let ceilings = dialog.ceilings();
                section(ui, tr!("Time limits"), |ui| {
                    for s in [
                        Setting::FormulaTimeLimit,
                        Setting::ProcedureTimeLimit,
                        Setting::RecalcTimeLimit,
                    ] {
                        number(ui, dialog, Field::Workbook(s), Some(ceilings.get(s)));
                    }
                });
                section(ui, tr!("Cancellation"), |ui| {
                    number(
                        ui,
                        dialog,
                        Field::Workbook(Setting::CancelGrace),
                        Some(ceilings.get(Setting::CancelGrace)),
                    );
                    // In a host a stuck formula's worker is ended, not
                    // abandoned, so there is nothing to limit or restart.
                    if crate::host::current().is_some() {
                        return;
                    }
                    number(
                        ui,
                        dialog,
                        Field::Workbook(Setting::MaxAbandoned),
                        Some(ceilings.get(Setting::MaxAbandoned)),
                    );
                    let abandoned = self.session.status().abandoned;
                    ui.label(tr!("Abandoned now"));
                    ui.horizontal(|ui| {
                        ui.label(abandoned.to_string());
                        let can = self.doc.path.is_some();
                        restart = ui
                            .add_enabled(can, egui::Button::new(tr!("Save and restart")))
                            .on_hover_text(tr!(
                                "Saves the workbook and starts Gossia again, which ends the \
                                 abandoned threads."
                            ))
                            .on_disabled_hover_text(tr!(
                                "The workbook has no file yet: save it with Save As first."
                            ))
                            .clicked();
                    });
                    ui.end_row();
                });
                section(ui, tr!("Guards"), |ui| {
                    number(
                        ui,
                        dialog,
                        Field::Workbook(Setting::MaxRange),
                        Some(ceilings.get(Setting::MaxRange)),
                    );
                });
            }
            Page::DataSources => {
                let ceilings = dialog.ceilings();
                section(ui, tr!("Limits"), |ui| {
                    for s in [Setting::SourceTimeout, Setting::MaxResponse] {
                        number(ui, dialog, Field::Workbook(s), Some(ceilings.get(s)));
                    }
                });
                note(
                    ui,
                    tr!(
                        "Data sources are not part of this version yet; these limits are kept for \
                     them."
                    ),
                );
            }
            Page::Reports => {
                let ceilings = dialog.ceilings();
                section(ui, tr!("Limits"), |ui| {
                    for s in [Setting::MaxDetailRows, Setting::MaxChartSvg] {
                        number(ui, dialog, Field::Workbook(s), Some(ceilings.get(s)));
                    }
                });
                group(ui, tr!("Storage"), |ui| {
                    ui.checkbox(
                        &mut dialog.keep_report_results,
                        tr!("Keep report results in the file"),
                    )
                    .on_hover_text(tr!(
                        "Each report keeps the values of its last run in the workbook, so it \
                             is drawn at once when the workbook is opened, and from them when it \
                             is not trusted."
                    ));
                });
                note(
                    ui,
                    tr!(
                        "Off, a report keeps only its parameters: the file stays small and holds no \
                     copies of report data, and the report is calculated again when it is shown."
                    ),
                );
            }
            Page::Trust => {
                group(ui, tr!("Trusted workbooks"), |ui| {
                    self.trusted_list(ui, dialog);
                });
                section(ui, tr!("Ceilings"), |ui| {
                    for s in Setting::ALL {
                        number(ui, dialog, Field::Ceiling(s), None);
                    }
                });
                note(
                    ui,
                    tr!(
                        "A workbook's setting is used only up to its ceiling, whatever the workbook \
                     asks for."
                    ),
                );
            }
            Page::Files => {
                section(ui, tr!("Reading files"), |ui| {
                    number(ui, dialog, Field::ZipEntries, None);
                    number(ui, dialog, Field::TotalBytes, None);
                    number(ui, dialog, Field::EntryBytes, None);
                });
                section(ui, tr!("Import"), |ui| {
                    number(ui, dialog, Field::ImportBytes, None);
                });
                section(ui, tr!("COBOL records"), |ui| {
                    ui.label(tr!("Copy as"));
                    ui.vertical(|ui| {
                        for kind in gossia_core::binary::BinaryText::ALL {
                            if ui.radio(dialog.binary_copy == kind, kind.label()).clicked() {
                                dialog.binary_copy = kind;
                            }
                        }
                    });
                    ui.end_row();
                });
                note(
                    ui,
                    &format!(
                        "JSON and XML nested more than {} levels deep are refused.",
                        gossia_core::xmltree::MAX_DEPTH
                    ),
                );
            }
            Page::About => about_page(ui),
        }
        restart
    }

    /// The trusted workbooks, each with *Revoke*, which takes effect on OK.
    fn trusted_list(&self, ui: &mut egui::Ui, dialog: &mut OptionsDialog) {
        let ids: Vec<String> = self.user.trust.ids().map(str::to_owned).collect();
        if ids.is_empty() {
            ui.weak(tr!("No workbooks are trusted."));
            return;
        }
        for id in ids {
            let name = match self.user.trust_files.get(&id) {
                Some(path) => path.display().to_string(),
                None => tr!("A workbook with no file (id {id})", id = id),
            };
            let this = id == self.doc.workbook_id;
            ui.horizontal(|ui| {
                let revoked = dialog.revoke.contains(&id);
                if revoked {
                    if ui.small_button(tr!("Keep")).clicked() {
                        dialog.revoke.remove(&id);
                    }
                } else if ui.small_button(tr!("Revoke")).clicked() {
                    dialog.revoke.insert(id.clone());
                }
                // A long path is cut to the width of the page; all of it on hover.
                let mut text = egui::RichText::new(&name);
                if revoked {
                    text = text.strikethrough();
                }
                ui.add(egui::Label::new(text).truncate())
                    .on_hover_text(&name);
            });
            if this {
                note(
                    ui,
                    tr!("Open now: a revoke applies when it is next opened."),
                );
            }
        }
    }
}

/// The About page: the application, its version and licence, and the main
/// third-party libraries with their versions and licences.
fn about_page(ui: &mut egui::Ui) {
    ui.label(egui::RichText::new("Gossia").size(26.0).strong());
    ui.label(tr!("Version {version}", version = gossia_calc::VERSION));
    ui.add_space(6.0);
    ui.add(
        egui::Label::new(tr!(
            "Workbooks of JSON and XML: each tab is a tree defined by a JSON Schema or an \
             XML Schema instead of a grid, and cells can hold Python formulas whose results \
             must match the schema."
        ))
        .wrap(),
    );
    ui.add_space(6.0);
    ui.label(tr!("Licensed under the Apache License, Version 2.0."));
    ui.hyperlink(env!("CARGO_PKG_REPOSITORY"));
    ui.add_space(8.0);
    group(ui, tr!("Third-party libraries"), |ui| {
        egui::Grid::new("about_libraries")
            .num_columns(3)
            .striped(true)
            .spacing([16.0, 4.0])
            .show(ui, |ui| {
                ui.strong(tr!("Library"));
                ui.strong(tr!("Version"));
                ui.strong(tr!("Licence"));
                ui.end_row();
                for (name, version, licence) in LIBRARIES {
                    ui.label(*name);
                    if *name == "RustPython" {
                        // Workbooks record the engine's revision, so it is shown too.
                        let revision = gossia_calc::ENGINE_REVISION;
                        ui.label(format!(
                            "{version} ({})",
                            revision.get(..12).unwrap_or(revision)
                        ))
                        .on_hover_text(tr!("Revision {revision}", revision = revision));
                    } else {
                        ui.label(*version);
                    }
                    ui.label(*licence);
                    ui.end_row();
                }
            });
    });
    note(
        ui,
        tr!(
            "Fonts: Inter, Geist Mono, Source Serif 4, Oswald, Dancing Script and Caveat, under \
         the SIL Open Font License; the Phosphor icons, under the MIT License."
        ),
    );
}

/// A titled group.
fn group(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.group(|ui| {
        ui.set_width(ui.available_width());
        ui.strong(title);
        add(ui);
    });
    ui.add_space(4.0);
}

/// A titled group of settings, in two columns so that the fields line up:
/// the label, and the rest. Each row ends with `end_row`.
fn section(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    group(ui, title, |ui| {
        egui::Grid::new(("options_section", title))
            .num_columns(2)
            // The same width in every section, so that all the fields line up.
            .min_col_width(200.0)
            .spacing([12.0, 6.0])
            .show(ui, add);
    });
}

/// Words under a section, wrapped to the width of the page.
fn note(ui: &mut egui::Ui, text: &str) {
    ui.add(egui::Label::new(egui::RichText::new(text).weak()).wrap());
    ui.add_space(4.0);
}

/// A number: its label, the field, the unit, its default and *Reset to
/// default*, why it cannot be used, and what a ceiling holds it down to.
fn number(ui: &mut egui::Ui, dialog: &mut OptionsDialog, field: Field, ceiling: Option<u64>) {
    let shown = field.shown();
    ui.label(field.label());
    ui.horizontal(|ui| {
        let text = dialog.texts.entry(field).or_default();
        // Numbers are right-aligned, as in a spreadsheet.
        let response = ui.add(
            egui::TextEdit::singleline(text)
                .desired_width(80.0)
                .align(egui::Align2::RIGHT_CENTER),
        );
        response
            .widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, field.label()));
        if !shown.suffix().is_empty() {
            ui.label(shown.suffix());
        }
        let default = field.default_value();
        ui.weak(format!("default {}", field.with_unit(default)));
        let at_default = dialog.value(field) == Ok(default);
        if ui
            .add_enabled(!at_default, egui::Button::new(tr!("Reset")).small())
            .on_hover_text(tr!("Reset to default"))
            .clicked()
        {
            dialog.reset(field);
        }
    });
    ui.end_row();
    match (dialog.value(field), ceiling) {
        (Err(why), _) => {
            ui.label("");
            ui.colored_label(error_color(ui), why);
            ui.end_row();
        }
        (Ok(value), Some(ceiling)) if value > ceiling => {
            ui.label("");
            ui.colored_label(
                ui.visuals().warn_fg_color,
                format!(
                    "{} is used: your ceiling (Trust and security).",
                    field.with_unit(ceiling)
                ),
            );
            ui.end_row();
        }
        _ => {}
    }
}
