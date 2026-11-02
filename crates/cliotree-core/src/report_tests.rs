use crate::form::{Control, ControlKind, RepeaterMode};
use crate::jsonschema;
use crate::report::{
    Orientation, Paper, ReportDesign, ReportKind, starting_arrays, starting_design,
};
use crate::{Path, TypeSpec};

fn orders_spec() -> TypeSpec {
    jsonschema::lower_str(
        r#"{
          "type": "object",
          "properties": {
            "customer": {"type": "string"},
            "items": {
              "type": "array",
              "items": {
                "type": "object",
                "properties": {
                  "sku": {"type": "string"},
                  "qty": {"type": "integer"},
                  "price": {"type": "object", "properties": {"amount": {"type": "number"}}}
                }
              }
            },
            "orders": {
              "type": "array",
              "items": {
                "type": "object",
                "properties": {
                  "no": {"type": "integer"},
                  "first-name": {"type": "string"},
                  "lines": {"type": "array", "items": {"type": "object", "properties": {"sku": {"type": "string"}}}}
                }
              }
            },
            "tags": {"type": "array", "items": {"type": "string"}}
          }
        }"#,
    )
    .unwrap()
}

#[test]
fn a_report_reads_back_as_it_was_written() {
    let mut design = ReportDesign::new(ReportKind::Multipage);
    design.page.paper = Paper::Letter;
    design.page.orientation = Orientation::Landscape;
    design.page.margins.left = 72;
    let mut field = Control::new(ControlKind::Field);
    field.value = Some("SUM(o.total for o in Orders.items)".into());
    design.layout.controls.push(field);
    let text = design.to_json();
    assert!(text.contains("\"kind\": \"multipage\""), "{text}");
    assert!(text.contains("\"paper\": \"letter\""), "{text}");
    assert_eq!(ReportDesign::from_json(&text).unwrap(), design);
}

#[test]
fn builtin_report_takes_its_controls_from_builtin_form() {
    let ok = r#"{"version": 1, "kind": "single_page", "layout": {"version": 1, "controls": []}}"#;
    let design = ReportDesign::from_json(ok).unwrap();
    assert_eq!(design.kind, ReportKind::SinglePage);
    assert_eq!(design.page.paper, Paper::A4);
    let bad_kind = r#"{"version": 1, "kind": "poster", "layout": {"version": 1, "controls": []}}"#;
    assert!(
        ReportDesign::from_json(bad_kind)
            .unwrap_err()
            .contains("kind")
    );
    // A control is checked as a form's is.
    let bad_font = r#"{"version": 1, "kind": "single_page", "layout": {"version": 1,
        "controls": [{"type": "label", "font": "gothic"}]}}"#;
    assert!(ReportDesign::from_json(bad_font).is_err());
    let no_layout = r#"{"version": 1, "kind": "single_page"}"#;
    assert!(ReportDesign::from_json(no_layout).is_err());
    let newer =
        r#"{"version": 9, "kind": "single_page", "layout": {"version": 1, "controls": []}}"#;
    assert!(
        ReportDesign::from_json(newer)
            .unwrap_err()
            .contains("newer")
    );
}

#[test]
fn the_page_turns_with_its_orientation() {
    let mut design = ReportDesign::new(ReportKind::SinglePage);
    assert_eq!(design.page.size(), (595.28, 841.89));
    design.page.orientation = Orientation::Landscape;
    assert_eq!(design.page.size(), (841.89, 595.28));
    assert!((design.page.content_width() - (841.89 - 72.0)).abs() < 0.01);
}

#[test]
fn flat_items_start_as_a_table_of_expressions() {
    let spec = orders_spec();
    let design = starting_design(
        ReportKind::SinglePage,
        &spec,
        "Orders",
        &"items".parse().unwrap(),
    )
    .unwrap();
    let repeater = &design.layout.controls[0];
    assert_eq!(repeater.items.as_deref(), Some("Orders.items"));
    assert_eq!(repeater.item_name.as_deref(), Some("order"));
    assert_eq!(repeater.mode, Some(RepeaterMode::Table));
    assert_eq!(repeater.path, None);
    let columns: Vec<(&str, &str)> = repeater
        .table_columns
        .iter()
        .map(|c| (c.name.as_str(), c.value.as_deref().unwrap()))
        .collect();
    assert_eq!(
        columns,
        [
            ("sku", "order.sku"),
            ("qty", "order.qty"),
            ("price.amount", "order.price.amount")
        ]
    );
    assert!(design.problems().is_empty(), "{:?}", design.problems());
}

#[test]
fn scalar_items_are_a_table_of_one_column() {
    let spec = orders_spec();
    let design = starting_design(
        ReportKind::SinglePage,
        &spec,
        "Orders",
        &"tags".parse().unwrap(),
    )
    .unwrap();
    let repeater = &design.layout.controls[0];
    assert_eq!(repeater.item_name.as_deref(), Some("tag"));
    assert_eq!(repeater.table_columns.len(), 1);
    assert_eq!(repeater.table_columns[0].value.as_deref(), Some("tag"));
}

#[test]
fn items_that_hold_arrays_start_as_designed_items() {
    let spec = orders_spec();
    let design = starting_design(
        ReportKind::Multipage,
        &spec,
        "Shop",
        &"orders".parse().unwrap(),
    )
    .unwrap();
    let repeater = &design.layout.controls[0];
    assert_eq!(repeater.items.as_deref(), Some("Shop.orders"));
    assert_eq!(repeater.item_name.as_deref(), Some("order"));
    assert_eq!(repeater.mode, Some(RepeaterMode::Stacked));
    let values: Vec<Option<&str>> = repeater
        .children
        .iter()
        .map(|c| c.value.as_deref().or(c.items.as_deref()))
        .collect();
    assert_eq!(
        values,
        [
            Some("order.no"),
            Some(r#"order["first-name"]"#),
            Some("order.lines")
        ]
    );
    let lines = &repeater.children[2];
    assert_eq!(lines.item_name.as_deref(), Some("line"));
    assert_eq!(lines.table_columns[0].value.as_deref(), Some("line.sku"));
    assert!(design.problems().is_empty(), "{:?}", design.problems());
}

#[test]
fn only_arrays_can_be_started_from() {
    let spec = orders_spec();
    let arrays: Vec<String> = starting_arrays(&spec)
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(arrays, ["items", "orders", "tags"]);
    assert!(
        starting_design(
            ReportKind::SinglePage,
            &spec,
            "Orders",
            &"customer".parse().unwrap()
        )
        .is_none()
    );
    assert!(starting_design(ReportKind::SinglePage, &spec, "Orders", &Path::root()).is_none());
}

#[test]
fn a_report_lists_what_only_a_form_can_have() {
    let mut design = ReportDesign::new(ReportKind::SinglePage);
    design.layout.controls.push(Control::bound(
        ControlKind::Field,
        &"customer".parse().unwrap(),
    ));
    design
        .layout
        .controls
        .push(Control::new(ControlKind::Button));
    let mut field = Control::new(ControlKind::Field);
    field.value = Some("1".into());
    field.enabled_when = Some("True".into());
    design.layout.controls.push(field);
    let mut label = Control::new(ControlKind::Label);
    label.text = Some("Fine".into());
    design.layout.controls.push(label);
    let problems = design.problems();
    let at: Vec<&[usize]> = problems.iter().map(|(a, _)| a.as_slice()).collect();
    assert_eq!(at, [&[0][..], &[1], &[2]]);
    assert!(problems[0].1.contains("path"), "{}", problems[0].1);
    assert!(problems[1].1.contains("buttons"));
    assert!(problems[2].1.contains("enabled when"));
}
