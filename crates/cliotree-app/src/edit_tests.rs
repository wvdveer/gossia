//! Tests of what the window does when the user edits, without a window.

use std::time::Duration;

use cliotree_core::{Path, Value};

use crate::ClioApp;

fn p(text: &str) -> Path {
    if text.is_empty() {
        Path::root()
    } else {
        text.parse().unwrap()
    }
}

fn app() -> ClioApp {
    let mut app = ClioApp::sample();
    settle(&mut app);
    app
}

fn settle(app: &mut ClioApp) {
    assert!(app.session.wait_idle(Duration::from_secs(60)));
    app.refresh_rows();
}

fn value(app: &ClioApp, path: &str) -> Option<Value> {
    app.calc()
        .data("Orders")
        .unwrap()
        .get(&p(path))
        .map(|n| n.value().clone())
}

#[test]
fn the_sample_is_calculated_when_it_opens() {
    let app = app();
    assert_eq!(value(&app, "total"), Some(Value::Float(62.92)));
    assert!(!app.calc().is_modified());
}

#[test]
fn a_cell_is_edited_as_its_formula_or_its_value() {
    let app = app();
    assert_eq!(app.cell_text(&p("total")), "=Orders.subtotal + Orders.tax");
    assert_eq!(app.cell_text(&p("tax_rate")), "0.21");
    assert_eq!(app.cell_text(&p("customer.name")), "Acme Ltd");
    assert_eq!(app.cell_text(&p("customer.city")), "Utrecht");
    assert_eq!(app.cell_text(&p("customer.vip")), "True");
    assert_eq!(
        app.cell_text(&p("items")),
        "",
        "a whole array has no literal text"
    );
    assert_eq!(app.cell_text(&p("items[1].note")), "gift wrap");
    assert_eq!(app.cell_text(&p("items[0].note")), "", "absent");
}

/// Entering formulas: text that starts with `=` or an apostrophe is shown with
/// an apostrophe in front, so that editing it and entering it changes nothing.
#[test]
fn text_that_looks_like_a_formula_is_shown_with_an_apostrophe_and_enters_as_itself() {
    let mut app = app();
    app.select(&p("customer.name"));
    assert!(app.begin_editing(&p("customer.name"), false, Some("'=Acme".to_owned())));
    assert!(app.commit_editing());
    assert_eq!(
        value(&app, "customer.name"),
        Some(Value::Str("=Acme".into()))
    );
    assert_eq!(app.cell_text(&p("customer.name")), "'=Acme");
    // Entering what the bar shows leaves the value as it is.
    assert!(app.begin_editing(&p("customer.name"), false, None));
    assert!(app.commit_editing());
    assert_eq!(
        value(&app, "customer.name"),
        Some(Value::Str("=Acme".into()))
    );
    assert_eq!(app.calc().history_len().0, 2);
}

#[test]
fn a_refused_entry_keeps_editing_and_says_why() {
    let mut app = app();
    app.select(&p("tax_rate"));
    app.begin_editing(&p("tax_rate"), false, Some("7".to_owned()));
    assert!(!app.commit_editing());
    assert!(app.editing.is_some());
    assert!(
        app.notice.as_deref().unwrap_or("").contains("maximum"),
        "{:?}",
        app.notice
    );
    // Moving the selection is refused while the text is not accepted.
    assert!(!app.select(&p("subtotal")));
    assert_eq!(app.selection, Some(p("tax_rate")));
    app.cancel_editing();
    assert!(app.select(&p("subtotal")));
    assert_eq!(value(&app, "tax_rate"), Some(Value::Float(0.21)));
}

#[test]
fn selecting_another_cell_enters_what_was_typed() {
    let mut app = app();
    app.select(&p("tax_rate"));
    app.begin_editing(&p("tax_rate"), true, Some("0.5".to_owned()));
    assert!(app.select(&p("subtotal")));
    assert_eq!(value(&app, "tax_rate"), Some(Value::Float(0.5)));
    settle(&mut app);
    assert_eq!(value(&app, "total"), Some(Value::Float(78.0)));
}

