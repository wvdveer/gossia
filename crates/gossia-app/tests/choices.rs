//! Lists of choices for dropdowns and typeahead fields, in the window
//! (DESIGN.md, "Forms", "Choices"): a *choices* expression gives the items, a
//! field shows an item's text and stores its code, and the list follows the
//! data it reads.

use std::time::Duration;

use egui::Key;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use gossia_app::GossiaApp;
use gossia_app::form::View;
use gossia_app::form_exprs::{ChoiceList, choice_list};
use gossia_core::form::{ColumnDesign, Control, ControlKind, Editor};
use gossia_core::{Path, Value};

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

fn p(text: &str) -> Path {
    text.parse().unwrap()
}

fn value(harness: &Harness<'_, GossiaApp>, path: &str) -> Option<Value> {
    harness
        .state()
        .calc()
        .data("Orders")?
        .get(&p(path))
        .map(|n| n.value().clone())
}

fn set(harness: &mut Harness<'_, GossiaApp>, path: &str, value: Value) {
    harness
        .state_mut()
        .session
        .calc_mut()
        .set_value("Orders", &p(path), value)
        .unwrap();
    settle(harness);
}

/// The Default form of Orders, with the field of `path` changed by `change`;
/// gives the field's address.
fn form(
    path: &str,
    change: impl FnOnce(&mut Control),
) -> (Harness<'static, GossiaApp>, Vec<usize>) {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 1000.0))
        .build_ui_state(|ui, app: &mut GossiaApp| app.show(ui), GossiaApp::sample());
    settle(&mut harness);
    let at;
    {
        let app = harness.state_mut();
        let mut design = app.design_of("Default");
        at = design
            .walk()
            .into_iter()
            .find(|(_, c)| c.kind == ControlKind::Field && c.bound_path() == Some(p(path)))
            .map(|(at, _)| at)
            .expect("the Default form has the field");
        change(design.get_mut(&at).unwrap());
        app.session
            .calc_mut()
            .forms_mut()
            .set("Orders", "Default", design);
        assert!(app.set_view(View::Form("Default".to_owned())));
    }
    settle(&mut harness);
    (harness, at)
}

fn list(harness: &Harness<'_, GossiaApp>, at: &[usize]) -> ChoiceList {
    choice_list(harness.state().calc(), "Orders", "Default", at, None, &[])
        .expect("the field has choices")
}

fn texts(list: &ChoiceList) -> Vec<String> {
    list.items
        .as_ref()
        .unwrap()
        .iter()
        .map(|c| c.text.clone())
        .collect()
}

const CITIES: &str = "[{'value': c, 'label': n} for c, n in \
                      [('UT', 'Utrecht'), ('AMS', 'Amsterdam'), ('RTM', 'Rotterdam')]]";

const CITY: &str = "Orders.customer.city";

#[test]
fn a_dropdown_shows_the_text_of_an_item_and_stores_its_code() {
    let (mut harness, _) = form("customer.city", |c| {
        c.choices = Some("[('UT', 'Utrecht'), ('AMS', 'Amsterdam')]".into());
    });
    // The sample's city, "Utrecht", is not one of the codes: kept, and marked.
    assert!(city_text(&harness).contains("Utrecht (not in the list)"));
    assert!(harness.query_by_label("not in the list").is_some());

    harness.get_by_label(CITY).click();
    harness.run();
    // An item shows its code and its text.
    harness.get_by_label("AMS \u{2014} Amsterdam").click();
    harness.run();
    settle(&mut harness);
    assert_eq!(
        value(&harness, "customer.city"),
        Some(Value::Str("AMS".into()))
    );
    assert!(harness.query_by_label("not in the list").is_none());
    assert_eq!(
        harness
            .get_by_label(CITY)
            .accesskit_node()
            .value()
            .as_deref(),
        Some("AMS \u{2014} Amsterdam")
    );

    // Undo is an ordinary edit of the tab.
    harness.state_mut().undo();
    settle(&mut harness);
    assert_eq!(
        value(&harness, "customer.city"),
        Some(Value::Str("Utrecht".into()))
    );
}

