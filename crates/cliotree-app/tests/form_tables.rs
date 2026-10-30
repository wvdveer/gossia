//! The filters and the hiding of the tabular view on the table of a form
//! (DESIGN.md, "Forms", "Filters and hiding in a form's table"), driven as a
//! user does.

use std::collections::BTreeSet;
use std::time::Duration;

use cliotree_app::ClioApp;
use cliotree_app::form::View;
use cliotree_app::table::Filter;
use cliotree_core::Path;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

fn p(text: &str) -> Path {
    text.parse().unwrap()
}

fn settle(harness: &mut Harness<'_, ClioApp>) {
    assert!(
        harness
            .state_mut()
            .session
            .wait_idle(Duration::from_secs(60))
    );
    harness.run();
}

/// The Default form of the sample's Orders.
fn orders_form() -> Harness<'static, ClioApp> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 900.0))
        .build_ui_state(|ui, app: &mut ClioApp| app.show(ui), ClioApp::sample());
    settle(&mut harness);
    assert!(
        harness
            .state_mut()
            .set_view(View::Form("Default".to_owned()))
    );
    harness.run();
    harness.run();
    harness
}

fn click(harness: &mut Harness<'_, ClioApp>, label: &str) {
    harness.get_by_label(label).click();
    harness.run();
}

fn shown(harness: &Harness<'_, ClioApp>, field: &str) -> bool {
    harness.query_by_label(field).is_some()
}

/// Only the rows whose sku is one of these.
fn only_skus(harness: &mut Harness<'_, ClioApp>, skus: &[&str]) {
    let values: BTreeSet<String> = skus.iter().map(|s| (*s).to_owned()).collect();
    let filter = Filter {
        values: Some(values),
        ..Filter::default()
    };
    harness
        .state_mut()
        .set_filter(&p("items"), "sku", Some(filter));
    harness.run();
}

#[test]
fn a_filter_of_the_array_shows_only_the_rows_that_pass() {
    let mut harness = orders_form();
    let key = harness.state().cell_key(&p("items[1].sku"));
    only_skus(&mut harness, &[&key]);
    assert!(!shown(&harness, "Orders.items[0].sku"));
    assert!(shown(&harness, "Orders.items[1].sku"));
    assert!(!shown(&harness, "Orders.items[2].sku"));
    harness.get_by_label("1 shown (0 hidden, 2 filtered)");
    harness.get_by_label("Filtered");
    click(&mut harness, "Clear all filters");
    assert!(shown(&harness, "Orders.items[0].sku"));
    assert!(harness.query_by_label("Filtered").is_none());
}

#[test]
fn a_row_edited_on_the_form_stays_shown_until_the_filter_is_applied_again() {
    let mut harness = orders_form();
    let key = harness.state().cell_key(&p("items[1].sku"));
    only_skus(&mut harness, &[&key]);
    harness.get_by_label("Orders.items[1].sku").click();
    harness.run();
    harness.key_press(egui::Key::End);
    harness.event(egui::Event::Text("X".to_owned()));
    harness.run();
    harness.key_press(egui::Key::Enter);
    settle(&mut harness);
    assert_eq!(harness.state().cell_key(&p("items[1].sku")), "B-220X");
    assert!(shown(&harness, "Orders.items[1].sku"));
    click(&mut harness, "Reapply filter");
    assert!(!shown(&harness, "Orders.items[1].sku"));
    harness.get_by_label("0 shown (0 hidden, 3 filtered)");
}

#[test]
fn a_column_title_filters_and_hides_its_column() {
    let mut harness = orders_form();
    harness.get_by_label("sku").click_secondary();
    harness.run();
    click(&mut harness, "Filter...");
    harness.get_by_label("Filter sku");
    click(&mut harness, "Cancel");

    harness.get_by_label("sku").click_secondary();
    harness.run();
    click(&mut harness, "Hide column");
    assert!(!shown(&harness, "Orders.items[0].sku"));
    assert!(shown(&harness, "Orders.items[0].qty"));
    // The tabular view of the array hides it too: it is one state.
    assert!(
        harness
            .state()
            .view_of(&p("items"))
            .is_some_and(|v| v.hidden_columns.contains("sku"))
    );
    click(&mut harness, "Hidden (1)");
    click(&mut harness, "Show column sku");
    assert!(shown(&harness, "Orders.items[0].sku"));
    assert!(harness.query_by_label_contains("Hidden (").is_none());
}

#[test]
fn a_row_is_hidden_from_its_menu_and_shown_again() {
    let mut harness = orders_form();
    harness.get_by_label("[1]").click_secondary();
    harness.run();
    click(&mut harness, "Hide row");
    assert!(!shown(&harness, "Orders.items[1].sku"));
    assert!(shown(&harness, "Orders.items[2].sku"));
    harness.get_by_label("2 shown (1 hidden, 0 filtered)");
    click(&mut harness, "Hidden (1)");
    click(&mut harness, "Show row [1]");
    assert!(shown(&harness, "Orders.items[1].sku"));
}

#[test]
fn layout_mode_shows_every_row_and_column() {
    let mut harness = orders_form();
    let key = harness.state().cell_key(&p("items[1].sku"));
    only_skus(&mut harness, &[&key]);
    harness.state_mut().hide_column(&p("items"), "qty");
    harness.run();
    assert!(!shown(&harness, "Orders.items[0].sku"));
    click(&mut harness, "Edit form");
    harness.run();
    assert!(shown(&harness, "Orders.items[0].sku"));
    assert!(shown(&harness, "Orders.items[0].qty"));
}
