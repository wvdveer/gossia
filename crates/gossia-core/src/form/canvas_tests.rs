//! Tests of form canvases (DESIGN.md, Later versions, "Form canvases,
//! 0.7.0"): paths with contexts, the stack of sections, the checks, the
//! generated form and converting older designs.

use super::canvas::*;
use super::*;
use crate::{Segment, Value, jsonschema};

fn spec() -> TypeSpec {
    jsonschema::lower_str(
        r##"{
          "type": "object",
          "properties": {
            "name": {"type": "string"},
            "customer": {"type": "object", "properties": {
                "city": {"type": "string"}, "vip": {"type": "boolean"}}},
            "items": {"type": "array", "items": {"type": "object",
                "properties": {"sku": {"type": "string"}, "qty": {"type": "integer"}}}},
            "tags": {"type": "object", "additionalProperties": {"type": "string"}}
          }
        }"##,
    )
    .unwrap()
}

/// Orders with lines inside each: a master and detail.
fn nested() -> TypeSpec {
    jsonschema::lower_str(
        r##"{
          "type": "object",
          "properties": {
            "orders": {"type": "array", "items": {"type": "object", "properties": {
                "number": {"type": "integer"},
                "lines": {"type": "array", "items": {"type": "object", "properties": {
                    "sku": {"type": "string"}, "qty": {"type": "integer"}}}}}}},
            "notes": {"type": "array", "items": {"type": "string"}}
          }
        }"##,
    )
    .unwrap()
}

fn field(path: &str) -> Control {
    Control {
        path: Some(path.to_owned()),
        ..Control::new(ControlKind::Field)
    }
}

fn context(name: &str) -> Control {
    Control {
        context: Some(name.to_owned()),
        ..Control::new(ControlKind::Field)
    }
}

/// A canvas form with the header holding `header`, one repeater over
/// `items` holding `row`, and a footer.
fn form(header: Vec<Control>, row: Vec<Control>) -> FormDesign {
    let mut design = FormDesign::new_canvas(None);
    design.controls[0].children = header;
    let at = design.add_repeater("items").unwrap();
    design.controls[at].children = row;
    design
}

fn problems(design: &FormDesign, spec: &TypeSpec) -> Vec<String> {
    design.broken(spec).into_iter().map(|(_, p)| p).collect()
}

// ----------------------------------------------------- paths with contexts

#[test]
fn a_context_path_reads_and_writes_back() {
    for text in [
        "items[sel].name",
        "orders[row].lines",
        "prices[currency]",
        "a[2][\"odd key\"][k].b",
        "a[i][j]",
        "plain.path[3]",
        "",
    ] {
        let parsed: ContextPath = text.parse().unwrap();
        assert_eq!(parsed.to_string(), text);
    }
    let parsed: ContextPath = "a[i].b[j][i]".parse().unwrap();
    assert_eq!(parsed.names(), vec!["i", "j"]);
    assert!(parsed.has_context());
    assert_eq!(parsed.fixed(), None);
    let plain: ContextPath = "a.b[2]".parse().unwrap();
    assert_eq!(plain.fixed(), Some("a.b[2]".parse().unwrap()));
}

#[test]
fn brackets_inside_a_quoted_key_are_not_a_context() {
    let parsed: ContextPath = "a[\"x[y]\"].b".parse().unwrap();
    assert!(!parsed.has_context());
    assert_eq!(
        parsed.fixed().unwrap().segments(),
        &[
            Segment::Key("a".into()),
            Segment::Key("x[y]".into()),
            Segment::Key("b".into())
        ]
    );
}

#[test]
fn a_bad_context_path_says_why() {
    assert!("[sel].name".parse::<ContextPath>().is_err());
    assert!("items[sel]name".parse::<ContextPath>().is_err());
    assert!("items[sel".parse::<ContextPath>().is_err());
    assert!("items[1x]".parse::<ContextPath>().is_err());
}

