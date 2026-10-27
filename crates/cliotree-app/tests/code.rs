//! The code editor (DESIGN.md, "Python modules"), driven the way a user does.
//! The tests run headlessly, where the editor's second window is drawn inside
//! the main one.

use std::time::Duration;

use cliotree_app::ClioApp;
use cliotree_app::code::{PAUSE, TITLE};
use cliotree_calc::ModuleStatus;
use egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::{Queryable, by};

const TAX: &str = "def vat(x):\n    return x * 0.21\n\n\ndef half(x):\n    return x / 2\n";

fn window_with(modules: &[(&str, &str)]) -> Harness<'static, ClioApp> {
    let mut app = ClioApp::sample();
    for (name, source) in modules {
        app.session.calc_mut().add_module(name, source).unwrap();
    }
    app.session.calc_mut().clear_history();
    // The test clock runs a quarter of a second a step: typing pauses only
    // when a test says so.
    app.code.pause = 3600.0;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1300.0, 900.0))
        .build_ui_state(|ui, app: &mut ClioApp| app.show(ui), app);
    settle(&mut harness);
    harness
}

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

fn click(harness: &mut Harness<'_, ClioApp>, label: &str) {
    harness.get_by_label(label).click();
    harness.run();
}

/// Clicks the editor's menu of that name: the main window has one too, drawn
/// first.
fn menu(harness: &mut Harness<'_, ClioApp>, label: &str) {
    harness.get_all_by_label(label).last().unwrap().click();
    harness.run();
}

fn open_editor(harness: &mut Harness<'_, ClioApp>) {
    harness.key_combination_modifiers(Modifiers::ALT, &[Key::F11]);
    harness.run();
    harness.get_by_label(TITLE);
}

fn code(harness: &Harness<'_, ClioApp>) -> String {
    harness.get_by_label("Code").value().unwrap_or_default()
}

fn modules(harness: &Harness<'_, ClioApp>) -> Vec<String> {
    harness
        .state()
        .calc()
        .module_names()
        .map(str::to_owned)
        .collect()
}

fn source(harness: &Harness<'_, ClioApp>, module: &str) -> String {
    harness
        .state()
        .calc()
        .module_source(module)
        .unwrap_or_default()
        .to_owned()
}

#[test]
fn alt_f11_opens_the_editor_and_insert_module_makes_module1() {
    let mut harness = window_with(&[]);
    harness.get_by_label("Tools").click();
    harness.run();
    harness.get(by().label_contains("Code editor"));
    harness.key_press(Key::Escape);
    harness.run();

    open_editor(&mut harness);
    harness.get_by_label("No modules yet");
    click(&mut harness, "Insert");
    click(&mut harness, "Module");
    assert_eq!(modules(&harness), ["Module1"]);
    assert_eq!(
        harness.state().calc().undo_label(),
        Some("Insert module Module1")
    );
    assert_eq!(code(&harness), "");
    assert!(
        harness.get_by_label("Code").is_focused(),
        "ready to type in"
    );

    // Close and return: Alt+F11 shows it again with the module still open.
    menu(&mut harness, "File");
    click(&mut harness, "Close and return to ClioTree");
    assert!(harness.query_by_label(TITLE).is_none());
    open_editor(&mut harness);
    assert_eq!(code(&harness), "");
}

/// Typing goes to the workbook after a pause, as one entry; Enter keeps and
/// adds indentation; undo in the editor is the workbook's.
#[test]
fn a_burst_of_typing_is_one_undoable_edit_of_the_module() {
    let mut harness = window_with(&[]);
    open_editor(&mut harness);
    click(&mut harness, "Insert");
    click(&mut harness, "Module");
    harness.get_by_label("Code").type_text("def double(x):");
    harness.run();
    harness.key_press(Key::Enter);
    harness.run();
    harness.get_by_label("Code").type_text("return x * 2");
    harness.run();
    let typed = "def double(x):\n    return x * 2";
    assert_eq!(code(&harness), typed);
    assert_eq!(
        source(&harness, "Module1"),
        "",
        "not in the workbook while typing"
    );

    // Once typing has paused, it is in the workbook and the module loads.
    let at = harness.state().code.tabs[0].typed_at.expect("typed");
    harness.state_mut().code.pause = PAUSE;
    harness.state_mut().sync_code(at + PAUSE);
    settle(&mut harness);
    assert_eq!(source(&harness, "Module1"), typed);
    assert_eq!(
        harness.state().calc().module_functions("Module1"),
        ["double"]
    );
    assert_eq!(
        harness.state().calc().module_status("Module1"),
        Some(&ModuleStatus::Loaded)
    );
    harness.get_by_label("Module1: loaded");
    assert_eq!(
        harness.state().calc().undo_label(),
        Some("Edit module Module1")
    );

    // Ctrl+Z in the editor undoes the whole burst, and Ctrl+Y puts it back.
    harness.get_by_label("Code").focus();
    harness.run();
    harness.key_combination_modifiers(Modifiers::COMMAND, &[Key::Z]);
    harness.run();
    assert_eq!(source(&harness, "Module1"), "");
    assert_eq!(code(&harness), "");
    harness.key_combination_modifiers(Modifiers::COMMAND, &[Key::Y]);
    harness.run();
    assert_eq!(code(&harness), typed);

    // Typing that has not paused yet is the first thing undone.
    harness.get_by_label("Code").type_text("x");
    harness.run();
    assert!(harness.state().code.has_typing());
    harness.key_combination_modifiers(Modifiers::COMMAND, &[Key::Z]);
    harness.run();
    assert_eq!(code(&harness), typed);
    assert!(!harness.state().code.has_typing());
}

