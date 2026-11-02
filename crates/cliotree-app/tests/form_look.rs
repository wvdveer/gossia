//! The look of a form, in the window (DESIGN.md, Forms, "Additions in 0.2.0,
//! shared with reports"): fixed colours, bold and italic, borders on groups,
//! vertical separators, and percent, prefix and suffix.

use std::time::Duration;

use cliotree_app::ClioApp;
use cliotree_app::form::View;
use cliotree_core::form::{Control, ControlKind};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

fn settle(harness: &mut Harness<'_, ClioApp>) {
    assert!(
        harness
            .state_mut()
            .session
            .wait_idle(Duration::from_secs(60)),
        "the workbook did not settle"
    );
    harness.run();
    harness.run();
}

/// The Default form of Orders with these controls at its end.
fn form(controls: Vec<Control>) -> Harness<'static, ClioApp> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 1800.0))
        .build_ui_state(|ui, app: &mut ClioApp| app.show(ui), ClioApp::sample());
    settle(&mut harness);
    {
        let app = harness.state_mut();
        let mut design = app.design_of("Default");
        design.controls.extend(controls);
        app.session
            .calc_mut()
            .forms_mut()
            .set("Orders", "Default", design);
        assert!(app.set_view(View::Form("Default".to_owned())));
    }
    settle(&mut harness);
    harness
}

fn value_of(harness: &Harness<'_, ClioApp>, label: &str) -> Option<String> {
    harness
        .get_by_role_and_label(egui::accesskit::Role::TextInput, label)
        .accesskit_node()
        .value()
}

fn value_field(label: &str, value: &str) -> Control {
    let mut field = Control::new(ControlKind::Field);
    field.label = Some(label.into());
    field.value = Some(value.into());
    field
}

#[test]
fn percent_prefix_and_suffix_dress_the_numbers_a_field_shows() {
    let mut share = value_field("Share", "0.25");
    share.percent = true;
    let mut price = value_field("Price", "1234.5");
    price.prefix = Some("€ ".into());
    price.thousands = true;
    price.decimals = Some(2);
    let mut weight = value_field("Weight", "3");
    weight.suffix = Some(" kg".into());
    let harness = form(vec![share, price, weight]);
    assert_eq!(value_of(&harness, "Share").as_deref(), Some("25%"));
    assert_eq!(value_of(&harness, "Price").as_deref(), Some("€ 1,234.50"));
    assert_eq!(value_of(&harness, "Weight").as_deref(), Some("3 kg"));
}

/// A fixed colour is picked on the property page: no expression is stored,
/// and switching back to an expression clears it.
#[test]
fn the_property_page_picks_a_fixed_colour() {
    let mut harness = form(Vec::new());
    harness.get_by_label("Edit form").click();
    harness.run();
    harness.get_by_label("Field customer.name").click();
    harness.run();
    let colour = |h: &Harness<'_, ClioApp>| {
        let design = h.state().design_of("Default");
        design
            .walk()
            .into_iter()
            .find(|(_, c)| c.path.as_deref() == Some("customer.name"))
            .and_then(|(_, c)| c.fill_colour.clone())
    };
    let choose = |h: &mut Harness<'_, ClioApp>, kind: &str| {
        h.get_by_label("Kind of fill colour").click();
        h.run();
        h.get_by_role_and_label(egui::accesskit::Role::Button, kind)
            .click();
        h.run();
    };
    choose(&mut harness, "Fixed");
    assert_eq!(colour(&harness).as_deref(), Some("#fff2cc"));
    // A fixed colour has a picker, not an expression box.
    assert!(
        harness
            .query_by_role_and_label(egui::accesskit::Role::TextInput, "Fill colour")
            .is_none()
    );
    choose(&mut harness, "Expression");
    assert_eq!(colour(&harness), None);
    assert!(
        harness
            .query_by_role_and_label(egui::accesskit::Role::TextInput, "Fill colour")
            .is_some()
    );
}

/// A fixed colour shows in an untrusted workbook, where an expression of a
/// colour does not run; bold, italic, a border and a vertical separator draw.
#[test]
fn the_look_draws_in_an_untrusted_workbook() {
    let mut group = Control::new(ControlKind::Group);
    group.label = Some("Boxed".into());
    group.border_width = Some(3);
    group.border_colour = Some("#336699".into());
    group.fill_colour = Some("#eef4ff".into());
    group.bold = Some(true);
    let mut label = Control::new(ControlKind::Label);
    label.text = Some("Slanted".into());
    label.italic = Some(true);
    label.text_colour = Some("#990000".into());
    let mut line = Control::new(ControlKind::Separator);
    line.vertical = true;
    line.height = Some(40);
    group.children = vec![label, line, value_field("Shown", "1")];
    let mut harness = form(vec![group]);
    harness.state_mut().session.calc_mut().set_trusted(false);
    harness.run();
    assert!(harness.query_by_label("Slanted").is_some());
    assert!(harness.query_by_label("Boxed").is_some());
    assert_eq!(value_of(&harness, "Shown").as_deref(), Some("#UNTRUSTED!"));
    // Nothing about the fixed colours needs trust.
    assert!(
        harness
            .query_by_label_contains("has to be trusted first")
            .is_none()
    );
}
