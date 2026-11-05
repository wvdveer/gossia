//! Tests of copy and paste of cells, without a window.

use std::time::Duration;

use gossia_core::{Path, Value};

use crate::GossiaApp;
use crate::clipboard::{CopyMode, PasteMode, from_tsv, to_tsv};

fn p(text: &str) -> Path {
    text.parse().unwrap()
}

fn app() -> GossiaApp {
    let mut app = GossiaApp::sample();
    settle(&mut app);
    app.set_tabular(&p("items"), true);
    settle(&mut app);
    app
}

fn settle(app: &mut GossiaApp) {
    assert!(app.session.wait_idle(Duration::from_secs(60)));
    app.refresh_rows();
    app.refresh_table();
}

fn value(app: &GossiaApp, path: &str) -> Option<Value> {
    app.calc()
        .data("Orders")
        .unwrap()
        .get(&p(path))
        .map(|n| n.value().clone())
}

fn items(app: &GossiaApp) -> usize {
    value(app, "items")
        .and_then(|v| v.as_array().map(<[_]>::len))
        .unwrap()
}

fn history(app: &GossiaApp) -> usize {
    app.calc().history_len().0
}

fn copy(app: &mut GossiaApp) -> Option<String> {
    app.copy_cells(CopyMode::Cells)
}

fn paste(app: &mut GossiaApp, text: &str) -> Result<(), String> {
    app.paste_clip(text, PasteMode::Cells)
}

fn formula(app: &GossiaApp, path: &str) -> Option<String> {
    app.calc().formula_text("Orders", &p(path))
}

#[test]
fn tab_separated_text_is_written_and_read_back() {
    let rows = vec![
        vec!["a".to_owned(), "two\twords".to_owned(), "".to_owned()],
        vec![
            "line\nbreak".to_owned(),
            "\"quoted\"".to_owned(),
            "it's".to_owned(),
        ],
        vec!["".to_owned()],
        vec!["say \"hi\"".to_owned()],
    ];
    let text = to_tsv(&rows);
    assert_eq!(
        text,
        "a\t\"two\twords\"\t\n\"line\nbreak\"\t\"\"\"quoted\"\"\"\tit's\n\nsay \"hi\"\n"
    );
    assert_eq!(from_tsv(&text), rows, "a blank row stays a row");
    assert_eq!(
        from_tsv("1\t2\r\n3\t4\r\n"),
        vec![vec!["1", "2"], vec!["3", "4"]],
        "Windows line ends, and the last one dropped"
    );
    assert_eq!(from_tsv(" x \t"), vec![vec![" x ", ""]], "not trimmed");
}

#[test]
fn shift_makes_a_range_and_copy_gives_its_cells() {
    let mut app = app();
    app.select(&p("items[0].sku"));
    assert!(app.extend_selection(&p("items[1].price")));
    let range = app.table_range().unwrap();
    assert_eq!((range.rows, range.columns), (0..=1, 0..=2));
    assert_eq!(copy(&mut app).unwrap(), "A-100\t2\t9.5\nB-220\t1\t24.0\n");
    // A plain click starts again from one cell.
    app.select(&p("items[2].qty"));
    assert!(!app.table_range().unwrap().is_range());
    assert_eq!(copy(&mut app).unwrap(), "12\n");
}

#[test]
fn a_copied_range_includes_hidden_rows_and_columns() {
    let mut app = app();
    app.hide_column(&p("items"), "qty");
    app.hide_row(&p("items"), 1);
    settle(&mut app);
    app.select(&p("items[0].sku"));
    app.table_extend_selection(1, 1);
    assert_eq!(
        app.selection,
        Some(p("items[2].price")),
        "Shift and the arrows move among the cells that are shown"
    );
    assert_eq!(
        copy(&mut app).unwrap(),
        "A-100\t2\t9.5\nB-220\t1\t24.0\nC-310\t12\t0.75\n"
    );
}