#[test]
fn binding_takes_an_index_or_a_key_from_the_context() {
    let parsed: ContextPath = "items[sel].name".parse().unwrap();
    let value = |v: Value| move |_: &str| Some(v.clone());
    assert_eq!(
        parsed.bind(value(Value::Int(3))).unwrap(),
        "items[3].name".parse().unwrap()
    );
    assert_eq!(
        parsed.bind(value(Value::Str("k k".into()))).unwrap(),
        "items[\"k k\"].name".parse().unwrap()
    );
    assert!(parsed.bind(value(Value::Int(-1))).is_err());
    assert!(parsed.bind(value(Value::Null)).is_err());
    assert!(parsed.bind(value(Value::Bool(true))).is_err());
    assert!(parsed.bind(|_| None).is_err());
}

#[test]
fn a_sample_path_steps_into_arrays_and_maps() {
    let spec = spec();
    let root = Path::root();
    assert_eq!(
        sample_path(&spec, &root, "items[sel].qty").unwrap(),
        Some("items[0].qty".parse().unwrap())
    );
    assert_eq!(
        sample_path(&spec, &root, "tags[k]").unwrap(),
        Some(Path::from(vec![
            Segment::Key("tags".into()),
            Segment::Key(String::new())
        ]))
    );
    // A context on an object's fields: which one is not known.
    assert_eq!(sample_path(&spec, &root, "customer[f]").unwrap(), None);
    assert!(sample_path(&spec, &root, "items[").is_err());
}

#[test]
fn a_context_value_is_read_by_its_type() {
    assert_eq!(ContextType::Integer.read("3"), Ok(Value::Int(3)));
    assert_eq!(ContextType::Integer.read(" "), Ok(Value::Null));
    assert!(ContextType::Integer.read("3.5").is_err());
    assert_eq!(ContextType::Number.read("3.5"), Ok(Value::Float(3.5)));
    assert_eq!(ContextType::Text.read(" a "), Ok(Value::Str(" a ".into())));
    assert_eq!(
        ContextType::Date.read("2026-11-15"),
        Ok(Value::Date(
            chrono::NaiveDate::from_ymd_opt(2026, 11, 15).unwrap()
        ))
    );
    assert!(ContextType::Date.read("15/11/2026").is_err());
    assert_eq!(ContextType::Boolean.read("Yes"), Ok(Value::Bool(true)));
    assert!(ContextType::Boolean.read("maybe").is_err());
}

// ------------------------------------------------------------ the sections

#[test]
fn repeaters_go_before_the_footer_with_a_section_between() {
    let mut design = FormDesign::new_canvas(None);
    assert_eq!(design.sections(), vec![Section::Header, Section::Footer]);
    assert_eq!(design.add_repeater("items"), Some(1));
    assert_eq!(design.add_repeater("orders"), Some(3));
    assert_eq!(design.add_repeater("notes"), Some(5));
    assert_eq!(
        design.sections(),
        vec![
            Section::Header,
            Section::Repeater(0),
            Section::Between(0),
            Section::Repeater(1),
            Section::Between(1),
            Section::Repeater(2),
            Section::Footer
        ]
    );
    assert_eq!(design.row_names(), vec!["row", "row2", "row3"]);
    assert_eq!(design.form_width(), DEFAULT_FORM_WIDTH);

    // The middle one goes with the section under it, the last with the one
    // above it.
    assert!(design.remove_repeater(3));
    assert_eq!(design.controls[3].path.as_deref(), Some("notes"));
    assert!(design.remove_repeater(3));
    assert_eq!(
        design.sections(),
        vec![Section::Header, Section::Repeater(0), Section::Footer]
    );
    assert!(!design.remove_repeater(0));
    assert!(design.remove_repeater(1));
    assert_eq!(design.sections(), vec![Section::Header, Section::Footer]);
}

#[test]
fn a_new_repeater_takes_a_row_name_no_context_has() {
    let mut design = FormDesign::new_canvas(None);
    design.controls[0].children.push(context("row"));
    let at = design.add_repeater("items").unwrap();
    assert_eq!(design.row_name_of(at).as_deref(), Some("row2"));
}