fn typeahead(allow_other: bool) -> Harness<'static, GossiaApp> {
    let (mut harness, _) = form("customer.city", |c| {
        c.choices = Some(CITIES.into());
        c.editor = Some(Editor::Typeahead);
        c.allow_other = allow_other;
    });
    set(&mut harness, "customer.city", Value::Str("UT".into()));
    harness
}

fn type_city(harness: &mut Harness<'_, GossiaApp>, text: &str) {
    harness.get_by_label(CITY).click();
    harness.run();
    harness.key_combination_modifiers(egui::Modifiers::COMMAND, &[Key::A]);
    harness.get_by_label(CITY).type_text(text);
    harness.run();
}

fn city_text(harness: &Harness<'_, GossiaApp>) -> String {
    harness
        .get_by_label(CITY)
        .accesskit_node()
        .value()
        .unwrap_or_default()
}

#[test]
fn a_typeahead_lists_the_matches_and_takes_one_with_the_arrows_and_enter() {
    let mut harness = typeahead(false);
    assert_eq!(city_text(&harness), "Utrecht");
    type_city(&mut harness, "dam");
    // Both contain it; in the order of the list.
    assert!(harness.query_by_label("Amsterdam").is_some());
    assert!(harness.query_by_label("Rotterdam").is_some());
    assert!(harness.query_by_label("Utrecht").is_none());
    harness.key_press(Key::ArrowDown);
    harness.run();
    harness.key_press(Key::Enter);
    harness.run();
    settle(&mut harness);
    assert_eq!(
        value(&harness, "customer.city"),
        Some(Value::Str("RTM".into()))
    );
    assert_eq!(city_text(&harness), "Rotterdam");
    assert!(
        harness.query_by_label("Amsterdam").is_none(),
        "the list closed"
    );
}

#[test]
fn a_typeahead_takes_the_item_whose_text_was_typed_and_refuses_other_text() {
    let mut harness = typeahead(false);
    // Typed in full, and left with Tab.
    type_city(&mut harness, "amsterdam");
    harness.key_press(Key::Tab);
    harness.run();
    settle(&mut harness);
    assert_eq!(
        value(&harness, "customer.city"),
        Some(Value::Str("AMS".into()))
    );

    type_city(&mut harness, "Paris");
    harness.key_press(Key::Tab);
    harness.run();
    settle(&mut harness);
    assert_eq!(
        value(&harness, "customer.city"),
        Some(Value::Str("AMS".into()))
    );
    assert!(
        harness
            .query_by_label_contains("is not one of the choices")
            .is_some()
    );
    assert_eq!(
        city_text(&harness),
        "Paris",
        "the text stays to be corrected"
    );
}

#[test]
fn allow_other_values_enters_typed_text_as_it_is() {
    let mut harness = typeahead(true);
    type_city(&mut harness, "Paris");
    harness.key_press(Key::Tab);
    harness.run();
    settle(&mut harness);
    assert_eq!(
        value(&harness, "customer.city"),
        Some(Value::Str("Paris".into()))
    );
    // Not one of the items: kept, and marked.
    assert!(harness.query_by_label("not in the list").is_some());
}

#[test]
fn a_list_that_reads_another_field_follows_it_and_an_enumeration_is_the_rule() {
    let (mut harness, at) = form("customer.status", |c| {
        c.choices = Some(
            "['new', 'paid'] if Orders.customer.vip else [('new', 'New'), ('shipped', 'Sent'), \
             ('lost', 'Lost')]"
                .into(),
        );
    });
    assert_eq!(texts(&list(&harness, &at)), ["new", "paid"]);
    set(&mut harness, "customer.vip", Value::Bool(false));
    assert_eq!(texts(&list(&harness, &at)), ["New", "Sent", "Lost"]);
    // "lost" is not a value of the enumeration: the dropdown leaves it out.
    harness.get_by_label("Orders.customer.status").click();
    harness.run();
    assert!(harness.query_by_label("shipped \u{2014} Sent").is_some());
    assert!(harness.query_by_label("lost \u{2014} Lost").is_none());
    harness.get_by_label("shipped \u{2014} Sent").click();
    harness.run();
    settle(&mut harness);
    assert_eq!(
        value(&harness, "customer.status"),
        Some(Value::Str("shipped".into()))
    );
}

