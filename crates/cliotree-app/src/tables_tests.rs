//! Tests of the tabular view without a window.

use std::time::Duration;

use cliotree_core::{Path, Value};

use crate::ClioApp;
use crate::table::{Condition, Filter};

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
    app.set_tabular(&p("items"), true);
    settle(&mut app);
    app
}

fn settle(app: &mut ClioApp) {
    assert!(app.session.wait_idle(Duration::from_secs(60)));
    app.refresh_rows();
    app.refresh_table();
}

fn skus(app: &ClioApp) -> Vec<String> {
    (0..3)
        .map(|i| {
            let node = app
                .calc()
                .data("Orders")
                .unwrap()
                .get(&p(&format!("items[{i}].sku")))
                .unwrap();
            node.value().as_str().unwrap().to_owned()
        })
        .collect()
}

fn number(app: &ClioApp, path: &str) -> Value {
    app.calc()
        .data("Orders")
        .unwrap()
        .get(&p(path))
        .unwrap()
        .value()
        .clone()
}

fn shown(app: &ClioApp) -> Vec<usize> {
    app.layout().unwrap().visible.rows.clone()
}

fn greater_than(n: &str) -> Filter {
    Filter {
        condition: Some(Condition::Greater(n.to_owned())),
        ..Filter::default()
    }
}

#[test]
fn only_an_array_of_objects_can_be_shown_as_a_table() {
    let mut app = ClioApp::sample();
    assert!(app.can_tabulate(&p("items")));
    assert!(!app.can_tabulate(&p("customer")));
    assert!(!app.can_tabulate(&p("customer.name")));
    app.set_tabular(&p("customer"), true);
    assert!(app.active_table().is_none());
    assert!(app.layout().is_none());
}

#[test]
fn a_table_has_the_schemas_columns_and_takes_the_items_out_of_the_tree() {
    let mut app = app();
    let layout = app.layout().unwrap();
    let names: Vec<_> = layout.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["sku", "qty", "price", "total", "note"]);
    assert_eq!(layout.visible.rows, [0, 1, 2]);
    // The tree has the array's row and no rows for the items.
    app.set_open(&p("items"), true);
    app.refresh_rows();
    assert!(app.row_index(&p("items")).is_some());
    assert!(app.row_index(&p("items[0]")).is_none());
    // Closing the table brings the items back.
    app.set_tabular(&p("items"), false);
    settle(&mut app);
    app.set_open(&p("items"), true);
    app.refresh_rows();
    assert!(app.row_index(&p("items[0]")).is_some());
    assert!(app.layout().is_none());
}

#[test]
fn a_sort_reorders_the_array_and_formulas_stay_at_their_place() {
    let mut app = app();
    app.sort_table(&p("items"), "qty", None);
    settle(&mut app);
    assert_eq!(skus(&app), ["B-220", "A-100", "C-310"]);
    // The formula in `total` belongs to the place: it now multiplies the item
    // that is there.
    assert_eq!(number(&app, "items[0].total"), Value::Float(24.0));
    assert_eq!(number(&app, "items[1].total"), Value::Float(19.0));
    assert_eq!(app.calc().undo_label(), Some("Sort Orders.items"));
    assert_eq!(app.calc().history_len(), (1, 0));

    // The same column again is descending.
    app.sort_table(&p("items"), "qty", None);
    settle(&mut app);
    assert_eq!(skus(&app), ["C-310", "A-100", "B-220"]);

    app.undo();
    app.undo();
    settle(&mut app);
    assert_eq!(skus(&app), ["A-100", "B-220", "C-310"]);
    assert_eq!(number(&app, "items[0].total"), Value::Float(19.0));
}

#[test]
fn a_column_that_cannot_be_ordered_leaves_the_data_alone_and_the_rows_can_be_shown() {
    let mut app = app();
    app.sort_table(&p("items"), "note", None);
    assert_eq!(skus(&app), ["A-100", "B-220", "C-310"]);
    assert_eq!(app.calc().history_len(), (0, 0));
    let refused = app.tables.refused.clone().unwrap();
    assert_eq!(refused.rows, [0, 2]);
    assert!(
        refused.message.contains("not supported between"),
        "{refused:?}"
    );

    app.show_refused_rows();
    settle(&mut app);
    assert_eq!(shown(&app), [0, 2], "the rows with no note");
    assert!(app.tables.refused.is_none());
}

