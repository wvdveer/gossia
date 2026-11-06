//! Workbook settings (DESIGN.md, "Settings and limits"): the limits that govern
//! how a workbook's Python and data sources run. They are stored in the
//! workbook (`settings.json`), are part of the executable content that trust
//! covers, and change by undoable edits.
//!
//! A workbook cannot raise its own limits past the user's [`Ceilings`]: the
//! value used is the smaller of the two ([`WorkbookSettings::effective`]).

use gossia_i18n::tr;
use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::{Map, Value as Json};

use super::*;
use crate::python::PythonSettings;
use crate::session::EvalSettings;

/// One workbook setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Setting {
    FormulaTimeLimit,
    ProcedureTimeLimit,
    RecalcTimeLimit,
    CancelGrace,
    MaxAbandoned,
    MaxRange,
    SourceTimeout,
    MaxResponse,
    /// The most detail rows a multipage report shows.
    MaxDetailRows,
    /// The largest SVG a chart function may return.
    MaxChartSvg,
}

/// What a setting's number counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Millis,
    Count,
    Bytes,
}

impl Setting {
    /// Every setting, in the order the files and the dialog list them.
    pub const ALL: [Setting; 10] = [
        Setting::FormulaTimeLimit,
        Setting::ProcedureTimeLimit,
        Setting::RecalcTimeLimit,
        Setting::CancelGrace,
        Setting::MaxAbandoned,
        Setting::MaxRange,
        Setting::SourceTimeout,
        Setting::MaxResponse,
        Setting::MaxDetailRows,
        Setting::MaxChartSvg,
    ];

    /// The name in `settings.json` and in the ceilings of the preferences.
    pub fn key(self) -> &'static str {
        match self {
            Setting::FormulaTimeLimit => "formula_time_limit_ms",
            Setting::ProcedureTimeLimit => "procedure_time_limit_ms",
            Setting::RecalcTimeLimit => "recalc_time_limit_ms",
            Setting::CancelGrace => "cancel_grace_ms",
            Setting::MaxAbandoned => "max_abandoned_threads",
            Setting::MaxRange => "max_range_length",
            Setting::SourceTimeout => "source_timeout_ms",
            Setting::MaxResponse => "max_response_bytes",
            Setting::MaxDetailRows => "max_detail_rows",
            Setting::MaxChartSvg => "max_chart_svg_bytes",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Setting::FormulaTimeLimit => tr!("Formula time limit"),
            Setting::ProcedureTimeLimit => tr!("Button procedure time limit"),
            Setting::RecalcTimeLimit => tr!("Recalculation time limit"),
            Setting::CancelGrace => tr!("Cancel grace period"),
            Setting::MaxAbandoned => tr!("Maximum abandoned threads"),
            Setting::MaxRange => tr!("Maximum range length"),
            Setting::SourceTimeout => tr!("Data source call timeout"),
            Setting::MaxResponse => tr!("Maximum response size"),
            Setting::MaxDetailRows => tr!("Maximum detail rows"),
            Setting::MaxChartSvg => tr!("Maximum chart SVG size"),
        }
    }

    pub fn unit(self) -> Unit {
        match self {
            Setting::MaxAbandoned | Setting::MaxRange | Setting::MaxDetailRows => Unit::Count,
            Setting::MaxResponse | Setting::MaxChartSvg => Unit::Bytes,
            _ => Unit::Millis,
        }
    }

    pub fn default_value(self) -> u64 {
        match self {
            Setting::FormulaTimeLimit => 1_000,
            Setting::ProcedureTimeLimit => 10_000,
            Setting::RecalcTimeLimit => 10_000,
            Setting::CancelGrace => 500,
            Setting::MaxAbandoned => 20,
            Setting::MaxRange => 1_000_000,
            Setting::SourceTimeout => 30_000,
            Setting::MaxResponse => 16 << 20,
            Setting::MaxDetailRows => 100_000,
            Setting::MaxChartSvg => 5 << 20,
        }
    }

    /// The ceiling a user has until they change it.
    pub fn default_ceiling(self) -> u64 {
        match self {
            Setting::FormulaTimeLimit => 60_000,
            Setting::ProcedureTimeLimit => 300_000,
            Setting::RecalcTimeLimit => 600_000,
            Setting::CancelGrace => 10_000,
            Setting::MaxAbandoned => 100,
            Setting::MaxRange => 1_000_000,
            Setting::SourceTimeout => 300_000,
            Setting::MaxResponse => 256 << 20,
            Setting::MaxDetailRows => 1_000_000,
            Setting::MaxChartSvg => 64 << 20,
        }
    }

    /// The smallest value that makes sense. Only the grace period may be 0.
    pub fn minimum(self) -> u64 {
        match self {
            Setting::CancelGrace => 0,
            _ => 1,
        }
    }

    pub fn from_key(key: &str) -> Option<Setting> {
        Setting::ALL.into_iter().find(|s| s.key() == key)
    }
}