#[test]
fn without_trust_or_when_it_fails_the_field_is_a_text_box_of_the_code() {
    let (mut harness, at) = form("customer.city", |c| {
        c.choices = Some(CITIES.into());
    });
    let role = |h: &Harness<'_, GossiaApp>| h.get_by_label(CITY).accesskit_node().role();
    assert_eq!(role(&harness), egui::accesskit::Role::ComboBox);
    harness.state_mut().session.calc_mut().set_trusted(false);
    settle(&mut harness);
    assert_eq!(role(&harness), egui::accesskit::Role::TextInput);
    assert!(list(&harness, &at).items.unwrap_err().contains("trusted"));

    harness.state_mut().session.calc_mut().set_trusted(true);
    settle(&mut harness);
    set_choices(&mut harness, &at, "Orders.customer.nothing");
    assert_eq!(role(&harness), egui::accesskit::Role::TextInput);
    let error = list(&harness, &at).items.unwrap_err();
    assert!(
        error.starts_with("Choices:") && error.contains("nothing"),
        "{error}"
    );

    set_choices(&mut harness, &at, "[[1, 2, 3]]");
    let error = list(&harness, &at).items.unwrap_err();
    assert!(error.contains("item 0 is not"), "{error}");
}

fn set_choices(harness: &mut Harness<'_, GossiaApp>, at: &[usize], text: &str) {
    let app = harness.state_mut();
    let mut design = app.design_of("Default");
    design.get_mut(at).unwrap().choices = Some(text.into());
    app.session
        .calc_mut()
        .forms_mut()
        .set("Orders", "Default", design);
    settle(harness);
}

#[test]
fn the_choices_of_a_column_run_for_each_row() {
    let (mut harness, _) = form("customer.city", |_| {});
    let at;
    {
        let app = harness.state_mut();
        let mut design = app.design_of("Default");
        at = design
            .walk()
            .into_iter()
            .find(|(_, c)| c.kind == ControlKind::Repeater && c.bound_path() == Some(p("items")))
            .map(|(at, _)| at)
            .expect("the Default form has a table of items");
        let mut note = ColumnDesign::named("note");
        note.choices = Some("[parent.sku + ' gift', parent.sku + ' rush']".into());
        design.get_mut(&at).unwrap().table_columns = vec![note];
        app.session
            .calc_mut()
            .forms_mut()
            .set("Orders", "Default", design);
    }
    settle(&mut harness);
    let row = |h: &Harness<'_, GossiaApp>, row: usize| {
        let list = choice_list(
            h.state().calc(),
            "Orders",
            "Default",
            &at,
            Some(("note", row)),
            &[],
        )
        .expect("the column has choices");
        texts(&list)
    };
    assert_eq!(row(&harness, 1), ["B-220 gift", "B-220 rush"]);
    assert_eq!(row(&harness, 2), ["C-310 gift", "C-310 rush"]);
    set(&mut harness, "items[2].sku", Value::Str("D-400".into()));
    assert_eq!(row(&harness, 2), ["D-400 gift", "D-400 rush"]);
}

#[test]
fn the_property_page_sets_the_choices_and_counts_what_an_enumeration_leaves_out() {
    let (mut harness, at) = form("customer.status", |_| {});
    harness.get_by_label("Edit form").click();
    harness.run();
    harness.get_by_label("Field customer.status").click();
    harness.run();
    assert!(
        harness
            .query_by_label("Allow other values (write-in codes)")
            .is_none()
    );
    let role = egui::accesskit::Role::TextInput;
    harness.get_by_role_and_label(role, "Choices").click();
    harness.run();
    harness
        .get_by_role_and_label(role, "Choices")
        .type_text("['new', 'paid', 'lost', 'gone']");
    harness.run();
    harness.key_press(egui::Key::Enter);
    harness.run();
    settle(&mut harness);
    harness.run();
    let design = harness.state().design_of("Default");
    assert_eq!(
        design.get(&at).unwrap().choices.as_deref(),
        Some("['new', 'paid', 'lost', 'gone']")
    );
    assert!(
        harness
            .query_by_label("2 items are not values of the enumeration and are left out")
            .is_some()
    );
    harness
        .get_by_label("Allow other values (write-in codes)")
        .click();
    harness.run();
    assert!(
        harness
            .state()
            .design_of("Default")
            .get(&at)
            .unwrap()
            .allow_other
    );
}
