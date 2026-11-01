//! The condition names (level 88) of a COBOL copybook on the Default form
//! (DESIGN.md, "COBOL copybooks"): the field is a dropdown of the names, and
//! still holds any other value.

use std::time::Duration;

use cliotree_app::ClioApp;
use cliotree_app::form::View;
use cliotree_core::copybook::CodePage;
use cliotree_core::{Path, Value};
use egui::Key;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

const COPYBOOK: &str = "\
       01  ACCOUNT-REC.
           05  ACCT-ID        PIC 9(4).
           05  STATE          PIC X.
               88  OPEN-STATE     VALUE 'O'.
               88  SHUT-STATE     VALUE 'C'.
";

const STATE: &str = "accounts.STATE";

fn settle(harness: &mut Harness<'_, ClioApp>) {
    assert!(
        harness
            .state_mut()
            .session
            .wait_idle(Duration::from_secs(60)),
        "the workbook did not settle"
    );
    harness.run();
}

fn state(harness: &Harness<'_, ClioApp>) -> Option<Value> {
    harness
        .state()
        .calc()
        .data("accounts")?
        .get(&"STATE".parse::<Path>().unwrap())
        .map(|n| n.value().clone())
}

fn shown(harness: &Harness<'_, ClioApp>) -> String {
    harness
        .get_by_label(STATE)
        .accesskit_node()
        .value()
        .unwrap_or_default()
}

/// The sample workbook with the copybook and a record of it, on the Default
/// form of the record's tab.
fn book(state: &str) -> Harness<'static, ClioApp> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 1000.0))
        .build_ui_state(|ui, app: &mut ClioApp| app.show(ui), ClioApp::sample());
    settle(&mut harness);
    {
        let app = harness.state_mut();
        let plan = app.plan_import("account.cpy", COPYBOOK.as_bytes()).unwrap();
        app.import(&plan).unwrap();
        let bytes = CodePage::Cp037.encode(&format!("0001{state}")).unwrap();
        let plan = app.plan_import("accounts.blk", &bytes).unwrap();
        app.import(&plan).unwrap();
        assert_eq!(app.sheet, "accounts");
        assert!(app.set_view(View::Form("Default".to_owned())));
    }
    settle(&mut harness);
    harness
}

#[test]
fn a_field_with_condition_names_is_a_dropdown_of_their_codes_and_names() {
    let mut harness = book("O");
    assert_eq!(shown(&harness), "O \u{2014} OPEN-STATE");

    harness.get_by_label(STATE).click();
    harness.run();
    harness.get_by_label("C \u{2014} SHUT-STATE").click();
    harness.run();
    settle(&mut harness);
    assert_eq!(state(&harness), Some(Value::Str("C".into())));
    assert_eq!(shown(&harness), "C \u{2014} SHUT-STATE");
}

#[test]
fn a_value_no_condition_names_is_kept_and_marked() {
    let harness = book("Z");
    assert!(
        shown(&harness).contains("Z (not in the list)"),
        "{}",
        shown(&harness)
    );
    assert_eq!(state(&harness), Some(Value::Str("Z".into())));
}

#[test]
fn other_takes_a_code_typed_in() {
    let mut harness = book("O");
    harness.get_by_label(STATE).click();
    harness.run();
    harness.get_by_label("Other...").click();
    harness.run();
    harness.run();
    // The field is a text box of the code now, with the focus.
    harness.key_combination_modifiers(egui::Modifiers::COMMAND, &[Key::A]);
    harness.get_by_label(STATE).type_text("Q");
    harness.run();
    harness.key_press(Key::Enter);
    harness.run();
    settle(&mut harness);
    assert_eq!(state(&harness), Some(Value::Str("Q".into())));
    assert!(shown(&harness).contains("Q (not in the list)"));
}

#[test]
fn without_allow_other_values_there_is_no_other() {
    let mut harness = book("O");
    {
        let app = harness.state_mut();
        let mut design = app.design_of("Default");
        let (at, control) = design
            .walk()
            .into_iter()
            .find(|(_, c)| c.bound_path() == Some("STATE".parse().unwrap()))
            .unwrap();
        assert!(control.allow_other, "the generated form allows other codes");
        design.get_mut(&at).unwrap().allow_other = false;
        app.session
            .calc_mut()
            .forms_mut()
            .set("accounts", "Default", design);
    }
    settle(&mut harness);
    harness.get_by_label(STATE).click();
    harness.run();
    assert!(harness.query_by_label("O \u{2014} OPEN-STATE").is_some());
    assert!(harness.query_by_label("Other...").is_none());
}