#[test]
fn a_form_that_is_not_a_canvas_has_no_sections() {
    let mut design = generate(&spec());
    assert!(design.sections().is_empty());
    assert_eq!(design.add_repeater("items"), None);
}

// -------------------------------------------------------------- the checks

#[test]
fn a_good_canvas_form_has_no_problems() {
    let spec = spec();
    let mut button = Control::new(ControlKind::RowButton);
    button.allows = Some(RowAllows::Add);
    let design = form(
        vec![
            context("sel"),
            field("items[sel].sku"),
            field("items[row].qty"),
        ],
        vec![field("sku"), field("qty"), button],
    );
    assert_eq!(problems(&design, &spec), Vec::<String>::new());
}

#[test]
fn the_stack_must_have_its_shape() {
    let spec = spec();
    let mut design = form(vec![], vec![]);
    design.controls[1].layout = None;
    assert_eq!(problems(&design, &spec).len(), 1);
    let mut design = form(vec![], vec![]);
    design.controls.swap(0, 1);
    assert_eq!(problems(&design, &spec).len(), 2);
}

#[test]
fn a_repeater_section_needs_a_path_to_an_array() {
    let spec = spec();
    let mut design = form(vec![], vec![]);
    design.controls[1].path = None;
    design.controls[1].items = Some("[1, 2]".into());
    assert_eq!(
        problems(&design, &spec),
        vec!["a repeater section needs a path to an array, not items"]
    );
    design.controls[1].items = None;
    design.controls[1].path = Some("name".into());
    assert_eq!(problems(&design, &spec), vec!["name is not an array"]);
}

#[test]
fn a_repeater_inside_a_section_is_refused() {
    let spec = spec();
    let mut inner = Control::new(ControlKind::Repeater);
    inner.path = Some("items".into());
    let design = form(vec![inner], vec![]);
    assert_eq!(
        problems(&design, &spec),
        vec!["on a canvas form a repeater is a section of its own"]
    );
}

#[test]
fn context_fields_belong_on_the_header_with_names_of_their_own() {
    let spec = spec();
    let design = form(vec![], vec![context("sel")]);
    assert_eq!(
        problems(&design, &spec),
        vec!["a context field goes on the form header"]
    );
    let design = form(vec![context("sel"), context("sel")], vec![]);
    assert_eq!(problems(&design, &spec).len(), 2);
    let mut design = form(vec![context("row")], vec![]);
    design.controls[1].row_name = Some("row".into());
    // The row name clashes too: both are told.
    assert_eq!(problems(&design, &spec).len(), 2);
    let mut both = context("sel");
    both.path = Some("name".into());
    let design = form(vec![both], vec![]);
    assert_eq!(
        problems(&design, &spec),
        vec!["a context field has no path and no value"]
    );
    let mut bad = context("sel");
    bad.context_default = Some("x".into());
    let design = form(vec![bad], vec![]);
    assert_eq!(problems(&design, &spec).len(), 1);
    let mut not_canvas = generate(&spec);
    not_canvas.controls.push(context("sel"));
    assert_eq!(
        problems(&not_canvas, &spec),
        vec!["a context field goes on the form header"]
    );
}

#[test]
fn a_path_uses_only_the_forms_contexts() {
    let spec = spec();
    let design = form(vec![field("items[nope].sku")], vec![]);
    assert_eq!(
        problems(&design, &spec),
        vec!["the form has no context field or row name called nope"]
    );
    let mut design = form(vec![], vec![]);
    design.controls[1].path = Some("items[row].x".into());
    assert!(
        problems(&design, &spec)
            .contains(&"a repeater's path cannot use its own row name".to_owned())
    );
    // A form that is not a canvas has no contexts.
    let mut flow = generate(&spec);
    flow.controls.push(field("items[sel].sku"));
    assert_eq!(
        problems(&flow, &spec),
        vec!["the form has no context field or row name called sel"]
    );
}

