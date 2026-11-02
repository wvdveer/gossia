//! Visual snapshot tests (DESIGN.md, "Testing and CI", level 7). Key screens
//! are rendered with wgpu and compared with stored images with a small
//! tolerance. They need a graphics adapter (a software Vulkan driver, Mesa
//! lavapipe, on Linux CI), so they run only when `CLIOTREE_SNAPSHOTS` is set:
//! `cargo xtask snapshots`.
//!
//! The images differ between operating systems, so each has its own directory
//! under `tests/snapshots/`. `UPDATE_SNAPSHOTS=1` writes the ones that differ.
//! A failing run leaves `*.new.png` and `*.diff.png` beside the stored image,
//! which the CI job uploads.

use std::time::Duration;

use cliotree_app::ClioApp;
use egui::{Key, Modifiers};
use egui_kittest::kittest::Queryable;
use egui_kittest::{Harness, SnapshotOptions, SnapshotResults};

fn enabled() -> bool {
    let on = std::env::var_os("CLIOTREE_SNAPSHOTS").is_some_and(|v| !v.is_empty());
    if !on {
        eprintln!("skipped: set CLIOTREE_SNAPSHOTS=1 (needs a graphics adapter)");
    }
    on
}

fn window() -> Harness<'static, ClioApp> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 700.0))
        .wgpu()
        .build_ui_state(|ui, app: &mut ClioApp| app.show(ui), ClioApp::sample());
    settle(&mut harness);
    harness
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

fn click(harness: &mut Harness<'_, ClioApp>, label: &str) {
    harness.get_by_label(label).click();
    harness.run();
}

/// Chooses a view of the current tab: View, Show as, the view.
fn show_as(harness: &mut Harness<'_, ClioApp>, view: &str) {
    harness.get_by_label("View").click();
    harness.run();
    harness.get_by_label("Show as").click();
    harness.run();
    harness.get_by_label(view).click();
    harness.run();
}

fn options() -> SnapshotOptions {
    let directory = format!("tests/snapshots/{}", std::env::consts::OS);
    // A few pixels may differ between drivers; a moved or missing widget differs
    // by far more.
    SnapshotOptions::new()
        .output_path(directory)
        .max_failed_pixels(50)
}

fn check(harness: &mut Harness<'_, ClioApp>, name: &str, results: &mut SnapshotResults) {
    results.add(harness.try_snapshot_options(name, &options()));
}

#[test]
fn the_sample_workbook() {
    if !enabled() {
        return;
    }
    let mut results = SnapshotResults::new();
    let mut harness = window();
    harness.state_mut().reveal(&"customer".parse().unwrap());
    harness.run();
    check(&mut harness, "tree_of_the_sample", &mut results);

    click(&mut harness, "Prices");
    check(&mut harness, "prices_tab", &mut results);
}

#[test]
fn the_tabular_view_and_its_dialogs() {
    if !enabled() {
        return;
    }
    let mut results = SnapshotResults::new();
    let mut harness = window();
    harness
        .state_mut()
        .set_tabular(&"items".parse().unwrap(), true);
    harness.run();
    settle(&mut harness);
    check(&mut harness, "table_of_items", &mut results);

    harness.get_by_label("sku").click_secondary();
    harness.run();
    check(&mut harness, "column_menu", &mut results);
    click(&mut harness, "Filter...");
    check(&mut harness, "filter_dialog", &mut results);
}

#[test]
fn errors_are_marked_and_listed() {
    if !enabled() {
        return;
    }
    let mut results = SnapshotResults::new();
    let mut harness = window();
    click(&mut harness, "0.21");
    harness.event(egui::Event::Text("=1 / 0".to_owned()));
    harness.run();
    harness.key_press(Key::Enter);
    harness.run();
    settle(&mut harness);
    click(&mut harness, "View");
    click(&mut harness, "Issues panel");
    check(&mut harness, "errors_and_issues", &mut results);
}

