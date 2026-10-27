//! Tests of the workbook settings (DESIGN.md, "Settings and limits"): the file,
//! the ceilings, the history and the trust digest.

use std::sync::Arc;
use std::time::Duration;

use cliotree_core::{json, jsonschema};

use crate::python::PythonSyntax;
use crate::{Calc, Ceilings, Meta, Setting, WorkbookSettings};

fn book() -> Calc {
    let mut calc = Calc::new(Arc::new(PythonSyntax));
    calc.add_sheet(
        "S",
        jsonschema::lower_str(r#"{"type": "object", "properties": {"a": {"type": "number"}}}"#)
            .unwrap(),
        json::parse(r#"{"a": 1}"#).unwrap().node,
    )
    .unwrap();
    calc.add_schema_sheet(
        "SSchema",
        json::parse(r#"{"type": "object", "properties": {"a": {"type": "number"}}}"#)
            .unwrap()
            .node,
    )
    .unwrap();
    calc.bind_schema("S", "SSchema").unwrap();
    calc.clear_history();
    calc
}

fn longer_formulas() -> WorkbookSettings {
    let mut settings = WorkbookSettings::default();
    settings.set(Setting::FormulaTimeLimit, 2_000);
    settings.set(Setting::MaxRange, 500);
    settings
}

fn meta() -> Meta {
    Meta {
        workbook_id: "id".to_owned(),
        app_version: "0.1.0".to_owned(),
        ..Meta::default()
    }
}

#[test]
fn default_settings_need_no_file_and_are_no_executable_content() {
    let calc = book();
    assert!(calc.settings().is_default());
    let package = calc.to_package(&meta()).unwrap();
    assert!(package.get("settings.json").is_none());
    assert!(calc.fingerprint().items.is_empty());
}

#[test]
fn settings_are_saved_with_only_what_differs_and_read_back() {
    let mut calc = book();
    calc.set_settings(longer_formulas());
    let package = calc.to_package(&meta()).unwrap();
    let text = std::str::from_utf8(package.get("settings.json").unwrap()).unwrap();
    assert_eq!(
        text,
        "{\n  \"formula_time_limit_ms\": 2000,\n  \"max_range_length\": 500\n}\n"
    );
    // After the modules, before what this version does not know.
    let paths: Vec<_> = package.entries().map(|(p, _)| p).collect();
    assert_eq!(paths.last(), Some(&"settings.json"));

    let opened = Calc::from_package(Arc::new(PythonSyntax), &package).unwrap();
    assert!(opened.warnings.is_empty(), "{:?}", opened.warnings);
    assert_eq!(opened.calc.settings(), &longer_formulas());
    assert!(opened.unknown.is_empty());
    // Reading is not an edit.
    assert!(!opened.calc.is_modified());
    assert_eq!(opened.calc.undo_label(), None);
    assert_eq!(opened.calc.to_package(&meta()).unwrap(), package);
}

#[test]
fn a_setting_this_version_does_not_know_is_kept() {
    let (settings, warnings) =
        WorkbookSettings::from_json("{\"query_cache_mb\": 64, \"cancel_grace_ms\": 0}");
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(settings.get(Setting::CancelGrace), 0);
    assert_eq!(
        settings.to_json().unwrap(),
        "{\n  \"cancel_grace_ms\": 0,\n  \"query_cache_mb\": 64\n}\n"
    );
}

#[test]
fn a_value_that_cannot_be_used_has_its_default_with_a_warning() {
    let (settings, warnings) = WorkbookSettings::from_json(
        "{\"formula_time_limit_ms\": \"long\", \"max_abandoned_threads\": 0, \
         \"recalc_time_limit_ms\": 1.5}",
    );
    assert!(settings.is_default());
    assert_eq!(warnings.len(), 3, "{warnings:?}");
    assert!(
        warnings[0].contains("formula_time_limit_ms"),
        "{warnings:?}"
    );
    assert!(warnings[1].contains("below the smallest"), "{warnings:?}");

    let (settings, warnings) = WorkbookSettings::from_json("[1, 2]");
    assert!(settings.is_default());
    assert!(warnings[0].contains("not a JSON object"), "{warnings:?}");
}

#[test]
fn a_file_with_settings_that_cannot_be_read_still_opens() {
    let calc = book();
    let mut package = calc.to_package(&meta()).unwrap();
    package
        .insert("settings.json", b"{ not json".to_vec())
        .unwrap();
    let opened = Calc::from_package(Arc::new(PythonSyntax), &package).unwrap();
    assert!(opened.calc.settings().is_default());
    assert!(
        opened.warnings.iter().any(|w| w.contains("settings.json")),
        "{:?}",
        opened.warnings
    );
}

#[test]
fn a_change_of_settings_is_one_undoable_entry() {
    let mut calc = book();
    calc.mark_saved();
    calc.set_settings(longer_formulas());
    assert!(calc.is_modified());
    assert_eq!(calc.undo_label(), Some("Change the workbook settings"));
    // The same settings again are no edit.
    calc.set_settings(longer_formulas());
    calc.undo().unwrap();
    assert!(calc.settings().is_default());
    assert!(!calc.is_modified());
    calc.redo().unwrap();
    assert_eq!(calc.settings(), &longer_formulas());
    assert_eq!(calc.redo_label(), None);
}

#[test]
fn settings_are_part_of_what_trust_covers() {
    let mut calc = book();
    let before = calc.fingerprint();
    calc.set_settings(longer_formulas());
    let after = calc.fingerprint();
    assert_ne!(before.hash, after.hash);
    assert!(after.items.contains_key("workbook settings"));
    let mut other = longer_formulas();
    other.set(Setting::MaxRange, 501);
    calc.set_settings(other);
    assert_ne!(
        calc.fingerprint().items["workbook settings"],
        after.items["workbook settings"]
    );
}

#[test]
fn a_ceiling_holds_a_setting_down() {
    let mut settings = WorkbookSettings::default();
    settings.set(Setting::FormulaTimeLimit, 3_600_000);
    let ceilings = Ceilings::default();
    let effective = settings.effective(&ceilings);
    assert_eq!(effective.get(Setting::FormulaTimeLimit), 60_000);
    assert_eq!(
        settings.held_down(&ceilings),
        [(Setting::FormulaTimeLimit, 3_600_000, 60_000)]
    );
    let mut lower = Ceilings::default();
    lower.set(Setting::MaxRange, 1_000);
    let effective = WorkbookSettings::default().effective(&lower);
    assert_eq!(effective.python_settings().max_range, 1_000);
    let eval = effective.eval_settings();
    assert_eq!(eval.formula_limit, Some(Duration::from_secs(1)));
    assert_eq!(eval.procedure_limit, Some(Duration::from_secs(10)));
    assert_eq!(eval.recalc_limit, Some(Duration::from_secs(10)));
    assert_eq!(eval.grace, Duration::from_millis(500));
    assert_eq!(eval.max_abandoned, 20);
}

#[test]
fn the_defaults_are_the_ones_the_design_gives() {
    let effective = WorkbookSettings::default().effective(&Ceilings::default());
    for (setting, value, ceiling) in [
        (Setting::FormulaTimeLimit, 1_000, 60_000),
        (Setting::ProcedureTimeLimit, 10_000, 300_000),
        (Setting::RecalcTimeLimit, 10_000, 600_000),
        (Setting::CancelGrace, 500, 10_000),
        (Setting::MaxAbandoned, 20, 100),
        (Setting::MaxRange, 1_000_000, 1_000_000),
        (Setting::SourceTimeout, 30_000, 300_000),
        (Setting::MaxResponse, 16 << 20, 256 << 20),
    ] {
        assert_eq!(effective.get(setting), value, "{setting:?}");
        assert_eq!(Ceilings::default().get(setting), ceiling, "{setting:?}");
        assert_eq!(Setting::from_key(setting.key()), Some(setting));
    }
}