#[test]
fn a_path_with_a_context_is_checked_against_the_schema() {
    let spec = spec();
    let design = form(vec![context("sel"), field("items[sel].nope")], vec![]);
    assert_eq!(
        problems(&design, &spec),
        vec!["the schema has nothing at items[sel].nope"]
    );
}

#[test]
fn one_add_delete_button_on_a_repeater_section() {
    let spec = spec();
    let button = || Control::new(ControlKind::RowButton);
    let design = form(vec![button()], vec![]);
    assert_eq!(
        problems(&design, &spec),
        vec!["an add/delete button goes on a repeater section"]
    );
    let design = form(vec![], vec![button(), button()]);
    assert_eq!(
        problems(&design, &spec),
        vec!["a repeater has one add/delete button"]
    );
}

#[test]
fn a_row_button_has_two_configurations() {
    let mut button = Control::new(ControlKind::RowButton);
    assert_eq!(button.row_button_label(true), "Add");
    assert_eq!(button.row_button_label(false), "Delete");
    assert!(button.row_allows().adds() && button.row_allows().deletes());
    button.add_label = Some("New line".into());
    button.allows = Some(RowAllows::Delete);
    assert_eq!(button.row_button_label(true), "New line");
    assert!(!button.row_allows().adds());
}

// ------------------------------------------------- storing and the version

#[test]
fn a_canvas_design_reads_back_and_an_older_build_refuses_it() {
    let spec = spec();
    let mut design = generate_canvas(&spec);
    design.width = Some(720);
    design.controls[0].children.push(Control {
        hidden: true,
        context_type: Some(ContextType::Text),
        context_default: Some("EUR".into()),
        ..context("currency")
    });
    let json = design.to_json();
    assert!(json.contains("\"version\": 2"));
    assert!(json.contains("\"type\": \"row_button\""));
    assert!(json.contains("\"default\": \"EUR\""));
    assert_eq!(FormDesign::from_json(&json).unwrap(), design);
    let newer = json.replacen("\"version\": 2", "\"version\": 3", 1);
    assert!(FormDesign::from_json(&newer).is_err());
    let narrow = json.replacen("\"width\": 720", "\"width\": 100", 1);
    assert!(FormDesign::from_json(&narrow).is_err());
}

// ------------------------------------------------------- the generated form

#[test]
fn the_generated_form_is_a_header_a_repeater_per_array_and_a_footer() {
    let spec = spec();
    let design = generate_canvas(&spec);
    assert!(design.canvas);
    assert_eq!(design.version, CANVAS_VERSION);
    assert_eq!(
        design.sections(),
        vec![Section::Header, Section::Repeater(0), Section::Footer]
    );
    let header = &design.controls[0];
    let paths: Vec<Option<&str>> = header
        .children
        .iter()
        .filter(|c| c.kind != ControlKind::Label)
        .map(|c| c.path.as_deref())
        .collect();
    assert_eq!(paths, vec![Some("name"), Some("customer"), Some("tags")]);
    // Placed one under another, without overlapping.
    let tops: Vec<u32> = header.children.iter().map(|c| c.y.unwrap()).collect();
    assert!(tops.windows(2).all(|w| w[0] <= w[1]));
    // The column titles are at the bottom of the header.
    let titles: Vec<&str> = header
        .children
        .iter()
        .filter(|c| c.kind == ControlKind::Label)
        .filter_map(|c| c.text.as_deref())
        .collect();
    assert_eq!(titles, vec!["sku", "qty"]);
    let row = &design.controls[1];
    assert!(row.is_section());
    assert_eq!(row.path.as_deref(), Some("items"));
    let kinds: Vec<(ControlKind, Option<&str>)> = row
        .children
        .iter()
        .map(|c| (c.kind, c.path.as_deref()))
        .collect();
    assert_eq!(
        kinds,
        vec![
            (ControlKind::Field, Some("sku")),
            (ControlKind::Field, Some("qty")),
            (ControlKind::RowButton, None)
        ]
    );
    // Everything is within the form's width.
    for c in &row.children {
        assert!(c.x.unwrap() + c.width.unwrap() <= design.form_width());
    }
    assert_eq!(problems(&design, &spec), Vec::<String>::new());
    assert!(design.unplaced(&spec).is_empty());
}