/// Reads a number for `setting`, or says why it cannot be used.
fn number(setting: Setting, value: &Json) -> Result<u64, String> {
    match value.as_u64() {
        Some(n) if n >= setting.minimum() => Ok(n),
        Some(n) => Err(format!(
            "{} is {n}, below the smallest allowed value {}",
            setting.key(),
            setting.minimum()
        )),
        None => Err(format!(
            "{} is {value}, which is not a whole number",
            setting.key()
        )),
    }
}

/// The key in `settings.json` of *Keep report results in the file*.
pub const KEEP_REPORT_RESULTS: &str = "keep_report_results";

/// The settings a workbook asks for. A setting it does not mention has its
/// default.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WorkbookSettings {
    /// Only the settings that differ from their defaults.
    values: BTreeMap<Setting, u64>,
    /// *Keep report results in the file* is off: a report's cache keeps only
    /// its parameters (DESIGN.md, Reports, "Report storage and the results
    /// cache").
    discard_report_results: bool,
    /// What a newer version wrote that this one does not know: kept, and
    /// written back unchanged.
    unknown: Map<String, Json>,
}

impl WorkbookSettings {
    pub fn get(&self, setting: Setting) -> u64 {
        self.values
            .get(&setting)
            .copied()
            .unwrap_or_else(|| setting.default_value())
    }

    /// Sets a value; one below the setting's minimum is raised to it.
    pub fn set(&mut self, setting: Setting, value: u64) {
        let value = value.max(setting.minimum());
        if value == setting.default_value() {
            self.values.remove(&setting);
        } else {
            self.values.insert(setting, value);
        }
    }

    /// Whether a report's results are kept in the file with its parameters
    /// (on until it is turned off).
    pub fn keep_report_results(&self) -> bool {
        !self.discard_report_results
    }

    pub fn set_keep_report_results(&mut self, keep: bool) {
        self.discard_report_results = !keep;
    }

    /// Every setting has its default and nothing unknown is kept, so the
    /// workbook needs no `settings.json`.
    pub fn is_default(&self) -> bool {
        self.values.is_empty() && self.unknown.is_empty() && !self.discard_report_results
    }

    /// The text of `settings.json`, or `None` when the file is not needed.
    /// The settings that differ from their defaults are written, in the
    /// order of [`Setting::ALL`], then what this version does not know, in
    /// key order.
    pub fn to_json(&self) -> Option<String> {
        if self.is_default() {
            return None;
        }
        let mut out = Map::new();
        for setting in Setting::ALL {
            if let Some(value) = self.values.get(&setting) {
                out.insert(setting.key().to_owned(), Json::from(*value));
            }
        }
        if self.discard_report_results {
            out.insert(KEEP_REPORT_RESULTS.to_owned(), Json::Bool(false));
        }
        let mut unknown: Vec<(&String, &Json)> = self.unknown.iter().collect();
        unknown.sort_by(|a, b| a.0.cmp(b.0));
        for (key, value) in unknown {
            out.insert(key.clone(), value.clone());
        }
        let mut text =
            serde_json::to_string_pretty(&Json::Object(out)).expect("JSON always serializes");
        text.push('\n');
        Some(text)
    }