#[test]
fn reversing_twice_restores_the_order_and_is_two_entries() {
    let mut app = app();
    app.reverse_rows(&p("items"));
    assert_eq!(skus(&app), ["C-310", "B-220", "A-100"]);
    app.reverse_rows(&p("items"));
    assert_eq!(skus(&app), ["A-100", "B-220", "C-310"]);
    assert_eq!(app.calc().history_len(), (2, 0));
}

/// Moving a row swaps it with the next row that is shown; a row hidden in
/// between stays where it is, and stays hidden.
#[test]
fn a_row_moves_past_hidden_rows_and_the_hiding_goes_with_the_item() {
    let mut app = app();
    app.hide_row(&p("items"), 1);
    settle(&mut app);
    assert_eq!(shown(&app), [0, 2]);
    app.move_row(&p("items"), 0, false);
    settle(&mut app);
    assert_eq!(skus(&app), ["C-310", "B-220", "A-100"]);
    assert_eq!(shown(&app), [0, 2], "B-220 is still the hidden one");
    // The first shown row cannot move up, nor the last move down.
    app.move_row(&p("items"), 0, true);
    app.move_row(&p("items"), 2, false);
    assert_eq!(app.calc().history_len(), (1, 0));
    // Sorting reorders the hidden item too, and it stays hidden.
    app.sort_table(&p("items"), "sku", None);
    settle(&mut app);
    assert_eq!(skus(&app), ["A-100", "B-220", "C-310"]);
    assert_eq!(shown(&app), [0, 2]);
    assert_eq!(app.hidden_rows(&p("items")).len(), 1);
    assert_eq!(app.hidden_rows(&p("items"))[0].0, 1);
    app.show_all_hidden(&p("items"));
    settle(&mut app);
    assert_eq!(shown(&app), [0, 1, 2]);
}

#[test]
fn a_filter_hides_rows_and_a_row_being_edited_stays_until_it_is_reapplied() {
    let mut app = app();
    app.set_filter(&p("items"), "qty", Some(greater_than("1")));
    settle(&mut app);
    assert_eq!(shown(&app), [0, 2]);
    let visible = &app.layout().unwrap().visible;
    assert_eq!((visible.total, visible.filtered, visible.hidden), (3, 1, 0));

    // Editing a row so that it no longer passes does not make it vanish.
    app.select(&p("items[0].qty"));
    app.begin_editing(&p("items[0].qty"), true, Some("1".into()));
    assert!(app.commit_editing());
    settle(&mut app);
    assert_eq!(shown(&app), [0, 2]);
    app.reapply_filters(&p("items"));
    settle(&mut app);
    assert_eq!(shown(&app), [2]);

    // Changing a filter is not an edit of the data.
    assert_eq!(app.calc().history_len(), (1, 0));

    // A row that is added stays shown.
    app.add_row(&p("items"), None);
    settle(&mut app);
    assert_eq!(shown(&app), [2, 3]);
    app.clear_all_filters(&p("items"));
    settle(&mut app);
    assert_eq!(shown(&app), [0, 1, 2, 3]);
}

#[test]
fn a_filter_follows_recalculation() {
    let mut app = app();
    // totals are 19, 24 and 9.
    app.set_filter(&p("items"), "total", Some(greater_than("10")));
    settle(&mut app);
    assert_eq!(shown(&app), [0, 1]);
    app.enter_directly(&p("items[2].qty"), "20");
    settle(&mut app);
    assert_eq!(number(&app, "items[2].total"), Value::Float(15.0));
    assert_eq!(shown(&app), [0, 1, 2], "live, not only when set");
}

