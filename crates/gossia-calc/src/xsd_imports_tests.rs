//! Tests of XSD tabs that import each other and have several global elements
//! (DESIGN.md, XML Schemas and namespaces, 0.5.0, step 15b-2).

use std::sync::Arc;

use gossia_core::{Kind, Path, Value, metaschema, xml};

use crate::python::PythonSyntax;
use crate::{Calc, ExportFormat, Meta};

/// The main schema: a book in `urn:book`, with a title from `urn:dc`, which
/// it imports from the tab `Dc` (written as a file name), and a second global
/// element.
const BOOK: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dc="urn:dc" xmlns:b="urn:book" targetNamespace="urn:book" elementFormDefault="qualified">
  <xs:import namespace="urn:dc" schemaLocation="dc.xsd"/>
  <xs:element name="book">
    <xs:complexType>
      <xs:sequence>
        <xs:element ref="dc:title"/>
        <xs:element name="pages" type="xs:integer"/>
      </xs:sequence>
      <xs:attribute ref="dc:lang"/>
    </xs:complexType>
  </xs:element>
  <xs:element name="shelf">
    <xs:complexType>
      <xs:sequence>
        <xs:element ref="b:book" maxOccurs="unbounded"/>
      </xs:sequence>
    </xs:complexType>
  </xs:element>
</xs:schema>
"#;

const DC: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" targetNamespace="urn:dc" elementFormDefault="qualified">
  <xs:element name="title" type="xs:string"/>
  <xs:attribute name="lang" type="xs:string"/>
</xs:schema>
"#;

const A_BOOK: &str = r#"<b:book xmlns:b="urn:book" xmlns:dc="urn:dc" dc:lang="en"><dc:title>Emma</dc:title><b:pages>474</b:pages></b:book>"#;

const A_SHELF: &str = r#"<shelf xmlns="urn:book" xmlns:dc="urn:dc"><book><dc:title>Emma</dc:title><pages>474</pages></book><book><dc:title>Persuasion</dc:title><pages>249</pages></book></shelf>"#;

fn p(text: &str) -> Path {
    if text.is_empty() {
        Path::root()
    } else {
        text.parse().unwrap()
    }
}

fn add_xsd(calc: &mut Calc, name: &str, text: &str) {
    let parsed = xml::parse(text, metaschema::xsd_meta()).unwrap();
    calc.add_xsd_sheet(name, parsed.node, parsed.document)
        .unwrap();
}

/// `Book` imports `Dc`; `Books` holds a book, `Shelf` a shelf.
fn book() -> Calc {
    let mut calc = Calc::new(Arc::new(PythonSyntax));
    // The importing tab first: it lowers once the imported one is there.
    add_xsd(&mut calc, "Book", BOOK);
    assert!(
        calc.schema_error("Book")
            .is_some_and(|e| e.contains("dc.xsd"))
    );
    add_xsd(&mut calc, "Dc", DC);
    assert_eq!(calc.schema_error("Book"), None);
    let spec = calc.schema_spec("Book").unwrap();
    let parsed = xml::parse(A_BOOK, &spec).unwrap();
    calc.create_xml_tab("Books", "Book", None, parsed.node, parsed.document)
        .unwrap();
    let spec = calc.record_spec("Book", "shelf").unwrap();
    let parsed = xml::parse(A_SHELF, &spec).unwrap();
    calc.create_xml_tab("Shelf", "Book", Some("shelf"), parsed.node, parsed.document)
        .unwrap();
    calc.clear_history();
    calc
}

fn fields(calc: &Calc, tab: &str, object: &str) -> Vec<String> {
    let spec = calc.spec(tab).unwrap();
    match &spec.get(spec.resolve(&p(object)).unwrap().ty).kind {
        Kind::Object { fields } => fields.iter().map(|f| f.name.clone()).collect(),
        other => panic!("not an object: {other:?}"),
    }
}

fn text_of(calc: &Calc, tab: &str) -> String {
    calc.export_text(tab, ExportFormat::XML, false).unwrap()
}

#[test]
fn an_import_names_the_schema_tab_of_its_namespace() {
    let calc = book();
    assert_eq!(calc.xsd_import_tab("dc.xsd").as_deref(), Some("Dc"));
    assert_eq!(calc.xsd_import_tab("schemas/DC.xsd").as_deref(), Some("Dc"));
    assert_eq!(calc.xsd_import_tab("Dc").as_deref(), Some("Dc"));
    assert_eq!(calc.xsd_import_tab("Books"), None, "not a schema tab");
    assert_eq!(calc.xsd_importers("Dc"), ["Book"]);
    assert_eq!(
        fields(&calc, "Books", ""),
        ["@dc:lang", "dc:title", "pages"]
    );
    assert_eq!(
        calc.data("Books")
            .unwrap()
            .get(&p("[\"dc:title\"]"))
            .unwrap()
            .value(),
        &Value::Str("Emma".into())
    );
    let text = text_of(&calc, "Books");
    assert!(text.ends_with(&format!("{A_BOOK}\n")), "{text}");
}

