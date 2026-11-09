//! An ordered list of child elements (`#children`) in the window (DESIGN.md,
//! XML Schemas and namespaces, 0.5.0, step 15c-2): an item is one row, its
//! element, and adding an item asks which element.

use std::sync::Arc;
use std::time::Duration;

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use gossia_app::GossiaApp;
use gossia_app::form::View;
use gossia_core::{Path, Value, metaschema, xml, xsd};

const XSD: &str = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
  <xs:element name="svg"><xs:complexType>
    <xs:choice minOccurs="0" maxOccurs="unbounded">
      <xs:element name="rect"><xs:complexType>
        <xs:attribute name="w" type="xs:integer"/>
      </xs:complexType></xs:element>
      <xs:element name="title" type="xs:string"/>
    </xs:choice>
    <xs:attribute name="id" type="xs:string"/>
  </xs:complexType></xs:element>
</xs:schema>"#;

const XML: &str = r#"<svg id="s"><rect w="1"/><title>Hello</title></svg>"#;

fn window() -> Harness<'static, GossiaApp> {
    let mut calc = gossia_calc::Calc::new(Arc::new(gossia_calc::python::PythonSyntax));
    let spec = xsd::lower_str(XSD).unwrap();
    let data = xml::parse(XML, &spec).unwrap();
    calc.add_sheet("Pic", spec, data.node).unwrap();
    calc.set_xml_document("Pic", data.document).unwrap();
    let schema = xml::parse(XSD, metaschema::xsd_meta()).unwrap();
    calc.add_xsd_sheet("S", schema.node, schema.document)
        .unwrap();
    calc.bind_schema("Pic", "S").unwrap();
    calc.set_auto_update("S", true).unwrap();
    let mut app = GossiaApp::new(calc);
    app.set_open(&list(), true);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1000.0, 600.0))
        .build_ui_state(|ui, app: &mut GossiaApp| app.show(ui), app);
    settle(&mut harness);
    harness
}

fn list() -> Path {
    "[\"#children\"]".parse().unwrap()
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

/// What each item holds: its element's name.
fn items(harness: &Harness<'_, GossiaApp>) -> Vec<String> {
    let data = harness.state().calc().data("Pic").unwrap();
    match data.get(&list()).map(|n| n.value().clone()) {
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| match item.value() {
                Value::Object(map) => map.keys().cloned().collect::<Vec<_>>().join("+"),
                other => format!("{other:?}"),
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn menu(harness: &mut Harness<'_, GossiaApp>, cell: &str, path: &[&str]) {
    harness.get_by_label(cell).click_secondary();
    harness.run();
    for item in path {
        harness.get_by_label(item).click();
        harness.run();
    }
    settle(harness);
}

#[test]
fn an_item_is_the_row_of_its_element() {
    let harness = window();
    harness.get_by_label("[0] rect");
    harness.get_by_label("[1] title");
    // An ordered list is never a table.
    assert!(!harness.state().can_tabulate(&list()));
}

#[test]
fn adding_an_item_asks_which_element() {
    let mut harness = window();
    menu(&mut harness, "#children", &["Add item", "title"]);
    assert_eq!(items(&harness), ["rect", "title", "title"]);
    menu(&mut harness, "[1] title", &["Insert item above", "rect"]);
    assert_eq!(items(&harness), ["rect", "rect", "title", "title"]);
    menu(&mut harness, "[0] rect", &["Delete item"]);
    assert_eq!(items(&harness), ["rect", "title", "title"]);
}

#[test]
fn the_elements_of_a_list_are_fields_of_its_items() {
    let harness = window();
    let field = harness
        .state()
        .field_of(&"[\"#children\"][1].title".parse().unwrap());
    assert_eq!(
        field,
        Some(("[\"#children\"][1]".parse().unwrap(), "title".to_owned()))
    );
}

/// On the generated form an ordered list is shown one item at a time, each
/// as its element, and New asks which element.
#[test]
fn a_form_shows_an_ordered_list_one_item_at_a_time() {
    let mut harness = window();
    assert!(
        harness
            .state_mut()
            .set_view(View::Form("Default".to_owned()))
    );
    settle(&mut harness);
    harness.get_by_label("1 of 2");
    harness.get_by_label("rect");
    harness.get_by_label("Next item").click();
    settle(&mut harness);
    harness.get_by_label("2 of 2");
    harness.get_by_label("title");
    harness.get_by_label("New").click();
    harness.run();
    harness.get_by_label("rect").click();
    settle(&mut harness);
    assert_eq!(items(&harness), ["rect", "title", "rect"]);
    harness.get_by_label("3 of 3");
}
