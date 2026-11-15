//! The design of the rows of a repeater section (DESIGN.md, Form canvases,
//! 0.7.0): controls whose paths are read from the row's item, and whose
//! expressions run for each item.

use std::time::Duration;

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use gossia_app::GossiaApp;
use gossia_app::form::View;
use gossia_core::Path;
use gossia_core::form::{Address, Control, ControlKind};

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

/// The rows of the repeater section of the items of Orders designed as a
/// label, the sku, and the price only when more than one is ordered.
fn designed() -> Harness<'static, GossiaApp> {
    let mut harness = harness();
    {
        let app = harness.state_mut();
        let at = repeater_at(app);
        let mut form = app.design_of("Default");
        let repeater = form.get_mut(&at).unwrap();
        let place = |mut c: Control, x: u32| {
            c.x = Some(x);
            c.y = Some(4);
            c.width = Some(140);
            c
        };
        let mut label = Control::new(ControlKind::Label);
        label.text = Some("One line of the order".into());
        let sku = Control::bound(ControlKind::Field, &p("sku"));
        let mut price = Control::bound(ControlKind::Field, &p("price"));
        price.visible_when = Some("parent.qty > 1".into());
        repeater.children = vec![place(label, 8), place(sku, 160), place(price, 320)];
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
fn each_row_is_drawn_by_the_design_of_its_section() {
    let harness = designed();
    assert_eq!(
        harness.query_all_by_label("One line of the order").count(),
        3
    );
    assert!(harness.query_by_label("Orders.items[0].sku").is_some());
    // qty is 2: the price is shown. The fields left out of the design are not.
    assert!(harness.query_by_label("Orders.items[0].price").is_some());
    assert!(harness.query_by_label("Orders.items[0].qty").is_none());
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