/// A node inside a formula's result is read-only, and the reason names the
/// formula.
#[test]
fn a_computed_node_cannot_be_edited_and_the_reason_names_the_formula() {
    let mut app = app();
    app.session
        .calc_mut()
        .set_formula("Orders", &p("items"), "PLAIN(Orders.items)")
        .unwrap_err();
    // Put a formula on an array that gives a whole array.
    app.session
        .calc_mut()
        .set_formula("Prices", &p("list"), "{'x': 1.0}")
        .unwrap();
    app.switch_to("Prices");
    settle(&mut app);
    assert!(!app.can_edit(&p("list.x")));
    assert!(!app.begin_editing(&p("list.x"), false, None));
    let notice = app.notice.clone().unwrap();
    assert!(
        notice.contains("Prices.list") && notice.contains("formula"),
        "{notice}"
    );
    assert!(
        app.can_edit(&p("list")),
        "the formula itself can be changed"
    );
    assert!(
        !app.can_edit(&p("nothing")),
        "the schema defines no such node"
    );
}

#[test]
fn a_boolean_and_a_choice_are_entered_without_typing() {
    let mut app = app();
    app.enter_directly(&p("customer.vip"), "false");
    app.enter_directly(&p("customer.status"), "shipped");
    assert_eq!(value(&app, "customer.vip"), Some(Value::Bool(false)));
    assert_eq!(
        value(&app, "customer.status"),
        Some(Value::Str("shipped".into()))
    );
    app.enter_directly(&p("customer.status"), "lost");
    assert!(app.notice.is_some(), "not one of the choices");
    assert_eq!(
        value(&app, "customer.status"),
        Some(Value::Str("shipped".into()))
    );
}

#[test]
fn items_are_added_inserted_and_deleted_and_each_is_one_undo() {
    let mut app = app();
    app.insert_item(&p("items"), None);
    let Some(Value::Array(items)) = value(&app, "items") else {
        panic!("no array");
    };
    assert_eq!(items.len(), 4);
    // The new item has its required fields filled in with defaults.
    assert_eq!(value(&app, "items[3].sku"), Some(Value::Str(String::new())));
    assert_eq!(
        value(&app, "items[3].qty"),
        Some(Value::Int(1)),
        "the minimum"
    );
    assert_eq!(value(&app, "items[3].price"), Some(Value::Int(0)));
    assert!(app.is_open(&p("items")), "the array opens to show it");

    app.insert_item(&p("items"), Some(0));
    assert_eq!(app.calc().history_len().0, 2);
    app.delete_node(&p("items[1]"));
    app.undo();
    app.undo();
    app.undo();
    let Some(Value::Array(items)) = value(&app, "items") else {
        panic!("no array");
    };
    assert_eq!(items.len(), 3);
}

#[test]
fn deleting_a_required_field_leaves_it_missing() {
    let mut app = app();
    app.delete_node(&p("customer.name"));
    assert_eq!(value(&app, "customer.name"), None);
    app.refresh_rows();
    app.refresh_summary();
    assert_eq!(
        app.summary.invalid, 1,
        "a missing required field is counted"
    );
}

#[test]
fn undo_and_redo_are_refused_while_a_cell_is_being_edited() {
    let mut app = app();
    app.enter_directly(&p("tax_rate"), "0.3");
    app.select(&p("subtotal"));
    app.begin_editing(&p("subtotal"), true, Some("5".to_owned()));
    app.undo();
    assert_eq!(value(&app, "tax_rate"), Some(Value::Float(0.3)));
    app.cancel_editing();
    app.undo();
    assert_eq!(value(&app, "tax_rate"), Some(Value::Float(0.21)));
    app.redo();
    assert_eq!(value(&app, "tax_rate"), Some(Value::Float(0.3)));
}

#[test]
fn the_selection_moves_among_the_rows_and_stays_in_range() {
    let mut app = app();
    app.move_selection(1);
    assert_eq!(
        app.selection,
        Some(p("")),
        "the first row is the tab itself"
    );
    app.move_selection(1);
    assert_eq!(app.selection, Some(p("customer")));
    app.move_selection(-10);
    assert_eq!(app.selection, Some(p("")));
    app.move_selection(100);
    assert_eq!(app.selection, Some(p("total")));
    assert_eq!(app.scroll_to, app.row_index(&p("total")));
}