#[test]
fn copy_gives_values_and_copy_formulas_gives_formulas() {
    let mut app = app();
    app.select(&p("items[0].price"));
    app.extend_selection(&p("items[0].total"));
    assert_eq!(copy(&mut app).unwrap(), "9.5\t19.0\n");
    assert_eq!(
        app.copy_cells(CopyMode::Formulas).unwrap(),
        "9.5\t=parent.qty * parent.price\n",
        "a cell without a formula gives its value"
    );
}

#[test]
fn a_paste_of_this_windows_copy_puts_the_formulas_back() {
    let mut app = app();
    app.select(&p("subtotal"));
    let text = copy(&mut app).unwrap();
    assert_eq!(text, "52.0\n");
    app.select(&p("tax"));
    paste(&mut app, &text).unwrap();
    settle(&mut app);
    assert_eq!(formula(&app, "tax"), formula(&app, "subtotal"));
    assert_eq!(value(&app, "tax"), Some(Value::Float(52.0)));
}

#[test]
fn paste_values_writes_the_values_even_of_this_windows_copy() {
    let mut app = app();
    app.select(&p("items[0].total"));
    let text = copy(&mut app).unwrap();
    app.select(&p("items[1].total"));
    app.paste_clip(&text, PasteMode::Values).unwrap();
    settle(&mut app);
    assert_eq!(value(&app, "items[1].total"), Some(Value::Float(19.0)));
    assert_eq!(
        formula(&app, "items[1].total"),
        None,
        "the column formula ends here"
    );
    assert!(formula(&app, "items[2].total").is_some());
}

#[test]
fn text_copied_elsewhere_is_pasted_as_values() {
    let mut app = app();
    app.select(&p("subtotal"));
    copy(&mut app).unwrap();
    app.select(&p("tax"));
    // Another program put something else on the clipboard since.
    paste(&mut app, "3.5").unwrap();
    settle(&mut app);
    assert_eq!(formula(&app, "tax"), None);
    assert_eq!(value(&app, "tax"), Some(Value::Float(3.5)));
}

#[test]
fn error_values_are_left_out_and_counted() {
    let mut app = app();
    app.select(&p("items[0].total"));
    app.enter_directly(&p("items[0].total"), "=1/0");
    settle(&mut app);
    app.select(&p("items[0].price"));
    app.extend_selection(&p("items[0].total"));
    let text = copy(&mut app).unwrap();
    assert_eq!(text, "9.5\t#DIV/0!\n");
    app.select(&p("items[1].price"));
    app.paste_clip(&text, PasteMode::Values).unwrap();
    settle(&mut app);
    assert_eq!(value(&app, "items[1].price"), Some(Value::Float(9.5)));
    assert!(formula(&app, "items[1].total").is_some(), "left as it was");
    assert_eq!(
        app.notice.as_deref(),
        Some("1 cell with an error was not pasted")
    );
    // An error code in text from elsewhere is left out too.
    app.select(&p("items[2].qty"));
    paste(&mut app, "#N/A\t2").unwrap();
    settle(&mut app);
    assert_eq!(value(&app, "items[2].qty"), Some(Value::Int(12)));
    assert_eq!(value(&app, "items[2].price"), Some(Value::Int(2)));
}

#[test]
fn the_stand_in_clipboard_carries_a_copy_to_a_paste() {
    let mut app = app();
    app.tables.stand_in = Some(String::new());
    app.select(&p("items[0].qty"));
    app.extend_selection(&p("items[0].price"));
    copy(&mut app).unwrap();
    assert_eq!(app.tables.stand_in.as_deref(), Some("2\t9.5\n"));
    assert!(app.tables.to_clipboard.is_none());
    app.select(&p("items[2].qty"));
    app.paste_from_clipboard(PasteMode::Values).unwrap();
    settle(&mut app);
    assert_eq!(value(&app, "items[2].qty"), Some(Value::Int(2)));
    assert_eq!(value(&app, "items[2].price"), Some(Value::Float(9.5)));
}