#[test]
fn tab_and_shift_tab_indent_and_unindent() {
    let mut harness = window_with(&[]);
    open_editor(&mut harness);
    click(&mut harness, "Insert");
    click(&mut harness, "Module");
    harness.key_press(Key::Tab);
    harness.run();
    harness.get_by_label("Code").type_text("x = 1");
    harness.run();
    assert_eq!(code(&harness), "    x = 1");
    assert!(
        harness.get_by_label("Code").is_focused(),
        "Tab stays in the code"
    );
    harness.key_combination_modifiers(Modifiers::SHIFT, &[Key::Tab]);
    harness.run();
    assert_eq!(code(&harness), "x = 1");
}

/// The procedure box names the function the cursor is in.
#[test]
fn the_procedure_box_follows_the_cursor_and_jumps() {
    let mut harness = window_with(&[("Tax", TAX)]);
    open_editor(&mut harness);
    assert_eq!(code(&harness), TAX);
    let procedure = |h: &Harness<'_, ClioApp>| h.get_by_label("Procedure box").value();
    assert_eq!(procedure(&harness).as_deref(), Some("vat"));
    harness.state_mut().code.jump = Some(4);
    harness.run();
    assert_eq!(procedure(&harness).as_deref(), Some("half"));
}

#[test]
fn modules_are_renamed_and_removed_from_the_explorer_and_undo_brings_them_back() {
    let mut harness = window_with(&[("Tax", TAX)]);
    open_editor(&mut harness);
    // The first "Tax" is the explorer's, the second the tab's.
    harness
        .get_all_by_label("Tax")
        .next()
        .unwrap()
        .click_secondary();
    harness.run();
    click(&mut harness, "Rename...");
    harness.get_by_label("Module name").type_text("Orders");
    harness.run();
    harness.get_by_label("Orders is the name of a tab, and a module cannot share it");
    harness.state_mut().code.rename.as_mut().unwrap().name = "Vat".to_owned();
    harness.run();
    click(&mut harness, "Rename");
    assert_eq!(modules(&harness), ["Vat"]);
    assert_eq!(
        harness.state().code.tabs[0].module,
        "Vat",
        "the tab follows"
    );

    harness
        .get_all_by_label("Vat")
        .next()
        .unwrap()
        .click_secondary();
    harness.run();
    click(&mut harness, "Remove...");
    click(&mut harness, "Remove");
    assert!(modules(&harness).is_empty());
    assert!(harness.state().code.tabs.is_empty());
    harness.get_by_label("No module is open. Insert > Module makes one.");

    menu(&mut harness, "Edit");
    click(&mut harness, "Undo Remove module Vat");
    assert_eq!(modules(&harness), ["Vat"]);
    assert_eq!(source(&harness, "Vat"), TAX);
    harness.key_combination_modifiers(Modifiers::COMMAND, &[Key::Z]);
    harness.run();
    assert_eq!(modules(&harness), ["Tax"]);
    assert_eq!(
        harness.state().code.tabs.len(),
        0,
        "a removed tab stays closed"
    );
}

#[test]
fn the_status_line_says_what_loading_came_to() {
    let mut harness = window_with(&[("Bad", "x = 1 / 0\n")]);
    open_editor(&mut harness);
    settle(&mut harness);
    let why = match harness.state().calc().module_status("Bad") {
        Some(ModuleStatus::Failed(why)) => why.clone(),
        other => panic!("Bad loaded: {other:?}"),
    };
    harness.get_by_label(&format!("Bad failed to load: {why}"));

    let mut harness = window_with(&[("Tax", TAX)]);
    harness.state_mut().session.calc_mut().set_trusted(false);
    open_editor(&mut harness);
    harness.get_by_label("Tax: not loaded, because the workbook is not trusted");
    // It can still be edited.
    assert!(
        harness.get_by_label("Code").is_focused(),
        "ready to type in"
    );
    harness.get_by_label("Code").type_text("# note");
    harness.run();
    harness.state_mut().commit_code();
    assert!(source(&harness, "Tax").starts_with("# note"));
}
