//! Hiding nodes in the tree view (DESIGN.md, "Tabular view of arrays",
//! "Hiding"): from the context menu of a row, counted on the parent, passed
//! over by the arrow keys, and shown again from the list.

use std::time::Duration;

use cliotree_app::ClioApp;
use cliotree_core::Path;
use egui::Key;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

fn p(text: &str) -> Path {
    text.parse().unwrap()
}

/// The sample, with the customer open.
fn window() -> Harness<'static, ClioApp> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 700.0))
        .build_ui_state(|ui, app: &mut ClioApp| app.show(ui), ClioApp::sample());
    assert!(
        harness
            .state_mut()
            .session
            .wait_idle(Duration::from_secs(60))
    );
    harness.state_mut().set_open(&p("customer"), true);
    harness.run();
    harness
}

fn click(harness: &mut Harness<'_, ClioApp>, label: &str) {
    harness.get_by_label(label).click();
    harness.run();
}

fn hide(harness: &mut Harness<'_, ClioApp>, row: &str) {
    harness.get_by_label(row).click_secondary();
    harness.run();
    click(harness, "Hide");
}

#[test]
fn a_hidden_node_is_counted_on_its_parent_and_shown_again() {
    let mut harness = window();
    click(&mut harness, "city");
    hide(&mut harness, "city");
    assert!(harness.query_by_label("city").is_none());
    assert!(harness.query_by_label("Utrecht").is_none());
    // The selection moved to the next row that is shown.
    assert_eq!(harness.state().selection, Some(p("customer.vip")));
    harness.get_by_label("(1 hidden)");
    harness.get_by_label("1 hidden");
    // Hidden is not an edit.
    assert!(!harness.state().is_dirty());

    click(&mut harness, "(1 hidden)");
    click(&mut harness, "Show city");
    harness.get_by_label("city");
    harness.get_by_label("Utrecht");
    assert!(harness.query_by_label("(1 hidden)").is_none());
}

#[test]
fn the_arrow_keys_pass_over_hidden_nodes() {
    let mut harness = window();
    hide(&mut harness, "city");
    click(&mut harness, "name");
    harness.key_press(Key::ArrowDown);
    harness.run();
    assert_eq!(harness.state().selection, Some(p("customer.vip")));
    harness.key_press(Key::ArrowUp);
    harness.run();
    assert_eq!(harness.state().selection, Some(p("customer.name")));
}

#[test]
fn hiding_a_node_hides_what_is_inside_it_and_show_all_brings_all_back() {
    let mut harness = window();
    hide(&mut harness, "customer");
    for gone in ["customer", "name", "Acme Ltd", "city"] {
        assert!(harness.query_by_label(gone).is_none(), "{gone}");
    }
    click(&mut harness, "(1 hidden)");
    click(&mut harness, "Show all");
    harness.get_by_label("Acme Ltd");
    // The list is also on the parent's menu.
    hide(&mut harness, "name");
    hide(&mut harness, "city");
    harness.get_by_label("(2 hidden)");
    harness.get_by_label("customer").click_secondary();
    harness.run();
    click(&mut harness, "Hidden (2)");
    harness.get_by_label("Show name");
    harness.get_by_label("Show city");
    click(&mut harness, "Show all");
    harness.get_by_label("Acme Ltd");
    harness.get_by_label("Utrecht");
}

#[test]
fn a_hidden_item_stays_hidden_when_the_items_are_reordered() {
    let mut harness = window();
    harness.state_mut().set_open(&p("items"), true);
    harness.run();
    // The third item (the 12 at 0.75) is hidden; reversing makes it the
    // first.
    harness.state_mut().hide_node(&p("items[2]"));
    harness.run();
    assert!(harness.query_by_label("[2]").is_none());
    harness.state_mut().reverse_rows(&p("items"));
    harness.run();
    let children = harness.state().hidden_children(&p("items"));
    assert_eq!(children.len(), 1);
    assert_eq!(children[0].path, p("items[0]"));
    assert!(harness.query_by_label("[0]").is_none());
    harness.get_by_label("[1]");
    harness.get_by_label("[2]");
}
