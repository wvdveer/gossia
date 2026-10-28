//! Comments in the window (DESIGN.md, "Comments in the UI"), driven as a user
//! does: the menu of a cell, the comment box, the popup on hover.

use std::sync::Arc;
use std::time::Duration;

use cliotree_app::ClioApp;
use cliotree_core::{Path, metaschema, xml, xsd};
use egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

const XSD: &str = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
  <xs:element name="order"><xs:complexType><xs:sequence>
    <xs:element name="customer" type="xs:string"/>
    <xs:element name="qty" type="xs:integer"/>
  </xs:sequence><xs:attribute name="id" type="xs:integer"/></xs:complexType></xs:element>
</xs:schema>"#;

const XML: &str = r#"<order id="7"><!--Who pays--><customer>Acme</customer><qty>2</qty></order>"#;

fn p(text: &str) -> Path {
    text.parse().unwrap()
}

fn window() -> Harness<'static, ClioApp> {
    let mut calc = cliotree_calc::Calc::new(Arc::new(cliotree_calc::python::PythonSyntax));
    let spec = xsd::lower_str(XSD).unwrap();
    let data = xml::parse(XML, &spec).unwrap();
    calc.add_sheet("Order", spec, data.node).unwrap();
    calc.set_xml_document("Order", data.document).unwrap();
    let schema = xml::parse(XSD, metaschema::xsd_meta()).unwrap();
    calc.add_xsd_sheet("S", schema.node, schema.document)
        .unwrap();
    calc.bind_schema("Order", "S").unwrap();
    let mut app = ClioApp::new(calc);
    // No waiting for the popup: the test clock moves in steps.
    app.user.prefs.display.comment_delay = 0;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1000.0, 600.0))
        .build_ui_state(|ui, app: &mut ClioApp| app.show(ui), app);
    assert!(
        harness
            .state_mut()
            .session
            .wait_idle(Duration::from_secs(60))
    );
    harness.run();
    harness
}

fn comment(harness: &Harness<'_, ClioApp>, path: &str) -> Option<String> {
    harness
        .state()
        .calc()
        .comment("Order", &p(path))
        .map(str::to_owned)
}

fn menu_item(harness: &mut Harness<'_, ClioApp>, cell: &str, item: &str) {
    harness.get_by_label(cell).click_secondary();
    harness.run();
    harness.get_by_label(item).click();
    harness.run();
}

#[test]
fn a_comment_is_inserted_edited_and_deleted_from_the_cells_menu() {
    let mut harness = window();
    assert_eq!(comment(&harness, "customer").as_deref(), Some("Who pays"));

    // Insert: the box opens; Enter is a line break, Ctrl+Enter saves.
    menu_item(&mut harness, "2", "Insert Comment");
    harness.get_by_label("Comment").type_text("Two boxes");
    harness.key_press(Key::Enter);
    harness.run();
    harness.get_by_label("Comment").type_text("not three");
    harness.key_combination_modifiers(Modifiers::COMMAND, &[Key::Enter]);
    harness.run();
    assert!(
        harness.query_by_label("Comment").is_none(),
        "the box closed"
    );
    assert_eq!(
        comment(&harness, "qty").as_deref(),
        Some("Two boxes\nnot three")
    );
    assert!(harness.state().calc().is_modified());

    // Text XML cannot hold keeps the box open with the reason.
    menu_item(&mut harness, "2", "Edit Comment");
    harness.get_by_label("Comment").type_text(" -- really");
    harness.key_combination_modifiers(Modifiers::COMMAND, &[Key::Enter]);
    harness.run();
    assert!(
        harness
            .query_all_by_label_contains("two hyphens")
            .next()
            .is_some()
    );
    // Esc cancels: nothing changed.
    harness.key_press(Key::Escape);
    harness.run();
    assert!(harness.query_by_label("Comment").is_none());
    assert_eq!(
        comment(&harness, "qty").as_deref(),
        Some("Two boxes\nnot three")
    );

    // Delete, and undo brings it back.
    menu_item(&mut harness, "2", "Delete Comment");
    assert_eq!(comment(&harness, "qty"), None);
    harness.key_combination_modifiers(Modifiers::COMMAND, &[Key::Z]);
    harness.run();
    assert_eq!(
        comment(&harness, "qty").as_deref(),
        Some("Two boxes\nnot three")
    );
}

#[test]
fn resting_on_a_cell_shows_its_comment_and_an_attribute_has_none() {
    let mut harness = window();
    assert!(harness.query_by_label("Who pays").is_none());
    harness.get_by_label("Acme").hover();
    harness.run();
    harness.run();
    harness.get_by_label("Who pays");
    // Away from the cell, the popup goes.
    harness.get_by_label("2").hover();
    harness.run();
    assert!(harness.query_by_label("Who pays").is_none());

    // An attribute cannot have one: the item is there, disabled.
    harness.get_by_label("7").click_secondary();
    harness.run();
    assert!(
        harness
            .get_by_label("Insert Comment")
            .accesskit_node()
            .is_disabled()
    );
    harness.key_press(Key::Escape);
    harness.run();

    // A click outside the box saves what was typed.
    menu_item(&mut harness, "Acme", "Edit Comment");
    harness.get_by_label("Comment").type_text(", by card");
    harness.get_by_label("qty").click();
    harness.run();
    assert_eq!(
        comment(&harness, "customer").as_deref(),
        Some("Who pays, by card")
    );
}

#[test]
fn a_json_tab_has_no_comment_commands() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1000.0, 600.0))
        .build_ui_state(|ui, app: &mut ClioApp| app.show(ui), ClioApp::sample());
    assert!(
        harness
            .state_mut()
            .session
            .wait_idle(Duration::from_secs(60))
    );
    harness.run();
    harness.get_by_label("62.92").click_secondary();
    harness.run();
    assert!(harness.query_by_label("Insert Comment").is_none());
    harness.get_by_label("customer").click_secondary();
    harness.run();
    assert!(harness.query_by_label("Insert Comment").is_none());
}