#[test]
fn an_array_inside_items_reads_through_the_outer_row() {
    let spec = nested();
    let design = generate_canvas(&spec);
    assert_eq!(
        design.sections(),
        vec![
            Section::Header,
            Section::Repeater(0),
            Section::Between(0),
            Section::Repeater(1),
            Section::Between(1),
            Section::Repeater(2),
            Section::Footer
        ]
    );
    let paths: Vec<&str> = design
        .repeater_sections()
        .into_iter()
        .map(|i| design.controls[i].path.as_deref().unwrap())
        .collect();
    assert_eq!(paths, vec!["orders", "orders[row].lines", "notes"]);
    // The orders' row has only the number: the lines are a section.
    let orders = &design.controls[1];
    assert_eq!(orders.children[0].path.as_deref(), Some("number"));
    assert_eq!(orders.children.len(), 2);
    // An array of strings: one field, the item itself, titled by the array.
    let notes = &design.controls[5];
    assert_eq!(notes.children[0].path.as_deref(), Some(""));
    assert_eq!(problems(&design, &spec), Vec::<String>::new());
}

// ------------------------------------------------- converting older designs

#[test]
fn designed_items_become_the_row() {
    let spec = spec();
    let mut flow = generate(&spec);
    let items = flow
        .controls
        .iter_mut()
        .find(|c| c.kind == ControlKind::Repeater)
        .unwrap();
    items.mode = Some(RepeaterMode::Stacked);
    items.item_name = Some("item".into());
    items.children = vec![field("qty"), field("sku")];
    let converted = to_canvas(&flow, &spec);
    assert!(converted.dropped.is_empty());
    let row = &converted.design.controls[1];
    assert_eq!(row.item_name.as_deref(), Some("item"));
    let paths: Vec<Option<&str>> = row.children.iter().map(|c| c.path.as_deref()).collect();
    assert_eq!(paths, vec![Some("qty"), Some("sku"), None]);
    assert!(row.children[1].y > row.children[0].y);
    assert_eq!(problems(&converted.design, &spec), Vec::<String>::new());
}

#[test]
fn a_tables_hidden_columns_and_titles_are_kept() {
    let spec = spec();
    let mut flow = generate(&spec);
    let items = flow
        .controls
        .iter_mut()
        .find(|c| c.kind == ControlKind::Repeater)
        .unwrap();
    items.table_columns = vec![
        ColumnDesign::named(ROW_NUMBER),
        ColumnDesign {
            label: Some("Quantity".into()),
            ..ColumnDesign::named("qty")
        },
        ColumnDesign {
            hidden: true,
            ..ColumnDesign::named("sku")
        },
    ];
    let design = to_canvas(&flow, &spec).design;
    let row = &design.controls[1];
    assert_eq!(row.children[0].path.as_deref(), Some("qty"));
    assert_eq!(row.children.len(), 2);
    assert!(
        design.controls[0]
            .children
            .iter()
            .any(|c| c.text.as_deref() == Some("Quantity"))
    );
}

#[test]
fn a_repeater_over_an_expression_is_dropped_and_named() {
    let spec = spec();
    let mut flow = generate(&spec);
    let mut related = Control::new(ControlKind::Repeater);
    related.items = Some("Orders.items".into());
    related.label = Some("Related orders".into());
    let mut group = Control::new(ControlKind::Group);
    group.children.push(related);
    flow.controls.push(group);
    let converted = to_canvas(&flow, &spec);
    assert_eq!(converted.dropped, vec!["Related orders"]);
    assert!(
        converted
            .design
            .walk()
            .iter()
            .all(|(_, c)| c.items.is_none())
    );
}

#[test]
fn a_canvas_is_not_converted_again() {
    let spec = spec();
    let design = generate_canvas(&spec);
    assert_eq!(to_canvas(&design, &spec).design, design);
}
