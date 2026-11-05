//! Tests of adding, deleting and rebinding tabs, and of importing files
//! (DESIGN.md, "Managing tabs" and "Import and export").

use std::path::PathBuf;
use std::time::Duration;

use gossia_core::validate::validate;
use gossia_core::{Path, Value, json};
use serde_json::json;

use crate::GossiaApp;
use crate::control;
use crate::import::{ImportFormat, SchemaChoice, format_of, format_of_text, name_from_file};
use crate::tab_commands::NewSchema;
use crate::workbook_file::new_workbook_id;

fn p(text: &str) -> Path {
    text.parse().unwrap()
}

fn settle(app: &mut GossiaApp) {
    assert!(app.session.wait_idle(Duration::from_secs(60)));
}

fn sample() -> GossiaApp {
    let mut app = GossiaApp::sample();
    app.files.browse = false;
    settle(&mut app);
    app
}

fn tabs(app: &GossiaApp) -> Vec<&str> {
    app.calc().sheet_names().collect()
}

fn valid(app: &GossiaApp, tab: &str) -> bool {
    let calc = app.calc();
    validate(calc.spec(tab).unwrap(), calc.data(tab).unwrap()).is_empty()
}

/// A folder of its own for the files of one test.
fn folder(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "gossia-{test}-{}-{}",
        std::process::id(),
        new_workbook_id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(dir: &std::path::Path, name: &str, text: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    path
}

// ------------------------------------------------------------- new tabs

#[test]
fn a_new_tab_with_its_own_schema_is_one_entry() {
    let mut app = sample();
    app.begin_new_tab();
    let dialog = app.tab_dialogs.new.clone().unwrap();
    assert_eq!(
        (dialog.name.as_str(), dialog.schema_name.as_str()),
        ("Data", "DataSchema")
    );
    app.create_tab(&dialog).unwrap();
    assert_eq!(
        tabs(&app),
        [
            "Orders",
            "Prices",
            "Orders_schema",
            "Prices_schema",
            "DataSchema",
            "Data"
        ]
    );
    assert_eq!(app.sheet, "Data", "the new tab is shown");
    assert!(
        app.calc().auto_update("DataSchema"),
        "fields are added from Data"
    );
    assert_eq!(app.calc().undo_label(), Some("New tab Data"));

    app.undo();
    assert_eq!(tabs(&app).len(), 4);
    assert_eq!(app.sheet, "Prices_schema", "the last tab is shown instead");
    app.redo();
    assert_eq!(tabs(&app).len(), 6);
}

#[test]
fn a_new_tab_can_use_a_schema_that_is_there() {
    let mut app = sample();
    app.begin_new_tab();
    let mut dialog = app.tab_dialogs.new.clone().unwrap();
    dialog.name = "Order2".to_owned();
    dialog.schema = NewSchema::Existing("Orders_schema".to_owned());
    app.create_tab(&dialog).unwrap();
    assert_eq!(app.calc().schema_of("Order2"), Some("Orders_schema"));
    assert!(valid(&app, "Order2"), "the required fields are filled in");
    assert_eq!(
        app.calc().schema_users("Orders_schema"),
        ["Orders", "Order2"]
    );

    dialog.name = "orders".to_owned();
    assert!(
        app.new_tab_problem(&dialog)
            .unwrap()
            .contains("exists already")
    );
}

#[test]
fn a_new_tab_can_hold_yaml_or_another_format_of_json_shaped_data() {
    use gossia_core::dataformat::DataFormat;
    let mut app = sample();
    app.begin_new_tab();
    let mut dialog = app.tab_dialogs.new.clone().unwrap();
    assert_eq!(dialog.format, DataFormat::Json);
    dialog.name = "Config".to_owned();
    dialog.schema_name = "ConfigSchema".to_owned();
    dialog.format = DataFormat::Yaml;
    app.create_tab(&dialog).unwrap();
    assert_eq!(app.calc().format("Config"), Some(DataFormat::Yaml));
    assert_eq!(app.raw_text(), "{}");
    // One entry makes both tabs and sets the format.
    assert_eq!(app.calc().undo_label(), Some("New tab Config"));
    app.undo();
    assert!(!tabs(&app).contains(&"Config"));
    assert!(!tabs(&app).contains(&"ConfigSchema"));

    // A format that holds a list gets a schema of a list.
    dialog.format = DataFormat::JsonLines;
    app.create_tab(&dialog).unwrap();
    assert_eq!(app.calc().format("Config"), Some(DataFormat::JsonLines));
    assert_eq!(
        app.calc().data("Config").unwrap().value(),
        &Value::Array(Vec::new())
    );

    // With a schema that is there, the format must fit its root.
    dialog.name = "Order2".to_owned();
    dialog.schema = NewSchema::Existing("Orders_schema".to_owned());
    let why = app.new_tab_problem(&dialog).unwrap();
    assert!(why.contains("JSON Lines"), "{why}");
    dialog.format = DataFormat::Toml;
    app.create_tab(&dialog).unwrap();
    assert_eq!(app.calc().format("Order2"), Some(DataFormat::Toml));
    assert!(valid(&app, "Order2"));
}

// ------------------------------------------------------------ deleting

#[test]
fn deleting_the_current_tab_shows_the_next_and_undo_puts_it_back() {
    let mut app = sample();
    app.set_tabular(&p("items"), true);
    let refused = app.delete_tab("Orders_schema").unwrap_err();
    assert!(refused.contains("Orders uses this schema"), "{refused}");

    app.delete_tab("Orders").unwrap();
    assert_eq!(tabs(&app), ["Prices", "Orders_schema", "Prices_schema"]);
    assert_eq!(app.sheet, "Prices");
    assert!(
        app.tables.tabular.is_empty(),
        "what the window kept is gone"
    );
    assert_eq!(app.calc().undo_label(), Some("Delete tab Orders"));
    assert_eq!(app.calc().delete_problem("Orders_schema"), None);

    app.undo();
    settle(&mut app);
    assert_eq!(tabs(&app)[0], "Orders");
    assert!(matches!(
        app.calc()
            .data("Orders")
            .unwrap()
            .get(&p("total"))
            .unwrap()
            .value(),
        Value::Float(_)
    ));
}

#[test]
fn the_delete_dialog_warns_about_formulas_that_use_the_tab() {
    let app = sample();
    let mentions = app.calc().mentions("Prices");
    let text = crate::tab_commands::delete_mentions_text("Prices", mentions);
    match mentions.formulas {
        0 => assert_eq!(text, None),
        _ => assert!(text.unwrap().contains("#NAME! until the tab is back")),
    }
    assert_eq!(
        crate::tab_commands::delete_mentions_text(
            "X",
            gossia_calc::Mentions {
                formulas: 2,
                modules: 1
            }
        )
        .as_deref(),
        Some("2 formulas and 1 module use X; they will show #NAME! until the tab is back.")
    );
}

// ----------------------------------------------------------- rebinding

#[test]
fn a_tab_is_given_another_schema() {
    let mut app = sample();
    app.switch_to("Prices");
    app.bind_tab("Prices", "Orders_schema").unwrap();
    assert_eq!(app.calc().schema_of("Prices"), Some("Orders_schema"));
    assert!(!valid(&app, "Prices"));
    assert_eq!(
        app.calc().undo_label(),
        Some("Bind Prices to Orders_schema")
    );
    app.undo();
    assert_eq!(app.calc().schema_of("Prices"), Some("Prices_schema"));
    assert!(valid(&app, "Prices"));
}

// ----------------------------------------------------------- importing

#[test]
fn names_are_made_from_the_file_name() {
    let free = |_: &str| false;
    assert_eq!(name_from_file("my-file.json", free), "my_file");
    assert_eq!(name_from_file("2024 orders.xml", free), "Data2024_orders");
    assert_eq!(name_from_file("_x.json", free), "Data_x");
    assert_eq!(name_from_file("class.json", free), "class2", "a keyword");
    assert_eq!(
        name_from_file("Orders.json", |n| n.eq_ignore_ascii_case("orders")),
        "Orders2"
    );
    assert_eq!(format_of("A.JSON"), ImportFormat::Json);
    assert_eq!(format_of("a.xml"), ImportFormat::Xml);
    assert_eq!(format_of("notes.txt"), ImportFormat::Text);
    assert_eq!(format_of("Sales.CSV"), ImportFormat::Csv);
    assert_eq!(format_of("sales.tsv"), ImportFormat::Tsv);
    assert_eq!(format_of("sales.tab"), ImportFormat::Tsv);
}

#[test]
fn pasted_text_is_read_as_the_format_it_shows() {
    assert_eq!(format_of_text("  {\"a\": 1}"), ImportFormat::Json);
    assert_eq!(format_of_text("[1, 2]"), ImportFormat::Json);
    assert_eq!(format_of_text("\u{FEFF}<a/>"), ImportFormat::Xml);
    assert_eq!(format_of_text("sku\tqty\nA\t2"), ImportFormat::Tsv);
    assert_eq!(format_of_text("sku,qty\nA,2"), ImportFormat::Csv);
    assert_eq!(format_of_text("one\ntwo"), ImportFormat::Text);
}

/// Import and export, "CSV and TSV": a CSV file is an array of objects with a
/// schema inferred for it, the header row naming the fields.
#[test]
fn a_csv_file_is_imported_as_an_array_of_rows() {
    let mut app = sample();
    let text = "sku,qty,price\nA-1,2,9.5\nB-2,1,24\n";
    let plan = app.plan_import("stock.csv", text.as_bytes()).unwrap();
    assert_eq!(plan.choice, SchemaChoice::Inferred);
    assert_eq!(plan.name, "stock");
    assert!(plan.replace.is_none());
    assert_eq!(
        plan.report(),
        ["The first row is taken as the names of the columns."]
    );
    app.import(&plan).unwrap();
    let data = json::to_string(app.calc().data("stock").unwrap(), false).unwrap();
    assert_eq!(
        data,
        r#"[{"sku":"A-1","qty":2,"price":9.5},{"sku":"B-2","qty":1,"price":24.0}]"#
    );
    assert!(valid(&app, "stock"));
    assert_eq!(app.calc().undo_label(), Some("Import stock.csv"));

    // A TSV file without a header row: its first row has a number in it.
    let plan = app
        .plan_import("more.tsv", b"A-3\t5\t1.25\nA-4\t6\t2\n")
        .unwrap();
    app.import(&plan).unwrap();
    let data = json::to_string(app.calc().data("more").unwrap(), false).unwrap();
    assert_eq!(
        data,
        r#"[{"column_1":"A-3","column_2":5,"column_3":1.25},{"column_1":"A-4","column_2":6,"column_3":2.0}]"#
    );
}

/// A paste in the Raw view is imported as a file is: a tab called Pasted. In
/// the tab's own format it may replace the tab's data instead.
#[test]
fn a_paste_is_imported_and_may_replace_the_tab_in_its_own_format() {
    let mut app = sample();
    let plan = app.plan_paste("sku\tqty\nA\t2\n").unwrap();
    assert_eq!(plan.format, ImportFormat::Tsv);
    assert_eq!(plan.name, "Pasted");
    assert!(plan.replace.is_none(), "TSV is not the format of Orders");
    app.import(&plan).unwrap();
    assert_eq!(app.sheet, "Pasted");
    assert_eq!(app.calc().undo_label(), Some("Import the pasted text"));

    app.show_tab("Orders");
    let orders = json::to_string(app.calc().data("Orders").unwrap(), true).unwrap();
    let mut plan = app
        .plan_paste(&orders.replace("Acme Ltd", "Globex"))
        .unwrap();
    assert_eq!(plan.replace.as_deref(), Some("Orders"));
    assert_eq!(plan.name, "Pasted2");
    assert_eq!(
        plan.choice,
        SchemaChoice::Existing("Orders_schema".to_owned()),
        "a new tab is the default"
    );
    let before = tabs(&app).len();
    plan.choice = SchemaChoice::Replace("Orders".to_owned());
    app.import(&plan).unwrap();
    assert_eq!(
        app.calc()
            .data("Orders")
            .unwrap()
            .get(&p("customer.name"))
            .map(|n| n.value().clone()),
        Some(Value::Str("Globex".into()))
    );
    assert_eq!(tabs(&app).len(), before, "no tab is added for a replace");

    let error = app.plan_paste("{nope").unwrap_err();
    assert!(error.starts_with("The pasted text is not JSON"), "{error}");
}

#[test]
fn data_that_fits_a_schema_tab_is_given_it() {
    let dir = folder("import-fits");
    let mut app = sample();
    let orders = json::to_string(app.calc().data("Orders").unwrap(), true).unwrap();
    let file = write(&dir, "orders.json", &orders);
    app.begin_import_path(&file).unwrap();
    let plan = app.imports.pop_front().unwrap();
    assert_eq!(plan.name, "orders2", "Orders is taken, ignoring case");
    assert_eq!(
        plan.choice,
        SchemaChoice::Existing("Orders_schema".to_owned())
    );
    let prices = plan
        .candidates
        .iter()
        .find(|c| c.schema == "Prices_schema")
        .unwrap();
    assert!(prices.problems > 0);
    app.import(&plan).unwrap();
    assert_eq!(tabs(&app).len(), 5, "no schema tab is added");
    assert_eq!(app.sheet, "orders2");
    assert_eq!(app.calc().schema_of("orders2"), Some("Orders_schema"));
    assert!(valid(&app, "orders2"));
    assert_eq!(app.calc().undo_label(), Some("Import orders.json"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn data_that_fits_no_schema_tab_gets_an_inferred_one() {
    let mut app = sample();
    let text = r#"{"city": "Utrecht", "people": [{"name": "Ann", "age": 31}, {"name": "Bo", "note": null}]}"#;
    let plan = app.plan_import("people.json", text.as_bytes()).unwrap();
    assert_eq!(plan.choice, SchemaChoice::Inferred);
    assert_eq!(plan.schema_name, "peopleSchema");
    assert_eq!(
        plan.report(),
        [
            "people[].note: 1 null left out, since version 1 has no null, so it is optional.",
            "people[].note: inferred from no data, so it is text.",
        ]
    );
    app.import(&plan).unwrap();
    assert_eq!(&tabs(&app)[4..], ["peopleSchema", "people"]);
    assert_eq!(app.calc().schema_of("people"), Some("peopleSchema"));
    assert!(!app.calc().auto_update("peopleSchema"));
    assert!(valid(&app, "people"));

    // The next file of that shape fits the schema that was inferred.
    let again = app
        .plan_import("people.json", br#"{"city": "Delft", "people": []}"#)
        .unwrap();
    assert_eq!(
        again.choice,
        SchemaChoice::Existing("peopleSchema".to_owned())
    );
    assert_eq!(again.name, "people2");

    app.undo();
    assert_eq!(tabs(&app).len(), 4, "both tabs go in one undo");
}

#[test]
fn xml_is_imported_with_an_inferred_xsd() {
    let mut app = sample();
    let text = r#"<?xml version="1.0"?>
<catalog><book id="b1"><title>Dune</title><year>1965</year></book><book id="b2"><title>Emma</title><year>1815</year></book></catalog>"#;
    let plan = app.plan_import("catalog.xml", text.as_bytes()).unwrap();
    assert_eq!(plan.choice, SchemaChoice::Inferred);
    app.import(&plan).unwrap();
    assert!(app.calc().is_xml("catalog"));
    assert_eq!(
        app.calc().schema_format("catalogSchema"),
        Some(gossia_calc::SchemaFormat::Xsd)
    );
    let year = app
        .calc()
        .data("catalog")
        .unwrap()
        .get(&p(r#"book[1].year"#))
        .map(|n| n.value().clone());
    assert_eq!(year, Some(Value::Int(1815)));
    assert!(valid(&app, "catalog"));

    let again = app.plan_import("more.xml", text.as_bytes()).unwrap();
    assert_eq!(
        again.choice,
        SchemaChoice::Existing("catalogSchema".to_owned())
    );
    app.import(&again).unwrap();
    assert!(app.calc().is_xml("more"));
    assert!(valid(&app, "more"));
}

#[test]
fn xml_outside_the_subset_can_still_go_into_an_xsd_tab_or_is_refused() {
    let app = sample();
    let refused = app
        .plan_import("p.xml", b"<p>Hello <b>you</b></p>")
        .unwrap_err();
    assert!(refused.contains("mixed content"), "{refused}");
    let broken = app.plan_import("x.xml", b"<a><b></a>").unwrap_err();
    assert!(
        broken.starts_with("x.xml is not well-formed XML"),
        "{broken}"
    );
    let not_json = app.plan_import("x.json", b"{nope").unwrap_err();
    assert!(not_json.starts_with("x.json is not JSON"), "{not_json}");
    let binary = app.plan_import("x.bin", &[0xFF, 0xFE, 0x00]).unwrap_err();
    assert!(binary.contains("not UTF-8"), "{binary}");
}

#[test]
fn plain_text_is_an_array_of_lines() {
    let mut app = sample();
    let plan = app
        .plan_import("notes.txt", b"\xEF\xBB\xBFone\r\ntwo\n")
        .unwrap();
    app.import(&plan).unwrap();
    let data = json::to_string(app.calc().data("notes").unwrap(), false).unwrap();
    assert_eq!(data, r#"["one","two"]"#);
}

#[test]
fn dropped_files_are_imported_one_dialog_after_another() {
    let dir = folder("drop");
    let mut app = sample();
    let a = write(&dir, "a.json", r#"{"x": 1}"#);
    let b = write(&dir, "b.xml", "<b><c>1</c></b>");
    let bad = write(&dir, "c.json", "{");
    app.drop_files(vec![a, b, bad]);
    assert_eq!(app.imports.len(), 2);
    assert!(
        app.notice
            .as_deref()
            .unwrap()
            .contains("c.json is not JSON")
    );
    let first = app.imports.pop_front().unwrap();
    app.import(&first).unwrap();
    let second = app.imports.pop_front().unwrap();
    app.import(&second).unwrap();
    assert_eq!(&tabs(&app)[4..], ["aSchema", "a", "bSchema", "b"]);

    // A workbook that is dropped is opened.
    let saved = dir.join("w.gossia");
    app.save_as(&saved).unwrap();
    let mut other = sample();
    other.drop_files(vec![saved]);
    assert_eq!(tabs(&other).len(), 8);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_control_channel_manages_tabs() {
    let dir = folder("control-tabs");
    let mut app = sample();
    control::run(&mut app, "new_tab", &json!({ "name": "Extra" })).unwrap();
    assert_eq!(app.calc().schema_of("Extra"), Some("ExtraSchema"));
    control::run(
        &mut app,
        "bind_schema",
        &json!({ "tab": "Prices", "schema": "Orders_schema" }),
    )
    .unwrap();
    control::run(&mut app, "delete_tab", &json!({ "name": "Extra" })).unwrap();
    assert!(app.calc().data("Extra").is_none());
    let file = write(&dir, "list.json", "[1, 2, 3]");
    let answer = control::run(
        &mut app,
        "import",
        &json!({ "path": file.display().to_string(), "name": "Numbers" }),
    )
    .unwrap();
    assert_eq!(answer["tab"], "Numbers");
    assert_eq!(answer["schema"], "NumbersSchema");
    assert_eq!(app.calc().schema_of("Numbers"), Some("NumbersSchema"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn json_written_by_hand_with_a_trailing_comma_is_imported() {
    let mut app = sample();
    // The shape of the randomuser.me sample, with its comma before `}`.
    let text = r#"{"results": [{"gender": "female", "location": {"street": {"number": 8929, "name": "Valwood Pkwy",
}, "postcode": "63104"}, "dob": {"date": "1992-03-08T15:13:16.688Z", "age": 30}}],
"info": {"seed": "56d27f4a53bd5441", "results": 1, "page": 1, "version": "1.4"}}"#;
    let plan = app.plan_import("randomuser.json", text.as_bytes()).unwrap();
    assert_eq!(
        plan.notes,
        ["a comma before a closing bracket, on line 1, was ignored."]
    );
    assert_eq!(plan.choice, SchemaChoice::Inferred);
    app.import(&plan).unwrap();
    assert!(valid(&app, "randomuser"));
}

// ------------------------------------------------- schema tabs of their own

/// A new tab with its own XSD: an XML tab whose root element is named after
/// it, with auto-update on, so that fields added from the data tab go into
/// the XSD.
#[test]
fn a_new_xml_tab_starts_from_a_blank_xsd() {
    let mut app = sample();
    app.begin_new_tab();
    let mut dialog = app.tab_dialogs.new.clone().unwrap();
    dialog.schema = NewSchema::Own(gossia_calc::SchemaFormat::Xsd);
    app.create_tab(&dialog).unwrap();
    let calc = app.calc();
    assert!(calc.is_xml("Data"));
    assert_eq!(
        calc.schema_format("DataSchema"),
        Some(gossia_calc::SchemaFormat::Xsd)
    );
    assert!(calc.auto_update("DataSchema"));
    assert_eq!(calc.undo_label(), Some("New tab Data"));
    let text = calc
        .export_text("Data", gossia_calc::ExportFormat::XML, false)
        .unwrap();
    assert!(text.contains("<Data"), "{text}");

    app.session
        .calc_mut()
        .add_field(
            "Data",
            &Path::root(),
            "note",
            gossia_calc::FieldKind::Text,
            false,
        )
        .unwrap();
    app.session
        .calc_mut()
        .set_value("Data", &p("note"), Value::Str("hello".into()))
        .unwrap();
    settle(&mut app);
    assert!(valid(&app, "Data"));
    let text = app
        .calc()
        .export_text("Data", gossia_calc::ExportFormat::XML, false)
        .unwrap();
    assert!(text.contains("<Data><note>hello</note></Data>"), "{text}");

    // The control channel makes one too.
    control::run(
        &mut app,
        "new_tab",
        &json!({ "name": "Notes", "xml": true }),
    )
    .unwrap();
    assert!(app.calc().is_xml("Notes"));
}

const ORDER_XSD: &str = r#"<?xml version="1.0"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
  <!-- An order -->
  <xs:element name="order">
    <xs:complexType>
      <xs:sequence>
        <xs:element name="customer" type="xs:string"/>
      </xs:sequence>
    </xs:complexType>
  </xs:element>
</xs:schema>"#;

/// An XSD file is imported as a schema tab, which a data tab can then use;
/// it can still be imported as data instead.
#[test]
fn an_xsd_file_is_imported_as_a_schema_tab() {
    let mut app = sample();
    let dir = folder("xsd-import");
    let path = write(&dir, "order.xsd", ORDER_XSD);
    app.begin_import_path(&path).unwrap();
    let plan = app.imports.pop_back().unwrap();
    assert_eq!(plan.choice, SchemaChoice::SchemaTab);
    assert_eq!(plan.name, "order");
    assert!(plan.schema_problem.is_none());
    app.import(&plan).unwrap();
    let calc = app.calc();
    assert_eq!(
        calc.schema_format("order"),
        Some(gossia_calc::SchemaFormat::Xsd)
    );
    assert!(calc.schema_spec("order").is_ok());
    assert_eq!(app.sheet, "order");
    assert_eq!(calc.undo_label(), Some("New tab order"));

    // A data file is then given it.
    let path = write(
        &dir,
        "first.xml",
        "<order><customer>Acme</customer></order>",
    );
    app.begin_import_path(&path).unwrap();
    let plan = app.imports.pop_back().unwrap();
    assert_eq!(plan.choice, SchemaChoice::Existing("order".to_owned()));

    // As data, an XSD is XML like any other.
    let path = write(&dir, "other.xsd", ORDER_XSD);
    app.begin_import_path(&path).unwrap();
    let mut plan = app.imports.pop_back().unwrap();
    plan.choice = SchemaChoice::Inferred;
    app.import(&plan).unwrap();
    assert!(app.calc().is_xml("other"));
}

/// JSON that names the JSON Schema meta-schema, or a `*.schema.json` file,
/// is imported as a schema tab; one that cannot be lowered still is, with
/// the reason in the report.
#[test]
fn a_json_schema_file_is_imported_as_a_schema_tab() {
    let mut app = sample();
    let dir = folder("json-schema-import");
    let path = write(
        &dir,
        "person.json",
        r#"{"$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object", "properties": {"name": {"type": "string"}}}"#,
    );
    app.begin_import_path(&path).unwrap();
    let plan = app.imports.pop_back().unwrap();
    assert_eq!(plan.choice, SchemaChoice::SchemaTab);
    app.import(&plan).unwrap();
    assert_eq!(
        app.calc().schema_format("person"),
        Some(gossia_calc::SchemaFormat::Json)
    );
    assert!(app.calc().schema_spec("person").is_ok());

    // Plain JSON data is not taken for a schema.
    let path = write(&dir, "data.json", r#"{"name": "Ada"}"#);
    app.begin_import_path(&path).unwrap();
    let plan = app.imports.pop_back().unwrap();
    assert!(plan.schema_file.is_none());
    assert_ne!(plan.choice, SchemaChoice::SchemaTab);

    // By its name, and one that does not lower.
    let path = write(
        &dir,
        "odd.schema.json",
        r#"{"type": "object", "properties": {"x": {"type": "frog"}}}"#,
    );
    app.begin_import_path(&path).unwrap();
    let plan = app.imports.pop_back().unwrap();
    assert_eq!(plan.choice, SchemaChoice::SchemaTab);
    assert!(plan.schema_problem.is_some());
    assert!(
        plan.report()
            .iter()
            .any(|l| l.contains("cannot be used as a schema")),
        "{:?}",
        plan.report()
    );
    app.import(&plan).unwrap();
    assert!(app.calc().is_schema_sheet("odd_schema"));
}

/// JSON5, JSONC and JSON Lines are tab formats of their own (DESIGN.md,
/// "Other file formats"): the tab keeps the format and the comments, and the
/// whole import is one entry.
#[test]
fn a_json5_file_becomes_a_json5_tab_with_its_comments() {
    use gossia_core::dataformat::DataFormat;
    let mut app = sample();
    assert_eq!(format_of("Settings.JSON5"), ImportFormat::Json5);
    assert_eq!(format_of("a.jsonc"), ImportFormat::Jsonc);
    assert_eq!(format_of("log.ndjson"), ImportFormat::JsonLines);
    let text = "// Editor settings\n{\n  // in points\n  fontSize: 12,\n  theme: 'dark',\n}\n";
    let plan = app.plan_import("editor.json5", text.as_bytes()).unwrap();
    assert_eq!(plan.choice, SchemaChoice::Inferred);
    app.import(&plan).unwrap();
    let calc = app.calc();
    assert_eq!(calc.format("editor"), Some(DataFormat::Json5));
    assert_eq!(calc.comment("editor", &p("fontSize")), Some("in points"));
    assert_eq!(
        calc.comment("editor", &Path::root()),
        Some("Editor settings")
    );
    assert_eq!(calc.undo_label(), Some("Import editor.json5"));
    let raw = app.raw_text().to_owned();
    assert!(raw.contains("// in points\n  fontSize: 12"), "{raw}");
    app.session.calc_mut().undo().unwrap();
    assert!(!tabs(&app).contains(&"editor"));

    let lines = "{\"n\": 1}\n{\"n\": 2}\n";
    assert_eq!(format_of_text(lines), ImportFormat::JsonLines);
    assert_eq!(format_of_text("{\"n\": 1}"), ImportFormat::Json);
    let plan = app.plan_import("rows.jsonl", lines.as_bytes()).unwrap();
    app.import(&plan).unwrap();
    assert_eq!(app.calc().format("rows"), Some(DataFormat::JsonLines));
    let e = app.plan_import("bad.jsonl", b"{}\n{oops}\n").unwrap_err();
    assert!(e.contains("not JSON Lines") && e.contains("line 2"), "{e}");
}

/// *Format* on a tab's menu asks before it takes comments off.
#[test]
fn a_json_tab_keeps_its_comments_in_the_workbook() {
    use gossia_calc::Meta;
    use gossia_core::dataformat::DataFormat;
    let mut app = sample();
    let plan = app
        .plan_import(
            "editor.jsonc",
            b"{
  // big
  \"size\": 12
}",
        )
        .unwrap();
    app.import(&plan).unwrap();
    // JSON has no comments, so the workbook keeps them (owner, 2026-11-01):
    // nothing is lost and nothing is asked.
    app.begin_set_format("editor", DataFormat::Json).unwrap();
    assert!(app.tab_dialogs.format.is_none());
    assert_eq!(app.calc().format("editor"), Some(DataFormat::Json));
    assert_eq!(app.calc().comment("editor", &p("size")), Some("big"));
    assert!(app.calc().format_has_comments("editor"));
    app.session
        .calc_mut()
        .set_comment("editor", &p("size"), Some("bigger"))
        .unwrap();
    // The file is strict JSON; the comment is in the overlay.
    let package = app.calc().to_package(&Meta::default()).unwrap();
    let data = String::from_utf8(package.get("sheets/editor/data.json").unwrap().to_vec()).unwrap();
    assert!(!data.contains("bigger"), "{data}");
    let overlay =
        String::from_utf8(package.get("sheets/editor/overlay.json").unwrap().to_vec()).unwrap();
    assert!(overlay.contains("bigger"), "{overlay}");
    let back = gossia_calc::Calc::from_package(
        std::sync::Arc::new(gossia_calc::python::PythonSyntax),
        &package,
    )
    .unwrap()
    .calc;
    assert_eq!(back.comment("editor", &p("size")), Some("bigger"));
    // Export leaves them out, and says so.
    assert_eq!(app.calc().comments_left_out("editor", DataFormat::Json), 1);
    assert_eq!(app.calc().comments_left_out("editor", DataFormat::Json5), 0);
    assert!(
        !app.calc()
            .export_text(
                "editor",
                gossia_calc::ExportFormat::Data(DataFormat::Json),
                true
            )
            .unwrap()
            .contains("bigger")
    );
}

/// TOML is a tab format of its own, with its comments and dates.
#[test]
fn a_toml_file_becomes_a_toml_tab() {
    use gossia_core::dataformat::DataFormat;
    let mut app = sample();
    assert_eq!(format_of("Cargo.TOML"), ImportFormat::Toml);
    let text = "# The package\n[package]\nname = \"demo\" # short\nreleased = 2025-10-17\n";
    assert_eq!(format_of_text(text), ImportFormat::Toml);
    assert_eq!(format_of_text("[1, 2]"), ImportFormat::Json);
    assert_eq!(format_of_text("a = 1\nb = 'x'"), ImportFormat::Toml);
    assert_eq!(format_of_text("one = two = three"), ImportFormat::Text);
    let plan = app.plan_import("Cargo.toml", text.as_bytes()).unwrap();
    app.import(&plan).unwrap();
    let calc = app.calc();
    assert_eq!(calc.format("Cargo"), Some(DataFormat::Toml));
    assert_eq!(calc.comment("Cargo", &p("package")), Some("The package"));
    assert_eq!(calc.comment("Cargo", &p("package.name")), Some("short"));
    assert!(matches!(
        calc.data("Cargo")
            .unwrap()
            .get(&p("package.released"))
            .unwrap()
            .value(),
        Value::Date(_)
    ));
    let raw = app.raw_text().to_owned();
    assert!(raw.contains("[package]\n# short\nname = \"demo\""), "{raw}");
    // Orders has an object at the top, so it could be written as TOML.
    assert!(
        app.calc()
            .format_problem("Orders", DataFormat::Toml)
            .is_none()
    );
}

#[test]
fn a_yaml_file_becomes_a_yaml_tab() {
    use gossia_core::dataformat::DataFormat;
    let mut app = sample();
    assert_eq!(format_of("compose.YML"), ImportFormat::Yaml);
    let text = "# Services\nweb:\n  image: nginx # pinned\n  ports:\n    - 80\n";
    assert_eq!(format_of_text(text), ImportFormat::Yaml);
    assert_eq!(format_of_text("- a\n- b\n"), ImportFormat::Yaml);
    assert_eq!(
        format_of_text("a: 1\n---\na: 2\n"),
        ImportFormat::YamlDocuments
    );
    assert_eq!(
        format_of_text("Dear Ann: hello\nthere: \"x"),
        ImportFormat::Text
    );
    assert_eq!(format_of_text("one, two"), ImportFormat::Csv);
    let plan = app.plan_import("compose.yaml", text.as_bytes()).unwrap();
    app.import(&plan).unwrap();
    let calc = app.calc();
    assert_eq!(calc.format("compose"), Some(DataFormat::Yaml));
    assert_eq!(calc.comment("compose", &p("web.image")), Some("pinned"));
    let raw = app.raw_text().to_owned();
    assert!(raw.contains("  # pinned\n  image: nginx\n"), "{raw}");
    // Several documents make a tab of YAML documents.
    let plan = app
        .plan_import("pods.yaml", b"name: a\n---\nname: b\n")
        .unwrap();
    app.import(&plan).unwrap();
    assert_eq!(app.calc().format("pods"), Some(DataFormat::YamlDocuments));
}
