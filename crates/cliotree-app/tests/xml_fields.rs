//! The field commands on an XML tab, in the window (DESIGN.md, "Editing the
//! schema from a data tab", "Attributes and elements"): Add field asks for an
//! element or an attribute, and an attribute is renamed without its `@`.

use std::sync::Arc;
use std::time::Duration;

use cliotree_app::ClioApp;
use cliotree_core::{Path, metaschema, xml, xsd};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

const XSD: &str = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
  <xs:element name="order"><xs:complexType><xs:sequence>
    <xs:element name="customer"><xs:complexType><xs:sequence>
      <xs:element name="name" type="xs:string"/>
      <xs:element name="city" type="xs:string"/>
    </xs:sequence><xs:attribute name="id" type="xs:integer"/></xs:complexType></xs:element>
  </xs:sequence></xs:complexType></xs:element>
</xs:schema>"#;

const XML: &str =
    r#"<order><customer id="7"><name>Acme</name><city>Delft</city></customer></order>"#;

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
    calc.set_auto_update("S", true).unwrap();
    let app = ClioApp::new(calc);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1000.0, 600.0))
        .build_ui_state(|ui, app: &mut ClioApp| app.show(ui), app);
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

fn fields(harness: &Harness<'_, ClioApp>) -> Vec<String> {
    harness
        .state()
        .calc()
        .field_places("Order", &"customer".parse::<Path>().unwrap())
        .unwrap()
        .fields
}

fn menu_item(harness: &mut Harness<'_, ClioApp>, cell: &str, item: &str) {
    harness.get_by_label(cell).click_secondary();
    harness.run();
    harness.get_by_label(item).click();
    harness.run();
}

#[test]
fn add_field_on_an_xml_tab_offers_an_attribute() {
    let mut harness = window();
    assert_eq!(fields(&harness), ["@id", "name", "city"]);
    menu_item(&mut harness, "customer", "Add field...");
    harness.get_by_label("Attribute").click();
    harness.run();
    harness.get_by_label("Field name").focus();
    harness.run();
    harness.get_by_label("Field name").type_text("lang");
    harness.run();
    harness.get_by_label("Add").click();
    harness.run();
    settle(&mut harness);
    assert!(
        harness.query_by_label("Field name").is_none(),
        "{:?}",
        harness.state().schema_dialog
    );
    assert_eq!(fields(&harness), ["@id", "@lang", "name", "city"]);

    // A field added as an element goes after the other elements.
    menu_item(&mut harness, "customer", "Add field...");
    harness.get_by_label("Field name").focus();
    harness.run();
    harness.get_by_label("Field name").type_text("note");
    harness.run();
    harness.get_by_label("Add").click();
    harness.run();
    settle(&mut harness);
    assert_eq!(fields(&harness), ["@id", "@lang", "name", "city", "note"]);
}

#[test]
fn an_attribute_is_renamed_without_its_at_sign() {
    let mut harness = window();
    harness
        .state_mut()
        .set_open(&"customer".parse::<Path>().unwrap(), true);
    harness.run();
    menu_item(&mut harness, "@id", "Rename field...");
    let name = harness.get_by_label("New field name");
    name.focus();
    harness.run();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.get_by_label("New field name").type_text("number");
    harness.run();
    harness.get_by_label("Rename").click();
    harness.run();
    settle(&mut harness);
    assert_eq!(fields(&harness), ["@number", "name", "city"]);
}