#[test]
fn the_insert_function_dialog_and_completion() {
    if !enabled() {
        return;
    }
    let mut results = SnapshotResults::new();
    let mut harness = window();
    click(&mut harness, "0.21");
    harness.event(egui::Event::Text("=Orders.".to_owned()));
    harness.run();
    check(&mut harness, "completion", &mut results);
    harness.key_combination_modifiers(Modifiers::NONE, &[Key::Escape]);
    harness.run();

    click(&mut harness, "fx");
    check(&mut harness, "insert_function", &mut results);
}

#[test]
fn a_schema_tab_and_its_field_dialog() {
    if !enabled() {
        return;
    }
    let mut results = SnapshotResults::new();
    let mut harness = window();
    click(&mut harness, "Orders_schema");
    check(&mut harness, "schema_tab", &mut results);
    click(&mut harness, "Auto-update");
    click(&mut harness, "Orders");
    harness.get_by_label("customer").click_secondary();
    harness.run();
    click(&mut harness, "Add field...");
    check(&mut harness, "add_field_dialog", &mut results);
}

/// The Default form of Orders with its first control coloured by expressions
/// and the second hidden by one.
#[test]
fn a_form_with_expressions_on_its_controls() {
    if !enabled() {
        return;
    }
    let mut results = SnapshotResults::new();
    let mut harness = window();
    {
        let app = harness.state_mut();
        let mut design = app.design_of("Default");
        design.controls[0].fill_colour =
            Some("'#fff2cc' if Orders.customer.vip else None".to_owned());
        design.controls[0].text_colour = Some("'darkred'".to_owned());
        design.controls[1].visible_when = Some("not Orders.customer.vip".to_owned());
        app.session
            .calc_mut()
            .forms_mut()
            .set("Orders", "Default", design);
    }
    show_as(&mut harness, "Default");
    settle(&mut harness);
    check(&mut harness, "form_expressions", &mut results);
}

/// The look of 0.2.0 on the Default form of Orders: fixed colours, bold and
/// italic, a group with a border and fill, a vertical separator, and percent,
/// prefix and suffix.
#[test]
fn a_form_with_its_look() {
    use cliotree_core::form::{Control, ControlKind};
    if !enabled() {
        return;
    }
    let mut results = SnapshotResults::new();
    let mut harness = window();
    {
        let app = harness.state_mut();
        let mut design = app.design_of("Default");
        let value = |label: &str, value: &str| {
            let mut field = Control::new(ControlKind::Field);
            field.label = Some(label.into());
            field.value = Some(value.into());
            field
        };
        let mut group = Control::new(ControlKind::Group);
        group.label = Some("Totals".into());
        group.columns = Some(3);
        group.border_width = Some(2);
        group.border_colour = Some("#336699".into());
        group.fill_colour = Some("#eef4ff".into());
        let mut share = value("Share", "0.125");
        share.percent = true;
        share.decimals = Some(1);
        share.bold = Some(true);
        let mut price = value("Price", "1234.5");
        price.prefix = Some("€ ".into());
        price.thousands = true;
        price.decimals = Some(2);
        price.text_colour = Some("#990000".into());
        let mut weight = value("Weight", "3");
        weight.suffix = Some(" kg".into());
        weight.italic = Some(true);
        let mut note = Control::new(ControlKind::Label);
        note.text = Some("Bold and italic, in serif".into());
        note.font = Some(cliotree_core::form::FontFamily::Serif);
        note.bold = Some(true);
        note.italic = Some(true);
        let mut line = Control::new(ControlKind::Separator);
        line.vertical = true;
        let mut after = Control::new(ControlKind::Label);
        after.text = Some("after a vertical line".into());
        after.fill_colour = Some("#fff2cc".into());
        group.children = vec![share, price, weight, note, line, after];
        design.controls.insert(0, group);
        app.session
            .calc_mut()
            .forms_mut()
            .set("Orders", "Default", design);
    }
    show_as(&mut harness, "Default");
    settle(&mut harness);
    check(&mut harness, "form_look", &mut results);
}