#[test]
fn the_filter_dialog_applies_a_condition_and_says_when_an_operand_is_wrong() {
    let mut app = app();
    app.open_filter_dialog(&p("items"), "qty");
    {
        let dialog = app.tables.dialog.as_mut().unwrap();
        assert_eq!(dialog.distinct, ["1", "2", "12"]);
        dialog.choice = crate::tables::Choice::Greater;
        dialog.first = "many".into();
    }
    app.apply_filter_dialog();
    let dialog = app.tables.dialog.as_ref().expect("still open");
    assert!(dialog.error.as_deref().unwrap().contains("whole number"));
    app.tables.dialog.as_mut().unwrap().first = "1".into();
    app.apply_filter_dialog();
    assert!(app.tables.dialog.is_none());
    settle(&mut app);
    assert_eq!(shown(&app), [0, 2]);
    assert!(app.filter_of(&p("items"), "qty").is_some());

    // Unchecking a value filters by the list.
    app.clear_all_filters(&p("items"));
    app.open_filter_dialog(&p("items"), "qty");
    app.tables.dialog.as_mut().unwrap().checked.remove("12");
    app.apply_filter_dialog();
    settle(&mut app);
    assert_eq!(shown(&app), [0, 1]);
}

#[test]
fn hiding_a_column_hides_only_what_is_drawn() {
    let mut app = app();
    app.hide_column(&p("items"), "price");
    settle(&mut app);
    let layout = app.layout().unwrap();
    let shown: Vec<_> = layout
        .shown
        .iter()
        .map(|&i| layout.columns[i].name.as_str())
        .collect();
    assert_eq!(shown, ["sku", "qty", "total", "note"]);
    assert_eq!(number(&app, "items[0].price"), Value::Float(9.5));
    app.show_column(&p("items"), "price");
    settle(&mut app);
    assert_eq!(app.layout().unwrap().shown.len(), 5);
    assert_eq!(
        app.calc().history_len(),
        (0, 0),
        "view state is not undoable"
    );
}

#[test]
fn hiding_the_selected_row_moves_the_selection_to_the_nearest_shown_one() {
    let mut app = app();
    app.select(&p("items[1].qty"));
    app.hide_row(&p("items"), 1);
    settle(&mut app);
    assert_eq!(app.selection, Some(p("items[2].qty")));
    app.hide_row(&p("items"), 2);
    settle(&mut app);
    assert_eq!(app.selection, Some(p("items[0].qty")), "the last one shown");
}

#[test]
fn the_arrows_move_among_the_cells_that_are_shown() {
    let mut app = app();
    app.hide_column(&p("items"), "price");
    app.hide_row(&p("items"), 1);
    settle(&mut app);
    app.select(&p("items[0].qty"));
    assert!(app.table_cell_selected());
    app.table_move_selection(1, 0);
    assert_eq!(app.selection, Some(p("items[0].total")), "price is skipped");
    app.table_move_selection(0, 1);
    assert_eq!(app.selection, Some(p("items[2].total")), "row 1 is skipped");
    app.table_move_selection(0, 5);
    assert_eq!(
        app.selection,
        Some(p("items[2].total")),
        "stays in the table"
    );
    app.table_move_selection(-9, 0);
    assert_eq!(app.selection, Some(p("items[2].sku")));
    app.select(&p("tax_rate"));
    assert!(!app.table_cell_selected());
}

#[test]
fn a_table_cell_is_edited_like_any_cell_and_rows_are_added_and_deleted() {
    let mut app = app();
    app.select(&p("items[0].qty"));
    app.begin_editing(&p("items[0].qty"), true, Some("5".into()));
    assert!(app.commit_editing());
    settle(&mut app);
    assert_eq!(number(&app, "items[0].total"), Value::Float(47.5));

    app.add_row(&p("items"), Some(1));
    settle(&mut app);
    assert_eq!(shown(&app), [0, 1, 2, 3]);
    assert_eq!(
        number(&app, "items[1].qty"),
        Value::Int(1),
        "a new row has its defaults"
    );
    app.delete_row(&p("items"), 1);
    settle(&mut app);
    assert_eq!(shown(&app), [0, 1, 2]);
}
