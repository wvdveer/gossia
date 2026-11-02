//! Tests of report tabs (DESIGN.md, "Reports", "Managing tabs"): their place
//! among the tabs, the history, saving and the trust digest.

use std::sync::Arc;

use cliotree_core::form::{Control, ControlKind};
use cliotree_core::report::{ReportDesign, ReportKind};
use cliotree_core::{json, jsonschema};

use crate::fake::FakeEngine;
use crate::{Calc, EditError, Meta, TabKind};

const SCHEMA: &str = r#"{"type": "object", "properties": {"n": {"type": "integer"}}}"#;

/// Tabs `A`, `B` and `C`, with no history.
fn book() -> Calc {
    let mut calc = Calc::new(Arc::new(FakeEngine::new()));
    for name in ["A", "B", "C"] {
        calc.add_sheet(
            name,
            jsonschema::lower_str(SCHEMA).unwrap(),
            json::parse(r#"{"n": 1}"#).unwrap().node,
        )
        .unwrap();
    }
    calc.clear_history();
    calc
}

fn tabs(calc: &Calc) -> Vec<String> {
    calc.tab_names()
}

fn design_with(value: &str) -> ReportDesign {
    let mut design = ReportDesign::new(ReportKind::SinglePage);
    let mut field = Control::new(ControlKind::Field);
    field.value = Some(value.into());
    design.layout.controls.push(field);
    design
}

#[test]
fn a_new_report_goes_at_the_end_and_undo_takes_it_out() {
    let mut calc = book();
    calc.create_report("Sales", design_with("1")).unwrap();
    assert_eq!(tabs(&calc), ["A", "B", "C", "Sales"]);
    assert_eq!(calc.tab_kind("Sales"), Some(TabKind::Report));
    assert_eq!(calc.tab_kind("A"), Some(TabKind::Data));
    // A report is not a data tab.
    assert!(!calc.sheet_names().any(|n| n == "Sales"));
    assert_eq!(calc.undo_label(), Some("New report Sales"));
    calc.undo().unwrap();
    assert_eq!(tabs(&calc), ["A", "B", "C"]);
    assert_eq!(calc.take_removed(), ["Sales"]);
    calc.redo().unwrap();
    assert_eq!(tabs(&calc), ["A", "B", "C", "Sales"]);
    assert_eq!(calc.report("Sales"), Some(&design_with("1")));
}

#[test]
fn a_report_name_is_checked_as_any_tab_name() {
    let mut calc = book();
    assert!(matches!(
        calc.create_report("b", design_with("1")),
        Err(EditError::BadName(_))
    ));
    assert!(matches!(
        calc.create_report("not a name", design_with("1")),
        Err(EditError::BadName(_))
    ));
    calc.create_report("Sales", design_with("1")).unwrap();
    // ...and a data tab cannot take a report's name either.
    assert!(calc.tab_name_problem("SALES", None).is_some());
    assert!(calc.rename_sheet("A", "sales").is_err());
}

#[test]
fn a_duplicate_goes_after_its_report_with_the_next_free_number() {
    let mut calc = book();
    calc.create_report("Sales", design_with("1")).unwrap();
    calc.move_sheet("Sales", 1).unwrap();
    assert_eq!(tabs(&calc), ["A", "Sales", "B", "C"]);
    assert_eq!(calc.duplicate_report("Sales").unwrap(), "Sales_2");
    assert_eq!(calc.duplicate_report("Sales").unwrap(), "Sales_3");
    assert_eq!(tabs(&calc), ["A", "Sales", "Sales_3", "Sales_2", "B", "C"]);
    assert_eq!(calc.report("Sales_2"), calc.report("Sales"));
    assert_eq!(calc.undo_label(), Some("Duplicate report Sales"));
    calc.undo().unwrap();
    calc.undo().unwrap();
    assert_eq!(tabs(&calc), ["A", "Sales", "B", "C"]);
}

#[test]
fn renaming_a_report_keeps_its_place() {
    let mut calc = book();
    calc.create_report("Sales", design_with("1")).unwrap();
    calc.move_sheet("Sales", 0).unwrap();
    calc.rename_sheet("Sales", "Revenue").unwrap();
    assert_eq!(tabs(&calc), ["Revenue", "A", "B", "C"]);
    assert_eq!(
        calc.take_renames(),
        [("Sales".to_owned(), "Revenue".to_owned())]
    );
    assert_eq!(calc.undo_label(), Some("Rename tab Sales to Revenue"));
    calc.undo().unwrap();
    assert_eq!(tabs(&calc), ["Sales", "A", "B", "C"]);
    // A data tab renamed among reports keeps its place too.
    calc.rename_sheet("B", "Bee").unwrap();
    assert_eq!(tabs(&calc), ["Sales", "A", "Bee", "C"]);
}

#[test]
fn reports_and_data_tabs_move_among_each_other() {
    let mut calc = book();
    calc.create_report("Sales", design_with("1")).unwrap();
    calc.move_sheet("C", 0).unwrap();
    assert_eq!(tabs(&calc), ["C", "A", "B", "Sales"]);
    // The data tabs keep the same order among themselves.
    assert_eq!(calc.sheet_names().collect::<Vec<_>>(), ["C", "A", "B"]);
    calc.move_sheet("Sales", 1).unwrap();
    assert_eq!(tabs(&calc), ["C", "Sales", "A", "B"]);
    calc.undo().unwrap();
    assert_eq!(tabs(&calc), ["C", "A", "B", "Sales"]);
    // A tab made now goes at the end.
    calc.add_sheet(
        "D",
        jsonschema::lower_str(SCHEMA).unwrap(),
        json::parse(r#"{"n": 1}"#).unwrap().node,
    )
    .unwrap();
    assert_eq!(tabs(&calc), ["C", "A", "B", "Sales", "D"]);
}

#[test]
fn deleting_a_tab_and_undoing_puts_it_back_in_its_place() {
    let mut calc = book();
    calc.create_report("Sales", design_with("1")).unwrap();
    calc.move_sheet("Sales", 1).unwrap();
    assert_eq!(calc.delete_problem("Sales"), None);
    calc.delete_sheet("Sales").unwrap();
    assert_eq!(calc.undo_label(), Some("Delete tab Sales"));
    assert_eq!(tabs(&calc), ["A", "B", "C"]);
    calc.undo().unwrap();
    assert_eq!(tabs(&calc), ["A", "Sales", "B", "C"]);
    // A data tab after a report comes back after it.
    calc.delete_sheet("B").unwrap();
    assert_eq!(tabs(&calc), ["A", "Sales", "C"]);
    calc.undo().unwrap();
    assert_eq!(tabs(&calc), ["A", "Sales", "B", "C"]);
}

#[test]
fn a_new_design_is_not_an_edit() {
    let mut calc = book();
    calc.create_report("Sales", design_with("1")).unwrap();
    let entries = calc.history_len().0;
    let forms = calc.forms_version();
    calc.set_report("Sales", design_with("2")).unwrap();
    assert_eq!(calc.history_len().0, entries);
    assert!(calc.forms_version() > forms);
    assert_eq!(calc.report("Sales"), Some(&design_with("2")));
    assert!(calc.set_report("Nope", design_with("2")).is_err());
}

#[test]
fn report_expressions_are_in_the_trust_digest() {
    let mut calc = book();
    let before = calc.fingerprint();
    let mut fixed = ReportDesign::new(ReportKind::SinglePage);
    let mut label = Control::new(ControlKind::Label);
    label.text = Some("Sales by month".into());
    label.fill_colour = Some("#ffeecc".into());
    fixed.layout.controls.push(label);
    calc.create_report("Fixed", fixed).unwrap();
    // Fixed text and a fixed colour run nothing.
    assert_eq!(calc.fingerprint(), before);
    calc.create_report("Sales", design_with("SUM(A.n)"))
        .unwrap();
    let with = calc.fingerprint();
    assert_ne!(with, before);
    calc.set_report("Sales", design_with("SUM(B.n)")).unwrap();
    assert_ne!(calc.fingerprint(), with);
}

/// A workbook that can be saved: a schema tab and a data tab.
fn savable() -> Calc {
    let mut calc = Calc::new(Arc::new(FakeEngine::new()));
    calc.create_json_schema_tab("Schema", json::parse(SCHEMA).unwrap().node)
        .unwrap();
    calc.create_data_tab(
        "Data",
        "Schema",
        json::parse(r#"{"n": 1}"#).unwrap().node,
        None,
    )
    .unwrap();
    calc
}

fn meta() -> Meta {
    Meta {
        workbook_id: "0123456789abcdef".to_owned(),
        app_version: "0.2.0".to_owned(),
        ..Meta::default()
    }
}

#[test]
fn reports_are_saved_in_their_place_and_read_back() {
    let mut calc = savable();
    let plain = calc.to_package(&meta()).unwrap();
    let manifest = String::from_utf8(plain.get("manifest.json").unwrap().to_vec()).unwrap();
    // Without reports the file is as version 1 wrote it.
    assert!(
        manifest.contains("\"format_version\": \"1.0\""),
        "{manifest}"
    );

    calc.create_report("Sales", design_with("SUM(Data.n)"))
        .unwrap();
    calc.move_sheet("Sales", 1).unwrap();
    let package = calc.to_package(&meta()).unwrap();
    let manifest = String::from_utf8(package.get("manifest.json").unwrap().to_vec()).unwrap();
    assert!(
        manifest.contains("\"format_version\": \"1.1\""),
        "{manifest}"
    );
    assert!(manifest.contains("\"kind\": \"report\""), "{manifest}");
    let design =
        String::from_utf8(package.get("reports/Sales/design.json").unwrap().to_vec()).unwrap();
    assert!(design.contains("SUM(Data.n)"));

    let opened = Calc::from_package(Arc::new(FakeEngine::new()), &package).unwrap();
    assert!(opened.warnings.is_empty(), "{:?}", opened.warnings);
    assert!(opened.unknown.is_empty());
    assert_eq!(opened.calc.tab_names(), ["Schema", "Sales", "Data"]);
    assert_eq!(opened.calc.report("Sales"), calc.report("Sales"));
    // Saving again gives the same bytes.
    let again = opened.calc.to_package(&meta()).unwrap();
    assert_eq!(
        again.entries().collect::<Vec<_>>(),
        package.entries().collect::<Vec<_>>()
    );
}

#[test]
fn a_report_that_cannot_be_read_is_left_out_with_a_warning() {
    let mut calc = savable();
    calc.create_report("Sales", design_with("1")).unwrap();
    let package = calc.to_package(&meta()).unwrap();
    let mut broken = crate::Package::new();
    for (path, bytes) in package.entries() {
        let bytes = if path == "reports/Sales/design.json" {
            br#"{"version": 1, "kind": "poster", "layout": {"version": 1, "controls": []}}"#
                .to_vec()
        } else {
            bytes.to_vec()
        };
        broken.insert(path, bytes).unwrap();
    }
    let opened = Calc::from_package(Arc::new(FakeEngine::new()), &broken).unwrap();
    assert_eq!(opened.calc.tab_names(), ["Schema", "Data"]);
    assert!(
        opened.warnings.iter().any(|w| w.contains("report Sales")),
        "{:?}",
        opened.warnings
    );
    // Its file is kept as it came.
    assert!(
        opened
            .unknown
            .iter()
            .any(|(p, _)| p == "reports/Sales/design.json")
    );
}
