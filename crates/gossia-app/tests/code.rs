//! The code editor (DESIGN.md, "Python modules"), driven the way a user does.
//! The tests run headlessly, where the editor's second window is drawn inside
//! the main one.

use std::time::Duration;

use egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::{Queryable, by};
use gossia_app::GossiaApp;
use gossia_app::code::{CodeItem, PAUSE, title};
use gossia_calc::ModuleStatus;

const TAX: &str = "def vat(x):\n    return x * 0.21\n\n\ndef half(x):\n    return x / 2\n";

fn window_with(modules: &[(&str, &str)]) -> Harness<'static, GossiaApp> {
    let mut app = GossiaApp::sample();
    for (name, source) in modules {
        app.session.calc_mut().add_module(name, source).unwrap();
    }
    app.session.calc_mut().clear_history();
    // The test clock runs a quarter of a second a step: typing pauses only
    // when a test says so.
    app.code.pause = 3600.0;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1300.0, 900.0))
        .build_ui_state(|ui, app: &mut GossiaApp| app.show(ui), app);
    settle(&mut harness);
    harness
}

fn settle(harness: &mut Harness<'_, GossiaApp>) {
    assert!(
        harness
            .state_mut()
            .session
            .wait_idle(Duration::from_secs(60)),
        "the workbook did not settle"
    );
    harness.run();
}

fn click(harness: &mut Harness<'_, GossiaApp>, label: &str) {
    harness.get_by_label(label).click();
    harness.run();
}

/// Clicks the editor's menu of that name: the main window has one too, drawn
/// first.
fn menu(harness: &mut Harness<'_, GossiaApp>, label: &str) {
    harness.get_all_by_label(label).last().unwrap().click();
    harness.run();
}

fn open_editor(harness: &mut Harness<'_, GossiaApp>) {
    harness.key_combination_modifiers(Modifiers::ALT, &[Key::F11]);
    harness.run();
    harness.get_by_label(title());
}

fn code(harness: &Harness<'_, GossiaApp>) -> String {
    harness.get_by_label("Code").value().unwrap_or_default()
}

fn modules(harness: &Harness<'_, GossiaApp>) -> Vec<String> {
    harness
        .state()
        .calc()
        .module_names()
        .map(str::to_owned)
        .collect()
}

