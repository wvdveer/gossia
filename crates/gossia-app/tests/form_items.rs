//! The design of a stacked repeater's items (DESIGN.md, "Forms", "Designed
//! items"): controls whose paths are read from the item shown, and whose
//! expressions run for each item.

use std::time::Duration;

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use gossia_app::GossiaApp;
use gossia_app::form::View;
use gossia_core::Path;
use gossia_core::form::{self as design, Address, Control, ControlKind, RepeaterMode};

fn p(text: &str) -> Path {
    text.parse().unwrap()
}

fn settle(harness: &mut Harness<'_, GossiaApp>) {
    assert!(
        harness
            .state_mut()
            .session
            .wait_idle(Duration::from_secs(60))
    );
    harness.run();
}

fn harness() -> Harness<'static, GossiaApp> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 900.0))
        .build_ui_state(|ui, app: &mut GossiaApp| app.show(ui), GossiaApp::sample());
    settle(&mut harness);
    harness
}

/// Where the repeater of items is on the Default form.
fn repeater_at(app: &GossiaApp) -> Address {
    app.design_of("Default")
        .walk()
        .into_iter()
        .find(|(_, c)| c.kind == ControlKind::Repeater && c.bound_path() == Some(p("items")))
        .map(|(at, _)| at)
        .unwrap()
}

/// The items of Orders shown stacked, each as a label, its sku, and its
/// price only when more than one is ordered.
fn designed() -> Harness<'static, GossiaApp> {
    let mut harness = harness();
    {
        let app = harness.state_mut();
        let at = repeater_at(app);
        let mut form = app.design_of("Default");
        let repeater = form.get_mut(&at).unwrap();
        repeater.mode = Some(RepeaterMode::Stacked);
        let mut label = Control::new(ControlKind::Label);
        label.text = Some("One line of the order".into());
        let sku = Control::bound(ControlKind::Field, &p("sku"));
        let mut price = Control::bound(ControlKind::Field, &p("price"));
        price.visible_when = Some("parent.qty > 1".into());
        repeater.children = vec![label, sku, price];
        app.session
            .calc_mut()
            .forms_mut()
            .set("Orders", "Default", form);
        assert!(app.set_view(View::Form("Default".to_owned())));
    }
    settle(&mut harness);
    harness.run();
    harness
}

#[test]
fn the_item_shown_is_drawn_by_its_design() {
    let mut harness = designed();
    harness.get_by_label("One line of the order");
    assert!(harness.query_by_label("Orders.items[0].sku").is_some());
    // qty is 2: the price is shown. The fields left out of the design are not.
    assert!(harness.query_by_label("Orders.items[0].price").is_some());
    assert!(harness.query_by_label("Orders.items[0].qty").is_none());
    harness.get_by_label("Next item").click();
    settle(&mut harness);
    harness.get_by_label("2 of 3");
    assert!(harness.query_by_label("Orders.items[1].sku").is_some());
    // qty is 1: the price is hidden, for this item only.
    assert!(harness.query_by_label("Orders.items[1].price").is_none());
}

#[test]
fn the_expressions_of_the_design_run_for_each_item() {
    let mut harness = designed();
    let app = harness.state();
    let at = {
        let mut at = repeater_at(app);
        at.push(2);
        at
    };
    let visible = |app: &GossiaApp, item| {
        app.calc()
            .control_expression_in(
                "Orders",
                "Default",
                &at,
                Some(item),
                gossia_calc::ExprKind::Visible,
            )
            .and_then(|s| s.result)
            .and_then(Result::ok)
    };
    use gossia_core::Value;
    assert_eq!(visible(app, 0), Some(Value::Bool(true)));
    assert_eq!(visible(app, 1), Some(Value::Bool(false)));
    assert_eq!(visible(app, 2), Some(Value::Bool(true)));
    // An edit in one item runs its expression again.
    harness.state_mut().enter_directly(&p("items[1].qty"), "5");
    settle(&mut harness);
    assert_eq!(visible(harness.state(), 1), Some(Value::Bool(true)));
    // An item added gets its own.
    harness.state_mut().add_row(&p("items"), None);
    settle(&mut harness);
    assert!(visible(harness.state(), 3).is_some());
}

#[test]
fn designing_the_items_starts_from_the_generated_look() {
    let mut harness = harness();
    let at = repeater_at(harness.state());
    {
        let app = harness.state_mut();
        assert!(app.set_view(View::Form("Default".to_owned())));
        app.set_layout(true);
        app.select_control(Some(at.clone()));
    }
    harness.run();
    harness.run();
    harness.get_by_label("Design the items").click();
    harness.run();
    let form = harness.state().design_of("Default");
    let repeater = form.get(&at).unwrap();
    assert_eq!(repeater.mode, Some(RepeaterMode::Stacked));
    let paths: Vec<Option<&str>> = repeater
        .children
        .iter()
        .map(|c| c.path.as_deref())
        .collect();
    assert_eq!(
        paths,
        [
            Some("sku"),
            Some("qty"),
            Some("price"),
            Some("total"),
            Some("note")
        ]
    );
    assert!(
        form.broken(harness.state().calc().spec("Orders").unwrap())
            .is_empty()
    );

    // A control added with the repeater selected goes into the design.
    assert!(
        harness
            .state_mut()
            .add_control(Control::new(ControlKind::Separator))
    );
    let form = harness.state().design_of("Default");
    assert_eq!(form.get(&at).unwrap().children.len(), 6);
    // A field not on the form is added after the repeater, not into it.
    harness.state_mut().select_control(Some(at.clone()));
    let name =
        design::control_for(harness.state().calc().spec("Orders").unwrap(), &p("tax")).unwrap();
    assert!(name.path.as_deref() == Some("tax"));
    assert!(harness.state_mut().add_field_to_form(&p("tax")));
    let form = harness.state().design_of("Default");
    let mut after = at.clone();
    *after.last_mut().unwrap() += 1;
    assert_eq!(form.get(&after).unwrap().path.as_deref(), Some("tax"));

    // Back to the generated items.
    harness.state_mut().select_control(Some(at.clone()));
    harness.run();
    harness.get_by_label("Use the generated items").click();
    harness.run();
    let form = harness.state().design_of("Default");
    assert!(form.get(&at).unwrap().children.is_empty());
}

#[test]
fn in_layout_mode_the_controls_of_the_items_can_be_selected() {
    let mut harness = designed();
    let at = repeater_at(harness.state());
    harness.state_mut().set_layout(true);
    harness.run();
    harness.run();
    // The controls of the design are drawn, the label among them.
    harness.get_by_label("One line of the order");
    let mut sku = at.clone();
    sku.push(1);
    harness.state_mut().select_control(Some(sku));
    harness.run();
    // Its path is offered from the item.
    harness.get_by_label("In the design of the items: paths are read from the item.");
}