    /// Reads `settings.json`. A value that cannot be used is left at its
    /// default, with a warning; keys this version does not know are kept.
    pub fn from_json(text: &str) -> (WorkbookSettings, Vec<String>) {
        let mut settings = WorkbookSettings::default();
        let mut warnings = Vec::new();
        let object = match serde_json::from_str::<Json>(text) {
            Ok(Json::Object(object)) => object,
            Ok(_) => {
                warnings.push(
                    "settings.json is not a JSON object, so the workbook has the default \
                     settings."
                        .to_owned(),
                );
                return (settings, warnings);
            }
            Err(e) => {
                warnings.push(format!(
                    "settings.json cannot be read ({e}), so the workbook has the default settings."
                ));
                return (settings, warnings);
            }
        };
        for (key, value) in object {
            if key == KEEP_REPORT_RESULTS {
                match value.as_bool() {
                    Some(keep) => settings.discard_report_results = !keep,
                    None => warnings.push(format!(
                        "In settings.json, {KEEP_REPORT_RESULTS} is {value}, which is not true \
                         or false; report results are kept."
                    )),
                }
                continue;
            }
            match Setting::from_key(&key) {
                Some(setting) => match number(setting, &value) {
                    Ok(n) => settings.set(setting, n),
                    Err(why) => warnings.push(format!(
                        "In settings.json, {why}; the default {} is used.",
                        setting.default_value()
                    )),
                },
                None => {
                    settings.unknown.insert(key, value);
                }
            }
        }
        (settings, warnings)
    }

    /// What is used: each setting held down to its ceiling.
    pub fn effective(&self, ceilings: &Ceilings) -> Effective {
        let mut values = BTreeMap::new();
        for setting in Setting::ALL {
            values.insert(setting, self.get(setting).min(ceilings.get(setting)));
        }
        Effective { values }
    }

    /// The settings a ceiling holds down, with the value asked for and the
    /// value used.
    pub fn held_down(&self, ceilings: &Ceilings) -> Vec<(Setting, u64, u64)> {
        Setting::ALL
            .into_iter()
            .filter(|s| self.get(*s) > ceilings.get(*s))
            .map(|s| (s, self.get(s), ceilings.get(s)))
            .collect()
    }
}

/// The user's ceiling for each workbook setting (an application preference).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ceilings {
    /// Only the ceilings that differ from their defaults.
    values: BTreeMap<Setting, u64>,
}

impl Ceilings {
    pub fn get(&self, setting: Setting) -> u64 {
        self.values
            .get(&setting)
            .copied()
            .unwrap_or_else(|| setting.default_ceiling())
    }

    /// Sets a ceiling; one below the setting's minimum is raised to it.
    pub fn set(&mut self, setting: Setting, value: u64) {
        let value = value.max(setting.minimum());
        if value == setting.default_ceiling() {
            self.values.remove(&setting);
        } else {
            self.values.insert(setting, value);
        }
    }
}

/// The settings in force: the workbook's, held down by the user's ceilings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Effective {
    values: BTreeMap<Setting, u64>,
}

impl Effective {
    pub fn get(&self, setting: Setting) -> u64 {
        self.values[&setting]
    }

    fn millis(&self, setting: Setting) -> Duration {
        Duration::from_millis(self.get(setting))
    }

    /// The limits of the evaluation thread.
    pub fn eval_settings(&self) -> EvalSettings {
        EvalSettings {
            formula_limit: Some(self.millis(Setting::FormulaTimeLimit)),
            recalc_limit: Some(self.millis(Setting::RecalcTimeLimit)),
            procedure_limit: Some(self.millis(Setting::ProcedureTimeLimit)),
            grace: self.millis(Setting::CancelGrace),
            max_abandoned: usize::try_from(self.get(Setting::MaxAbandoned)).unwrap_or(usize::MAX),
            ..EvalSettings::default()
        }
    }

    /// What the interpreter is made with.
    pub fn python_settings(&self) -> PythonSettings {
        PythonSettings {
            max_range: i64::try_from(self.get(Setting::MaxRange)).unwrap_or(i64::MAX),
            source_timeout: self.millis(Setting::SourceTimeout),
            max_response: self.get(Setting::MaxResponse),
            ..PythonSettings::default()
        }
    }
}

impl Calc {
    /// The settings the workbook asks for.
    pub fn settings(&self) -> &WorkbookSettings {
        &self.settings
    }

    /// Changes the workbook settings, as one undoable entry. Nothing is
    /// recorded if nothing changes.
    pub fn set_settings(&mut self, settings: WorkbookSettings) {
        if settings == self.settings {
            return;
        }
        let before = std::mem::replace(&mut self.settings, settings.clone());
        self.revision += 1;
        self.history.record(
            "Change the workbook settings".to_owned(),
            vec![Op::Settings {
                before: Box::new(before),
                after: Box::new(settings),
            }],
            None,
        );
    }

    /// Puts the settings in place, for undo and redo and for reading a file.
    pub(super) fn store_settings(&mut self, settings: WorkbookSettings) {
        self.settings = settings;
        self.revision += 1;
    }
}