#[test]
fn a_paste_writes_every_cell_as_one_undoable_entry() {
    let mut app = app();
    app.select(&p("items[0].qty"));
    let before = history(&app);
    paste(&mut app, "5\t1.5\n6\t2\n").unwrap();
    settle(&mut app);
    assert_eq!(history(&app), before + 1);
    assert_eq!(value(&app, "items[0].qty"), Some(Value::Int(5)));
    assert_eq!(value(&app, "items[0].price"), Some(Value::Float(1.5)));
    assert_eq!(value(&app, "items[1].qty"), Some(Value::Int(6)));
    assert_eq!(value(&app, "items[1].price"), Some(Value::Int(2)));
    assert_eq!(value(&app, "items[0].total"), Some(Value::Float(7.5)));
    app.undo();
    settle(&mut app);
    assert_eq!(value(&app, "items[0].qty"), Some(Value::Int(2)));
    assert_eq!(value(&app, "items[1].price"), Some(Value::Float(24.0)));
}

#[test]
fn a_cell_that_does_not_fit_refuses_the_whole_paste() {
    let mut app = app();
    app.select(&p("items[0].qty"));
    let before = history(&app);
    let error = paste(&mut app, "5\t1.5\n6\tcheap\n").unwrap_err();
    settle(&mut app);
    assert!(error.contains("items[1].price"), "{error}");
    assert!(error.contains("nothing was pasted"), "{error}");
    assert_eq!(app.notice.as_deref(), Some(error.as_str()));
    assert_eq!(history(&app), before, "nothing to undo or redo");
    assert_eq!(value(&app, "items[0].qty"), Some(Value::Int(2)));
}

#[test]
fn one_cell_fills_the_range() {
    let mut app = app();
    app.select(&p("items[0].qty"));
    app.extend_selection(&p("items[2].qty"));
    paste(&mut app, "7").unwrap();
    settle(&mut app);
    for i in 0..3 {
        assert_eq!(value(&app, &format!("items[{i}].qty")), Some(Value::Int(7)));
    }
}

#[test]
fn rows_past_the_end_are_added_in_the_same_entry() {
    let mut app = app();
    app.select(&p("items[2].sku"));
    paste(&mut app, "X-1\t3\t1\nY-2\t4\t2\n").unwrap();
    settle(&mut app);
    assert_eq!(items(&app), 4);
    assert_eq!(value(&app, "items[2].sku"), Some(Value::Str("X-1".into())));
    assert_eq!(value(&app, "items[3].sku"), Some(Value::Str("Y-2".into())));
    assert_eq!(value(&app, "items[3].qty"), Some(Value::Int(4)));
    app.undo();
    settle(&mut app);
    assert_eq!(items(&app), 3);
    assert_eq!(
        value(&app, "items[2].sku"),
        Some(Value::Str("C-310".into()))
    );
}

#[test]
fn copying_a_row_onto_another_keeps_the_column_formula() {
    let mut app = app();
    app.select(&p("items[0].sku"));
    app.extend_selection(&p("items[0].total"));
    let text = copy(&mut app).unwrap();
    app.select(&p("items[1].sku"));
    paste(&mut app, &text).unwrap();
    settle(&mut app);
    assert_eq!(
        value(&app, "items[1].sku"),
        Some(Value::Str("A-100".into()))
    );
    assert_eq!(
        app.calc()
            .formula_text("Orders", &p("items[1].total"))
            .as_deref(),
        Some("parent.qty * parent.price"),
    );
    assert_eq!(value(&app, "items[1].total"), Some(Value::Float(19.0)));
}

#[test]
fn a_paste_wider_than_the_table_is_refused() {
    let mut app = app();
    app.select(&p("items[0].note"));
    let error = paste(&mut app, "a\tb").unwrap_err();
    assert!(error.contains("2 columns wide"), "{error}");
    assert_eq!(value(&app, "items[0].note"), None);
}

#[test]
fn outside_a_table_one_cell_is_copied_and_pasted() {
    let mut app = app();
    app.select(&p("tax_rate"));
    assert!(app.table_range().is_none());
    assert_eq!(copy(&mut app).unwrap(), "0.21\n");
    paste(&mut app, "0.1\n").unwrap();
    settle(&mut app);
    assert_eq!(value(&app, "tax_rate"), Some(Value::Float(0.1)));
    assert!(paste(&mut app, "1\t2").is_err());
}