#[test]
fn a_table_with_expressions_on_its_columns() {
    if !enabled() {
        return;
    }
    let mut results = SnapshotResults::new();
    let mut harness = window();
    {
        let app = harness.state_mut();
        let mut design = app.design_of("Default");
        let at = design
            .walk()
            .into_iter()
            .find(|(_, c)| c.kind == cliotree_core::form::ControlKind::Repeater)
            .map(|(at, _)| at)
            .unwrap();
        let column = |name: &str| cliotree_core::form::ColumnDesign::named(name);
        let mut qty = column("qty");
        qty.fill_colour = Some("'#ffd6d6' if this > 5 else None".to_owned());
        qty.text_colour = Some("'darkred' if this > 5 else None".to_owned());
        let mut sku = column("sku");
        sku.enabled_when = Some("parent.qty < 10".to_owned());
        let mut note = column("note");
        note.visible_when = Some("False".to_owned());
        let mut total = column("total");
        total.fill_colour = Some("'lightgreen' if this > 20 else None".to_owned());
        design.get_mut(&at).unwrap().table_columns = vec![sku, qty, note, total];
        app.session
            .calc_mut()
            .forms_mut()
            .set("Orders", "Default", design);
    }
    show_as(&mut harness, "Default");
    settle(&mut harness);
    check(&mut harness, "table_column_expressions", &mut results);
}

#[test]
fn the_default_form() {
    if !enabled() {
        return;
    }
    let mut results = SnapshotResults::new();
    let mut harness = window();
    show_as(&mut harness, "Default");
    check(&mut harness, "default_form_orders", &mut results);

    click(&mut harness, "Prices");
    show_as(&mut harness, "Default");
    check(&mut harness, "default_form_prices", &mut results);
}

#[test]
fn the_raw_view() {
    if !enabled() {
        return;
    }
    let mut results = SnapshotResults::new();
    let mut harness = window();
    show_as(&mut harness, "Raw");
    check(&mut harness, "raw_view_orders", &mut results);
}

#[test]
fn layout_mode_and_a_custom_form() {
    if !enabled() {
        return;
    }
    let mut results = SnapshotResults::new();
    let mut harness = window();
    show_as(&mut harness, "Default");
    harness.get_by_label("Edit form").click();
    harness.run();
    click(&mut harness, "Field customer.name");
    check(&mut harness, "layout_mode_orders", &mut results);

    // A form of its own: the customer group in two columns, no layout mode.
    harness.get_by_label("Edit form").click();
    harness.run();
    harness.state_mut().edit_design(|d| {
        d.get_mut(&[0])?.columns = Some(2);
        Some(None)
    });
    harness.run();
    check(&mut harness, "two_column_form_orders", &mut results);
}

#[test]
fn the_options_dialog() {
    if !enabled() {
        return;
    }
    let mut results = SnapshotResults::new();
    let mut harness = window();
    click(&mut harness, "Tools");
    click(&mut harness, "Options...");
    click(&mut harness, "Evaluation");
    // A workbook value above its ceiling, which the page explains.
    harness.state_mut().options.as_mut().unwrap().texts.insert(
        cliotree_app::options::Field::Workbook(cliotree_calc::Setting::FormulaTimeLimit),
        "90".to_owned(),
    );
    harness.run();
    check(&mut harness, "options_evaluation", &mut results);
    click(&mut harness, "About");
    check(&mut harness, "options_about", &mut results);
}

#[test]
fn the_code_editor() {
    if !enabled() {
        return;
    }
    let mut results = SnapshotResults::new();
    let mut app = ClioApp::sample();
    let calc = app.session.calc_mut();
    calc.add_module(
        "Tax",
        "import math\n\n\ndef vat(x, rate=0.21):\n    \"\"\"The VAT on an amount.\"\"\"\n    return round(x * rate, 2)\n\n\ndef gross(x):\n    # The amount with VAT.\n    return x + vat(x)\n",
    )
    .unwrap();
    calc.add_module("Broken", "def oops(:\n    pass\n").unwrap();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 700.0))
        .wgpu()
        .build_ui_state(|ui, app: &mut ClioApp| app.show(ui), app);
    settle(&mut harness);
    harness.key_combination_modifiers(Modifiers::ALT, &[Key::F11]);
    harness.run();
    // No blinking cursor in the image.
    harness.ctx.memory_mut(|m| {
        if let Some(id) = m.focused() {
            m.surrender_focus(id);
        }
    });
    harness.run();
    check(&mut harness, "code_editor", &mut results);
}