fn source(harness: &Harness<'_, GossiaApp>, module: &str) -> String {
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
    click(&mut harness, "Close and return to Gossia");
    assert!(harness.query_by_label(title()).is_none());
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
    let procedure = |h: &Harness<'_, GossiaApp>| h.get_by_label("Procedure box").value();
    assert_eq!(procedure(&harness).as_deref(), Some("vat"));
    harness.state_mut().code.jump = Some((4, 0));
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
        harness.state().code.tabs[0].item,
        CodeItem::Module("Vat".to_owned()),
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

/// The title of the tab shown.
fn shown(harness: &Harness<'_, GossiaApp>) -> String {
    harness
        .state()
        .code
        .current_tab()
        .map(|t| t.item.title())
        .unwrap_or_default()
}

/// *Debug > Compile* lists the problems of the module shown, each a link to
/// its place, and the list follows the module as it is fixed.
#[test]
fn compile_lists_the_problems_and_a_click_goes_to_the_line() {
    let mut harness = window_with(&[("Tax", "import math\nimport os\n\ndef f():\n    return 1\n")]);
    open_editor(&mut harness);
    assert_eq!(shown(&harness), "Tax");
    menu(&mut harness, "Debug");
    click(&mut harness, "Compile Tax");
    settle(&mut harness);
    harness.get_by_label("Compile Tax: 1 problem");
    harness
        .get(by().label_contains("Line 2, column 8: no module named os"))
        .click();
    harness.run();
    harness.run();
    assert!(harness.get_by_label("Code").is_focused());

    harness
        .state_mut()
        .session
        .calc_mut()
        .set_module_source("Tax", "import math\n\ndef f():\n    return 1\n")
        .unwrap();
    settle(&mut harness);
    harness.get_by_label("Compile Tax: no problems");
    click(&mut harness, "Close the list");
    assert!(harness.state().code.compiled.is_none());
}

/// An import of matplotlib fails with the advice to use Gossia pyplot, and
/// the status line and the list of *Compile* offer to change the import; the
/// change is one entry to undo.
#[test]
fn an_import_of_matplotlib_is_changed_to_gossia_pyplot_with_a_button() {
    let fix = "Change to import gossia.pyplot as plt";
    let (before, after) = (
        "import matplotlib.pyplot as plt\n",
        "import gossia.pyplot as plt\n",
    );
    let mut harness = window_with(&[("Plots", before)]);
    open_editor(&mut harness);
    settle(&mut harness);
    // On the status line, under the load error.
    click(&mut harness, fix);
    settle(&mut harness);
    assert_eq!(source(&harness, "Plots"), after);
    assert_eq!(
        harness.state().calc().module_status("Plots"),
        Some(&ModuleStatus::Loaded)
    );
    harness.run();
    assert_eq!(code(&harness), after);
    assert!(harness.query_by_label(fix).is_none());
    harness.state_mut().undo();
    settle(&mut harness);
    assert_eq!(source(&harness, "Plots"), before);

    // In the list of Compile, beside the problem.
    menu(&mut harness, "Debug");
    click(&mut harness, "Compile Plots");
    settle(&mut harness);
    harness.get_by_label("Compile Plots: 1 problem");
    assert_eq!(harness.get_all_by_label(fix).count(), 2);
    harness.get_all_by_label(fix).next().unwrap().click();
    harness.run();
    settle(&mut harness);
    assert_eq!(source(&harness, "Plots"), after);
    harness.get_by_label("Compile Plots: no problems");
}

/// A module goes out as a `.py` file and comes back in as a new module, named
/// after the file, with a number while the name is taken.
#[test]
fn a_module_is_exported_to_a_py_file_and_imported_from_one() {
    let mut harness = window_with(&[("Tax", TAX)]);
    harness.state_mut().files.browse = false;
    open_editor(&mut harness);
    let path = std::env::temp_dir().join(format!(
        "export_tax_{}_{}.py",
        std::process::id(),
        gossia_app::workbook_file::new_workbook_id()
    ));
    menu(&mut harness, "File");
    click(&mut harness, "Export Tax...");
    harness.state_mut().code.files.dialog.as_mut().unwrap().text = path.display().to_string();
    harness.run();
    click(&mut harness, "Export");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), TAX);

    menu(&mut harness, "File");
    click(&mut harness, "Import module...");
    harness.state_mut().code.files.dialog.as_mut().unwrap().text = path.display().to_string();
    harness.run();
    click(&mut harness, "Import");
    let stem = path
        .file_stem()
        .unwrap()
        .to_string_lossy()
        .replace('-', "_");
    assert_eq!(modules(&harness), ["Tax".to_owned(), stem.clone()]);
    assert_eq!(source(&harness, &stem), TAX);
    assert_eq!(shown(&harness), stem, "the new module is open");
    assert_eq!(
        harness.state().calc().undo_label(),
        Some(format!("Insert module {stem}").as_str())
    );

    // Again: the name is taken now.
    let name = harness.state_mut().import_module_from(&path).unwrap();
    assert_eq!(name, format!("{stem}2"));
    std::fs::remove_file(&path).unwrap();
    // A file that is not there says so, and nothing is added.
    assert!(harness.state_mut().import_module_from(&path).is_err());
    assert_eq!(modules(&harness).len(), 3);
}

