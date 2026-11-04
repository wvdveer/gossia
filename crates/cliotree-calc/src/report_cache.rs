//! The results cache of a report (DESIGN.md, Reports, "Report storage and
//! the results cache"): the values its expressions came to in its last run,
//! with the parameters of that run, kept in the workbook as
//! `reports/<Report>/cache.json`. It is to a report what stored values are
//! to formulas: the report can be drawn from it without running Python.
//!
//! A run is taken into the cache when every expression of the report being
//! shown has its result: when the report stops being shown, and when the
//! workbook is saved. A cache made for another design (its digest differs)
//! is not used.
//!
//! The cache is data, not executable content: it is not in the trust digest.
//! Values are written as JSON, with a tag for what JSON has no kind for: a
//! date `{"$date": "2026-01-31"}`, a date and time `{"$datetime": ...}`, an
//! error value `{"$error": {"kind": "#DIV/0!", "message": ...}}`, a float
//! that is not finite `{"$float": "nan"}`, and an object with a key that
//! starts with `$` `{"$object": {...}}`.

use chrono::{DateTime, NaiveDate, NaiveDateTime};
use cliotree_core::report::ReportDesign;
use cliotree_core::{DateTimeValue, ErrorKind, ErrorValue, Node, Value};
use indexmap::IndexMap;
use serde_json::{Map, Value as Json, json};
use sha2::{Digest, Sha256};

use super::{Calc, ExprKind, REPORT_FORM};

/// The version of `cache.json` this build writes.
const CACHE_VERSION: u64 = 1;

/// Where a cached result belongs: the control, its column, the items, and
/// the expression.
pub(crate) type CacheKey = (Vec<usize>, Option<String>, Vec<usize>, ExprKind);

/// What a report's cache holds.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ReportCache {
    /// The digest of the design the values were calculated for.
    pub digest: String,
    /// When they were calculated (RFC 3339, UTC).
    pub calculated: String,
    /// The values the user chose for the parameters (the others had their
    /// defaults).
    pub chosen: IndexMap<String, Value>,
    /// The value of each parameter in the run.
    pub params: IndexMap<String, Value>,
    /// The list each parameter chosen from a list was chosen from.
    pub lists: IndexMap<String, Value>,
    /// The result of every expression, or the error in words.
    pub(crate) results: IndexMap<CacheKey, Result<Value, String>>,
    /// It was made for the design as it is now. Worked out when either
    /// changes, not each time a value is looked up.
    pub(crate) fits: bool,
}

/// The digest of a design, which a cache is made for.
pub fn design_digest(design: &ReportDesign) -> String {
    let hash = Sha256::digest(design.to_json().as_bytes());
    hash.iter().map(|b| format!("{b:02x}")).collect()
}

fn now() -> String {
    jiff::Timestamp::now()
        .round(jiff::Unit::Second)
        .map(|t| t.to_string())
        .unwrap_or_default()
}

// ------------------------------------------------------------ the values

fn kind_name(kind: ExprKind) -> &'static str {
    match kind {
        ExprKind::Visible => "visible",
        ExprKind::Enabled => "enabled",
        ExprKind::TextColour => "text_colour",
        ExprKind::FillColour => "fill_colour",
        ExprKind::Choices => "choices",
        ExprKind::Value => "value",
        ExprKind::Template => "template",
        ExprKind::Items => "items",
        ExprKind::Chart => "chart",
    }
}

fn kind_of(name: &str) -> Option<ExprKind> {
    ExprKind::ALL.into_iter().find(|k| kind_name(*k) == name)
}

/// A value as `cache.json` writes it.
pub fn value_to_json(value: &Value) -> Json {
    match value {
        Value::Null => Json::Null,
        Value::Bool(b) => Json::Bool(*b),
        Value::Int(n) => Json::from(*n),
        Value::Float(x) => match serde_json::Number::from_f64(*x) {
            Some(n) => Json::Number(n),
            None => json!({ "$float": x.to_string().to_lowercase() }),
        },
        Value::Str(s) => Json::String(s.clone()),
        Value::Date(d) => json!({ "$date": d.format("%Y-%m-%d").to_string() }),
        Value::DateTime(DateTimeValue::Naive(t)) => {
            json!({ "$datetime": t.format("%Y-%m-%dT%H:%M:%S%.f").to_string() })
        }
        Value::DateTime(DateTimeValue::Zoned(t)) => json!({ "$datetime": t.to_rfc3339() }),
        Value::Array(items) => {
            Json::Array(items.iter().map(|n| value_to_json(n.value())).collect())
        }
        Value::Object(fields) => {
            let object: Map<String, Json> = fields
                .iter()
                .map(|(k, n)| (k.clone(), value_to_json(n.value())))
                .collect();
            if fields.keys().any(|k| k.starts_with('$')) {
                json!({ "$object": object })
            } else {
                Json::Object(object)
            }
        }
        Value::Error(e) => json!({ "$error": { "kind": e.kind.code(), "message": e.message } }),
    }
}