/// An XML tab with comments: the triangle on two cells, and the popup of
/// one of them.
#[test]
fn comments_on_an_xml_tab() {
    use cliotree_core::{metaschema, xml, xsd};
    if !enabled() {
        return;
    }
    const XSD: &str = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
  <xs:element name="order"><xs:complexType><xs:sequence>
    <xs:element name="customer" type="xs:string"/>
    <xs:element name="qty" type="xs:integer"/>
    <xs:element name="note" type="xs:string"/>
  </xs:sequence></xs:complexType></xs:element></xs:schema>"#;
    const XML: &str = "<order><!--Who pays: the account holder, not the person who \
        placed the order, unless the account says otherwise.--><customer>Acme</customer>\
        <!--Boxes--><qty>2</qty><note>Leave at the door</note></order>";
    let mut calc =
        cliotree_calc::Calc::new(std::sync::Arc::new(cliotree_calc::python::PythonSyntax));
    let spec = xsd::lower_str(XSD).unwrap();
    let data = xml::parse(XML, &spec).unwrap();
    calc.add_sheet("Order", spec, data.node).unwrap();
    calc.set_xml_document("Order", data.document).unwrap();
    let schema = xml::parse(XSD, metaschema::xsd_meta()).unwrap();
    calc.add_xsd_sheet("S", schema.node, schema.document)
        .unwrap();
    calc.bind_schema("Order", "S").unwrap();
    let mut app = ClioApp::new(calc);
    app.user.prefs.display.comment_delay = 0;
    app.user.prefs.display.comment_width = 260;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 500.0))
        .wgpu()
        .build_ui_state(|ui, app: &mut ClioApp| app.show(ui), app);
    settle(&mut harness);
    harness.get_by_label("Acme").hover();
    harness.run();
    harness.run();
    let mut results = SnapshotResults::new();
    check(&mut harness, "comments", &mut results);
}

/// *New tab...* with Report chosen, and the page of a report tab, with the
/// chart on its tab.
#[test]
fn a_report_tab() {
    if !enabled() {
        return;
    }
    let mut results = SnapshotResults::new();
    let mut harness = window();
    harness.state_mut().begin_new_report();
    harness.run();
    harness.run();
    check(&mut harness, "new_report_dialog", &mut results);
    click(&mut harness, "Create");
    settle(&mut harness);
    check(&mut harness, "report_tab", &mut results);
}

/// A single page report drawn from its design: a title, a group of two
/// columns with fields, and a table of the items of Orders.
#[test]
fn a_drawn_report() {
    use cliotree_core::form::{Control, ControlKind};
    if !enabled() {
        return;
    }
    let mut results = SnapshotResults::new();
    let mut harness = window();
    {
        let app = harness.state_mut();
        app.create_report_tab(
            "Sales",
            Default::default(),
            Some(&("Orders".to_owned(), "items".parse().unwrap())),
        )
        .unwrap();
        let mut design = app.calc().report("Sales").unwrap().clone();
        let mut title = Control::new(ControlKind::Label);
        title.text = Some("Order for {Orders.customer.name}".into());
        title.template = true;
        title.font_size = Some(20);
        title.bold = Some(true);
        let field = |label: &str, value: &str| {
            let mut f = Control::new(ControlKind::Field);
            f.label = Some(label.into());
            f.value = Some(value.into());
            f.label_width = Some(90);
            f
        };
        let mut totals = Control::new(ControlKind::Group);
        totals.label = Some("Totals".into());
        totals.columns = Some(2);
        totals.border_width = Some(1);
        totals.fill_colour = Some("#eef4ff".into());
        let mut total = field("Total", "Orders.total");
        total.prefix = Some("€ ".into());
        total.decimals = Some(2);
        total.bold = Some(true);
        totals.children = vec![
            field("Subtotal", "Orders.subtotal"),
            field("Tax", "Orders.tax"),
            field("City", "Orders.customer.city"),
            total,
        ];
        design.layout.controls.insert(0, totals);
        design.layout.controls.insert(0, title);
        app.session.calc_mut().set_report("Sales", design).unwrap();
    }
    for _ in 0..3 {
        settle(&mut harness);
    }
    check(&mut harness, "report_drawn", &mut results);
}