#[test]
fn revealing_a_node_opens_what_is_above_it() {
    let mut app = app();
    app.reveal(&p("items[2].price"));
    assert!(app.is_open(&p("items")) && app.is_open(&p("items[2]")));
    assert!(app.row_index(&p("items[2].price")).is_some());
    assert_eq!(app.selection, Some(p("items[2].price")));
}

// ------------------------------------------------------- completion

#[test]
fn names_are_offered_while_typing_a_formula_and_accepted_into_the_text() {
    let mut app = app();
    app.select(&p("tax_rate"));
    app.begin_editing(&p("tax_rate"), false, Some("=Orders.sub".to_owned()));
    app.update_completion();
    let labels: Vec<_> = app
        .completion
        .items
        .iter()
        .map(|i| i.label.clone())
        .collect();
    assert_eq!(labels, ["subtotal"]);
    app.accept_completion(0);
    let editing = app.editing.as_ref().unwrap();
    assert_eq!(editing.text, "=Orders.subtotal");
    assert_eq!(editing.cursor, "=Orders.subtotal".len());
    assert_eq!(editing.set_cursor, Some(editing.cursor));
    assert!(app.completion.items.is_empty());
}

#[test]
fn nothing_is_offered_for_a_literal_or_an_empty_word() {
    let mut app = app();
    app.select(&p("customer.name"));
    app.begin_editing(&p("customer.name"), false, Some("Orders.sub".to_owned()));
    app.update_completion();
    assert!(app.completion.items.is_empty(), "not a formula");
    app.cancel_editing();
    app.begin_editing(&p("customer.name"), false, Some("=".to_owned()));
    app.update_completion();
    assert!(app.completion.items.is_empty(), "wait for a first letter");
    app.editing.as_mut().unwrap().text = "=Orders.".to_owned();
    app.editing.as_mut().unwrap().cursor = 8;
    app.update_completion();
    assert!(!app.completion.items.is_empty(), "after a dot, the fields");
}

#[test]
fn a_dismissed_popup_stays_away_until_the_text_changes() {
    let mut app = app();
    app.select(&p("tax_rate"));
    app.begin_editing(&p("tax_rate"), false, Some("=Orders.su".to_owned()));
    app.update_completion();
    assert!(!app.completion.items.is_empty());
    app.completion.dismissed_for = Some("=Orders.su".to_owned());
    app.update_completion();
    assert!(app.completion.items.is_empty());
    app.editing.as_mut().unwrap().text = "=Orders.sub".to_owned();
    app.editing.as_mut().unwrap().cursor = 11;
    app.update_completion();
    assert!(!app.completion.items.is_empty());
}

// -------------------------------------------------- Insert Function

#[test]
fn insert_function_writes_a_function_where_the_cursor_is() {
    let mut app = app();
    app.select(&p("tax_rate"));
    app.insert_function_text("IFERROR(");
    let editing = app.editing.as_ref().unwrap();
    assert_eq!(editing.text, "=IFERROR(", "a formula is started");
    assert_eq!(editing.cursor, 9);
    app.editing.as_mut().unwrap().text = "=1 + 2".to_owned();
    app.editing.as_mut().unwrap().cursor = 3;
    app.insert_function_text("abs(");
    assert_eq!(app.editing.as_ref().unwrap().text, "=1 abs(+ 2");
}

#[test]
fn insert_function_needs_a_cell_to_put_it_in() {
    let mut app = app();
    app.insert_function_text("sum(");
    assert!(app.editing.is_none());
    assert!(
        app.notice
            .as_deref()
            .unwrap_or("")
            .contains("select a cell")
    );
}

#[test]
fn functions_are_found_by_name_or_by_what_they_do() {
    let by_name = ClioApp::functions_matching("iferr");
    assert_eq!(by_name.len(), 1);
    assert_eq!(by_name[0].name, "IFERROR");
    assert!(
        ClioApp::functions_matching("regular expression")
            .iter()
            .any(|f| f.name == "re")
    );
    assert!(ClioApp::functions_matching("").len() > 20);
    assert!(ClioApp::functions_matching("zzzz").is_empty());
}