/// A value from `cache.json`, or why it cannot be read.
pub fn value_from_json(json: &Json) -> Result<Value, String> {
    Ok(match json {
        Json::Null => Value::Null,
        Json::Bool(b) => Value::Bool(*b),
        Json::Number(n) => match n.as_i64() {
            Some(i) => Value::Int(i),
            None => Value::Float(n.as_f64().ok_or("a number that does not fit")?),
        },
        Json::String(s) => Value::Str(s.clone()),
        Json::Array(items) => Value::Array(
            items
                .iter()
                .map(|j| value_from_json(j).map(Node::new))
                .collect::<Result<_, _>>()?,
        ),
        Json::Object(object) => {
            let tag = (object.len() == 1)
                .then(|| object.iter().next())
                .flatten()
                .filter(|(k, _)| k.starts_with('$'));
            let text = |v: &Json| v.as_str().map(str::to_owned).ok_or("a tag of no text");
            match tag {
                Some((k, v)) if k == "$date" => {
                    Value::Date(text(v)?.parse::<NaiveDate>().map_err(|e| e.to_string())?)
                }
                Some((k, v)) if k == "$datetime" => {
                    let t = text(v)?;
                    match DateTime::parse_from_rfc3339(&t) {
                        Ok(z) => Value::DateTime(DateTimeValue::Zoned(z)),
                        Err(_) => Value::DateTime(DateTimeValue::Naive(
                            t.parse::<NaiveDateTime>().map_err(|e| e.to_string())?,
                        )),
                    }
                }
                Some((k, v)) if k == "$float" => {
                    Value::Float(text(v)?.parse::<f64>().map_err(|e| e.to_string())?)
                }
                Some((k, v)) if k == "$error" => {
                    let code = v.get("kind").and_then(Json::as_str).unwrap_or_default();
                    Value::Error(ErrorValue {
                        kind: ErrorKind::from_code(code)
                            .ok_or_else(|| format!("{code:?} is not an error"))?,
                        message: v
                            .get("message")
                            .and_then(Json::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                        origin: None,
                    })
                }
                Some((k, v)) if k == "$object" => {
                    let inner = v.as_object().ok_or("$object holds no object")?;
                    object_of(inner)?
                }
                Some((k, _)) => return Err(format!("{k} is not a tag this version knows")),
                None => object_of(object)?,
            }
        }
    })
}

fn object_of(object: &Map<String, Json>) -> Result<Value, String> {
    Ok(Value::Object(
        object
            .iter()
            .map(|(k, j)| value_from_json(j).map(|v| (k.clone(), Node::new(v))))
            .collect::<Result<_, _>>()?,
    ))
}

fn values_to_json(values: &IndexMap<String, Value>) -> Json {
    Json::Object(
        values
            .iter()
            .map(|(k, v)| (k.clone(), value_to_json(v)))
            .collect(),
    )
}

fn values_from_json(json: Option<&Json>) -> Result<IndexMap<String, Value>, String> {
    match json {
        None => Ok(IndexMap::new()),
        Some(Json::Object(object)) => object
            .iter()
            .map(|(k, j)| value_from_json(j).map(|v| (k.clone(), v)))
            .collect(),
        Some(_) => Err("parameters are an object".to_owned()),
    }
}

impl ReportCache {
    /// The text of `cache.json`: with `results` false, only the chosen
    /// parameters (*Keep report results in the file* off). The results are
    /// in a stable order, so an unchanged cache gives the same bytes.
    pub fn to_json(&self, results: bool) -> String {
        let mut out = Map::new();
        out.insert("version".to_owned(), Json::from(CACHE_VERSION));
        out.insert("digest".to_owned(), Json::from(self.digest.clone()));
        out.insert("chosen".to_owned(), values_to_json(&self.chosen));
        if results {
            out.insert("calculated".to_owned(), Json::from(self.calculated.clone()));
            out.insert("params".to_owned(), values_to_json(&self.params));
            out.insert("lists".to_owned(), values_to_json(&self.lists));
            let mut entries: Vec<(&CacheKey, &Result<Value, String>)> =
                self.results.iter().collect();
            entries.sort_by(|a, b| {
                let key = |k: &CacheKey| (k.0.clone(), k.1.clone(), k.2.clone(), kind_name(k.3));
                key(a.0).cmp(&key(b.0))
            });
            let results: Vec<Json> = entries
                .into_iter()
                .map(|((at, column, rows, kind), result)| {
                    let mut entry = Map::new();
                    entry.insert("at".to_owned(), json!(at));
                    if let Some(c) = column {
                        entry.insert("column".to_owned(), Json::from(c.clone()));
                    }
                    if !rows.is_empty() {
                        entry.insert("rows".to_owned(), json!(rows));
                    }
                    entry.insert("kind".to_owned(), Json::from(kind_name(*kind)));
                    match result {
                        Ok(v) => entry.insert("value".to_owned(), value_to_json(v)),
                        Err(e) => entry.insert("error".to_owned(), Json::from(e.clone())),
                    };
                    Json::Object(entry)
                })
                .collect();
            out.insert("results".to_owned(), Json::Array(results));
        }
        let mut text =
            serde_json::to_string_pretty(&Json::Object(out)).expect("JSON always serializes");
        text.push('\n');
        text
    }

    /// Reads `cache.json`, or says why it cannot be used.
    pub fn from_json(text: &str) -> Result<ReportCache, String> {
        let json: Json = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let version = json.get("version").and_then(Json::as_u64).unwrap_or(0);
        if version > CACHE_VERSION {
            return Err(format!(
                "it was written by a newer version (cache version {version})"
            ));
        }
        let text_of = |k: &str| {
            json.get(k)
                .and_then(Json::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let indices = |j: Option<&Json>| -> Result<Vec<usize>, String> {
            match j {
                None => Ok(Vec::new()),
                Some(Json::Array(items)) => items
                    .iter()
                    .map(|i| {
                        i.as_u64()
                            .map(|n| n as usize)
                            .ok_or_else(|| "a place is a whole number".to_owned())
                    })
                    .collect(),
                Some(_) => Err("a place is a list".to_owned()),
            }
        };
        let mut results = IndexMap::new();
        for entry in json
            .get("results")
            .and_then(Json::as_array)
            .into_iter()
            .flatten()
        {
            let kind = entry
                .get("kind")
                .and_then(Json::as_str)
                .and_then(kind_of)
                .ok_or("a result of no kind this version knows")?;
            let key = (
                indices(entry.get("at"))?,
                entry
                    .get("column")
                    .and_then(Json::as_str)
                    .map(str::to_owned),
                indices(entry.get("rows"))?,
                kind,
            );
            let result = match (entry.get("value"), entry.get("error")) {
                (_, Some(e)) => Err(e.as_str().unwrap_or_default().to_owned()),
                (Some(v), None) => Ok(value_from_json(v)?),
                (None, None) => Ok(Value::Null),
            };
            results.insert(key, result);
        }
        Ok(ReportCache {
            digest: text_of("digest"),
            calculated: text_of("calculated"),
            chosen: values_from_json(json.get("chosen"))?,
            params: values_from_json(json.get("params"))?,
            lists: values_from_json(json.get("lists"))?,
            results,
            fits: false,
        })
    }
}

impl Calc {
    /// The cache of `report`, if it was made for its design as it is now.
    pub fn report_cache(&self, report: &str) -> Option<&ReportCache> {
        self.caches.get(report).filter(|c| c.fits)
    }

    /// Works out again whether the cache of `report` fits its design.
    pub(crate) fn refit_cache(&mut self, report: &str) {
        let digest = self.reports.get(report).map(design_digest);
        if let Some(cache) = self.caches.get_mut(report) {
            cache.fits = digest.as_deref() == Some(cache.digest.as_str());
        }
    }

    /// The cache of `report` as it was read or last taken, whatever design
    /// it was made for.
    pub fn stored_report_cache(&self, report: &str) -> Option<&ReportCache> {
        self.caches.get(report)
    }

    /// Puts in the cache of a report as a workbook has it. The values the
    /// user chose are taken from it, if it was made for the design.
    pub fn set_report_cache(&mut self, report: &str, cache: ReportCache) {
        self.caches.insert(report.to_owned(), cache);
        self.refit_cache(report);
        if let Some(chosen) = self.report_cache(report).map(|c| c.chosen.clone()) {
            self.set_chosen_params(report, chosen);
        }
    }

    /// Starts `report` afresh, as `cliotree-cli` runs it (DESIGN.md, "Batch
    /// export from the command line"): its cache is gone, so nothing shown
    /// comes from an earlier run, and no value is chosen for its parameters,
    /// so each takes its default.
    pub fn forget_report_run(&mut self, report: &str) {
        self.caches.remove(report);
        self.set_chosen_params(report, indexmap::IndexMap::new());
    }

    /// What an expression of a report came to in the run its cache holds.
    pub fn cached_expression(
        &self,
        report: &str,
        at: &[usize],
        column: Option<&str>,
        rows: &[usize],
        kind: ExprKind,
    ) -> Option<&Result<Value, String>> {
        let key = (at.to_vec(), column.map(str::to_owned), rows.to_vec(), kind);
        self.report_cache(report)?.results.get(&key)
    }

    /// Takes the run of the report being shown into its cache, if every
    /// expression of it has its result. The time it was calculated is kept
    /// when nothing in it changed. Returns whether it was taken.
    pub fn take_report_run(&mut self) -> bool {
        let Some(report) = self.exprs.shown_report.clone() else {
            return false;
        };
        let Some(design) = self.reports.get(&report) else {
            return false;
        };
        if !self.trusted || self.exprs.resync || self.pending_expressions() > 0 {
            return false;
        }
        let mut results = IndexMap::new();
        for (key, state) in self.exprs.report_results(&report) {
            match state {
                Some(result) => {
                    results.insert(key, result);
                }
                // One that has not run: this is not a whole run.
                None => return false,
            }
        }
        let states = self.report_params(&report);
        let params: IndexMap<String, Value> = states
            .iter()
            .map(|s| (s.name.clone(), s.value.clone()))
            .collect();
        let lists: IndexMap<String, Value> = design
            .params
            .iter()
            .filter_map(|p| {
                let key = super::param_key(&report, &p.name, ExprKind::Choices);
                let result = self.exprs_result(&key)?;
                Some((p.name.clone(), result))
            })
            .collect();
        let mut cache = ReportCache {
            digest: design_digest(design),
            calculated: String::new(),
            chosen: self.params.get(&report).cloned().unwrap_or_default(),
            params,
            lists,
            results,
            fits: true,
        };
        let old = self.caches.get(&report);
        cache.calculated = match old {
            Some(old)
                if old.digest == cache.digest
                    && old.params == cache.params
                    && old.lists == cache.lists
                    && old.results == cache.results =>
            {
                old.calculated.clone()
            }
            _ => now(),
        };
        if old != Some(&cache) {
            self.caches.insert(report, cache);
        }
        true
    }

    /// The result an expression came to, if it ran without an error.
    fn exprs_result(&self, key: &super::ExprKey) -> Option<Value> {
        self.expression_state(key)?.result?.ok()
    }

    /// The cache of every report, by name, to be saved.
    pub fn report_caches(&self) -> impl Iterator<Item = (&str, &ReportCache)> {
        self.caches.iter().map(|(n, c)| (n.as_str(), c))
    }
}

impl super::control_exprs::ControlExprs {
    /// Every expression of the report `report` that has a place in its
    /// design, with its result (None if it has not run).
    pub(crate) fn report_results(
        &self,
        report: &str,
    ) -> Vec<(CacheKey, Option<Result<Value, String>>)> {
        self.entries()
            .filter(|(k, _)| k.tab == report && k.form == REPORT_FORM && !k.at.is_empty())
            .map(|(k, result)| {
                (
                    (k.at.clone(), k.column.clone(), k.rows.clone(), k.kind),
                    result,
                )
            })
            .collect()
    }
}