#[test]
fn editing_the_imported_tab_retypes_the_tabs_of_the_importing_one() {
    let mut calc = book();
    let at = p("element[0][\"@type\"]");
    calc.set_value("Dc", &at, Value::Str("xs:integer".into()))
        .unwrap();
    assert_eq!(calc.schema_error("Book"), None);
    let kind = |calc: &Calc| {
        let spec = calc.spec("Books").unwrap();
        spec.get(spec.resolve(&p("[\"dc:title\"]")).unwrap().ty)
            .kind
            .clone()
    };
    assert!(matches!(kind(&calc), Kind::Integer(_)));
    calc.undo().unwrap();
    assert!(matches!(kind(&calc), Kind::String(_)));
}

#[test]
fn renaming_the_imported_tab_renames_the_import_and_deleting_it_is_refused() {
    let mut calc = book();
    calc.rename_sheet("Dc", "Dublin").unwrap();
    assert_eq!(
        calc.data("Book")
            .unwrap()
            .get(&p("import[0][\"@schemaLocation\"]"))
            .unwrap()
            .value(),
        &Value::Str("Dublin".into())
    );
    assert_eq!(calc.schema_error("Book"), None);
    let why = calc.delete_problem("Dublin").expect("refused");
    assert!(why.contains("Book imports this schema"), "{why}");
    calc.undo().unwrap();
    assert_eq!(calc.xsd_import_tab("Dc").as_deref(), Some("Dc"));
    assert_eq!(calc.schema_error("Book"), None);
}

#[test]
fn an_exported_xsd_names_the_files_of_the_tabs_it_imports() {
    let calc = book();
    let text = text_of(&calc, "Book");
    assert!(text.contains(r#"schemaLocation="Dc.xsd""#), "{text}");
}

#[test]
fn a_tab_holds_the_global_element_it_was_made_with() {
    let calc = book();
    assert_eq!(calc.xsd_roots("Book"), ["book", "shelf"]);
    assert_eq!(calc.spec("Shelf").unwrap().xml().unwrap().root, "shelf");
    assert_eq!(
        fields(&calc, "Shelf", "book[0]"),
        ["@dc:lang", "dc:title", "pages"]
    );
    let text = text_of(&calc, "Shelf");
    assert!(text.contains("<dc:title>Persuasion</dc:title>"), "{text}");
}

#[test]
fn the_global_element_and_the_imports_are_saved_and_read_back() {
    let calc = book();
    let meta = Meta {
        workbook_id: "0123456789abcdef".to_owned(),
        app_version: "0.5.0".to_owned(),
        ..Meta::default()
    };
    let package = calc.to_package(&meta).unwrap();
    let back = Calc::from_package(Arc::new(PythonSyntax), &package)
        .unwrap()
        .calc;
    assert_eq!(back.schema_error("Book"), None);
    assert_eq!(back.spec("Shelf").unwrap().xml().unwrap().root, "shelf");
    assert_eq!(text_of(&back, "Shelf"), text_of(&calc, "Shelf"));
    assert_eq!(text_of(&back, "Books"), text_of(&calc, "Books"));
}

#[test]
fn rebinding_keeps_the_root_of_the_tab() {
    let mut calc = book();
    add_xsd(&mut calc, "Book2", &BOOK.replace("pages", "leaves"));
    calc.rebind_sheet("Shelf", "Book2").unwrap();
    assert_eq!(calc.spec("Shelf").unwrap().xml().unwrap().root, "shelf");
    calc.undo().unwrap();
    assert_eq!(calc.schema_of("Shelf"), Some("Book"));
    assert_eq!(calc.spec("Shelf").unwrap().xml().unwrap().root, "shelf");
}

#[test]
fn field_commands_work_on_refs() {
    let mut calc = book();
    // Auto-update needs one tab that uses the schema.
    calc.delete_sheet("Shelf").unwrap();
    calc.set_auto_update("Book", true).unwrap();
    let why = calc
        .rename_field("Books", &Path::root(), "dc:title", "name")
        .unwrap_err()
        .to_string();
    assert!(why.contains("global element"), "{why}");
    let why = calc
        .rename_field("Books", &Path::root(), "@dc:lang", "@language")
        .unwrap_err()
        .to_string();
    assert!(why.contains("global attribute"), "{why}");
    calc.delete_field("Books", &Path::root(), "dc:title")
        .unwrap();
    assert_eq!(fields(&calc, "Books", ""), ["@dc:lang", "pages"]);
    assert!(!text_of(&calc, "Book").contains("dc:title"));
}