/// The *Forms* branch shows the design of a customised form as JSON: a text
/// that is a design becomes the form's design, one that is not leaves the
/// form as it was and says why.
#[test]
fn a_customised_form_is_shown_as_json_and_its_text_becomes_its_design() {
    let mut harness = window_with(&[]);
    open_editor(&mut harness);
    harness.get_by_label("No customised forms");
    let spec = harness.state().calc().spec("Orders").unwrap().clone();
    let design = gossia_core::form::generate(&spec);
    harness
        .state_mut()
        .session
        .calc_mut()
        .forms_mut()
        .set("Orders", "Entry", design.clone());
    harness.run();
    click(&mut harness, "Entry");
    assert_eq!(shown(&harness), "Orders: Entry");
    assert_eq!(code(&harness), design.to_json());

    let set_text = |harness: &mut Harness<'_, GossiaApp>, text: &str| {
        let app = harness.state_mut();
        let current = app.code.current;
        let tab = &mut app.code.tabs[current];
        tab.text = text.to_owned();
        tab.typed_at = Some(0.0);
        app.commit_code();
        harness.run();
    };
    set_text(&mut harness, "{\"version\": 1");
    harness.get(by().label_contains("Form Entry of Orders keeps its design until this is fixed"));
    assert_eq!(
        harness.state().calc().forms().get("Orders", "Entry"),
        Some(&design)
    );
    assert_eq!(code(&harness), "{\"version\": 1", "the text stays as typed");

    let mut titled = design.clone();
    titled.title = Some("New order".to_owned());
    set_text(&mut harness, &titled.to_json());
    assert_eq!(
        harness.state().calc().forms().get("Orders", "Entry"),
        Some(&titled)
    );
    harness.get_by_label("Form Entry of Orders: this is its design");
}

/// The Immediate window runs an expression on the evaluation thread, with
/// `this` at the node selected in the workbook, and Up brings it back.
#[test]
fn the_immediate_window_shows_a_result_and_recalls_what_was_typed() {
    let mut harness = window_with(&[("Tax", TAX)]);
    open_editor(&mut harness);
    harness.get_by_label("this is Orders");
    harness.get_by_label("Immediate expression").click();
    harness.run();
    harness
        .get_by_label("Immediate expression")
        .type_text("Tax.half(7)");
    harness.key_press(Key::Enter);
    harness.run();
    settle(&mut harness);
    harness.get_by_label("> Tax.half(7)");
    harness.get_by_label("3.5");
    harness
        .get_by_label("Immediate expression")
        .type_text("1 / 0");
    harness.key_press(Key::Enter);
    harness.run();
    settle(&mut harness);
    harness.get(by().label_contains("#DIV/0!"));
    assert_eq!(harness.state().user.immediate, ["Tax.half(7)", "1 / 0"]);

    harness.key_press(Key::ArrowUp);
    harness.run();
    harness.key_press(Key::ArrowUp);
    harness.run();
    assert_eq!(harness.state().code.immediate.input, "Tax.half(7)");
    harness.key_press(Key::ArrowDown);
    harness.run();
    harness.key_press(Key::ArrowDown);
    harness.run();
    assert_eq!(harness.state().code.immediate.input, "");

    // Not in a workbook that is not trusted.
    harness.state_mut().session.calc_mut().set_trusted(false);
    harness.run();
    harness.get_by_label(
        "The Immediate window runs Python, so it works once the workbook is trusted.",
    );
}

/// Moving the pointer over the Immediate window redraws it, and it kept
/// shrinking a little at each redraw: its height is the user's to set.
#[test]
fn the_immediate_window_keeps_its_height_while_the_pointer_moves_over_it() {
    let mut harness = window_with(&[("Tax", TAX)]);
    open_editor(&mut harness);
    let top = |h: &Harness<'_, GossiaApp>| h.get_by_label("Immediate").rect().top();
    let before = top(&harness);
    for _ in 0..30 {
        harness.get_by_label("Immediate expression").hover();
        harness.run();
        harness.get_by_label("Immediate").hover();
        harness.run();
    }
    assert_eq!(top(&harness), before);
}
