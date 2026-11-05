//! Tests of COBOL copybooks and records in the window (DESIGN.md, "COBOL
//! copybooks"): importing `.cpy` and `.blk` files, the Raw view's dump, the
//! layouts of `REDEFINES`, the code page, and new copybook tabs.

use std::time::Duration;

use gossia_calc::{Meta, SchemaFormat};
use gossia_core::binary::{BinaryText, to_base64, to_hex};
use gossia_core::copybook::CodePage;
use gossia_core::dataformat::DataFormat;
use gossia_core::{Path, Value};

use crate::GossiaApp;
use crate::import::{ImportFormat, SchemaChoice, format_of};
use crate::tab_commands::NewSchema;

const COPYBOOK: &str = "\
      * Customers
       01  CUSTOMER-REC.
           05  CUST-ID        PIC 9(4).
           05  AREA-TEXT      PIC X(4).
           05  AREA-NUMBER REDEFINES AREA-TEXT PIC 9(4).
       01  SHORT-REC.
           05  CODE           PIC X(4).
       01  PAIR-REC.
           05  PAIR           PIC X(8).
";

fn p(text: &str) -> Path {
    text.parse().unwrap()
}

fn sample() -> GossiaApp {
    let mut app = GossiaApp::sample();
    app.files.browse = false;
    assert!(app.session.wait_idle(Duration::from_secs(60)));
    app
}

fn ebcdic(text: &str) -> Vec<u8> {
    CodePage::Cp037.encode(text).unwrap()
}

/// The sample with the copybook imported as the tab `customer`.
fn with_copybook() -> GossiaApp {
    let mut app = sample();
    let plan = app
        .plan_import("customer.cpy", COPYBOOK.as_bytes())
        .unwrap();
    assert_eq!(plan.choice, SchemaChoice::SchemaTab);
    app.import(&plan).unwrap();
    app
}

fn at(app: &GossiaApp, tab: &str, path: &str) -> Value {
    app.calc()
        .data(tab)
        .unwrap()
        .get(&p(path))
        .unwrap()
        .value()
        .clone()
}

#[test]
fn copybooks_and_records_are_told_by_their_extension() {
    assert_eq!(format_of("CUST.CPY"), ImportFormat::Copybook);
    assert_eq!(format_of("people.blk"), ImportFormat::Blk);
}

#[test]
fn a_copybook_becomes_a_schema_tab_shown_as_its_text() {
    let mut app = with_copybook();
    assert_eq!(
        app.calc().schema_format("customer"),
        Some(SchemaFormat::Copybook)
    );
    assert_eq!(
        app.calc().copybook_records("customer"),
        ["CUSTOMER-REC", "SHORT-REC", "PAIR-REC"]
    );
    let raw = app.raw_text().to_owned();
    assert!(raw.contains("      * Customers\n"), "{raw}");
    assert!(raw.contains("       01  CUSTOMER-REC.\n"), "{raw}");
}

#[test]
fn a_record_is_read_with_a_copybook_and_a_record_of_it() {
    let mut app = with_copybook();
    let bytes = ebcdic("0002ABCD");
    let plan = app.plan_import("people.blk", &bytes).unwrap();
    // Two records of the copybook are 8 bytes long; the first is chosen.
    assert_eq!(plan.candidates.len(), 2);
    assert_eq!(plan.choice, SchemaChoice::Existing("customer".into()));
    assert_eq!(plan.record.as_deref(), Some("CUSTOMER-REC"));
    app.import(&plan).unwrap();
    assert_eq!(app.calc().format("people"), Some(DataFormat::Blk));
    // The tab holds the one record: its root is the record object.
    assert_eq!(at(&app, "people", "CUST_ID"), Value::Int(2));
    let raw = app.raw_text().to_owned();
    assert!(raw.starts_with("00000000  F0 F0 F0 F2"), "{raw}");
    assert!(raw.ends_with("0002ABCD"), "{raw}");

    // The other record of the copybook, kept when the workbook is saved.
    let mut plan = app.plan_import("codes.blk", &bytes).unwrap();
    plan.record = Some("PAIR-REC".into());
    app.import(&plan).unwrap();
    assert_eq!(app.calc().record_of("codes").as_deref(), Some("PAIR-REC"));
    assert_eq!(at(&app, "codes", "PAIR"), Value::Str("0002ABCD".into()));
    let package = app.calc().to_package(&Meta::default()).unwrap();
    let back = gossia_calc::Calc::from_package(
        std::sync::Arc::new(gossia_calc::python::PythonSyntax),
        &package,
    )
    .unwrap()
    .calc;
    assert_eq!(back.record_of("codes").as_deref(), Some("PAIR-REC"));
    assert_eq!(back.data("codes"), app.calc().data("codes"));
}

#[test]
fn a_file_must_be_one_record_long() {
    let app = sample();
    let why = app.plan_import("people.blk", &ebcdic("0001")).unwrap_err();
    assert!(why.contains("Import its copybook"), "{why}");
    let app = with_copybook();
    // Two records' worth is not one record.
    let why = app
        .plan_import("people.blk", &ebcdic("0001ABCD0002WXYZ"))
        .unwrap_err();
    assert!(why.contains("a record of its 16 bytes"), "{why}");
    // 4 bytes are SHORT-REC.
    let plan = app.plan_import("codes.blk", &ebcdic("WXYZ")).unwrap();
    assert_eq!(plan.record.as_deref(), Some("SHORT-REC"));
}

#[test]
fn showing_another_layout_asks_before_deleting_formulas() {
    let mut app = with_copybook();
    let plan = app.plan_import("people.blk", &ebcdic("00021234")).unwrap();
    app.import(&plan).unwrap();
    app.session
        .calc_mut()
        .set_formula("people", &p("AREA_TEXT"), "'4321'")
        .unwrap();
    app.begin_set_redefine("people", 0, 1).unwrap();
    let change = app.tab_dialogs.redefine.clone().unwrap();
    assert_eq!(
        (change.from.as_str(), change.to.as_str()),
        ("AREA-TEXT", "AREA-NUMBER")
    );
    assert_eq!(change.formulas, 1);
    // Nothing changes until the answer is Yes.
    assert_eq!(app.calc().redefine_choices("people"), Vec::<usize>::new());
    app.set_redefine("people", 0, 1).unwrap();
    assert!(app.calc().formula_text("people", &p("AREA_TEXT")).is_none());
    // With no formulas there, the change is made without asking.
    app.tab_dialogs.redefine = None;
    app.begin_set_redefine("people", 0, 0).unwrap();
    assert!(app.tab_dialogs.redefine.is_none());
    assert!(matches!(at(&app, "people", "AREA_TEXT"), Value::Str(_)));
}

#[test]
fn the_code_page_is_set_on_the_copybook_tab() {
    let mut app = with_copybook();
    let plan = app.plan_import("people.blk", &ebcdic("0001ABCD")).unwrap();
    app.import(&plan).unwrap();
    app.set_code_page("customer", CodePage::Cp500).unwrap();
    let calc = app.calc();
    let layout = calc.spec("people").unwrap().copybook().unwrap();
    assert_eq!(layout.code_page, CodePage::Cp500);
}

#[test]
fn a_new_tab_can_have_its_own_copybook() {
    let mut app = sample();
    app.begin_new_tab();
    let mut dialog = app.tab_dialogs.new.clone().unwrap();
    dialog.name = "Ledger".into();
    dialog.schema_name = "LedgerBook".into();
    dialog.schema = NewSchema::Own(SchemaFormat::Copybook);
    app.create_tab(&dialog).unwrap();
    let calc = app.calc();
    assert_eq!(
        calc.schema_format("LedgerBook"),
        Some(SchemaFormat::Copybook)
    );
    assert_eq!(calc.format("Ledger"), Some(DataFormat::Blk));
    assert!(!calc.auto_update("LedgerBook"));
    assert_eq!(calc.record_of("Ledger").as_deref(), Some("LEDGER-REC"));
}

#[test]
fn records_are_copied_as_options_say_and_pasted_back() {
    let mut app = with_copybook();
    let bytes = ebcdic("0002ABCD");
    let plan = app.plan_import("people.blk", &bytes).unwrap();
    app.import(&plan).unwrap();
    for kind in BinaryText::ALL {
        app.user.prefs.binary_copy = kind;
        let copied = app.records_text().unwrap().unwrap();
        assert_eq!(kind.read(&copied), Some(bytes.clone()), "{kind:?}");
    }
    // Hex digits pasted on the tab offer to replace its record.
    app.user.prefs.binary_copy = BinaryText::Base64;
    let other = ebcdic("0009WXYZ");
    let mut plan = app.plan_paste(&to_hex(&other)).unwrap();
    assert_eq!(plan.format, ImportFormat::Blk);
    assert_eq!(plan.replace.as_deref(), Some("people"));
    assert!(plan.notes[0].contains("8 bytes"), "{:?}", plan.notes);
    plan.choice = SchemaChoice::Replace("people".into());
    app.import(&plan).unwrap();
    assert_eq!(at(&app, "people", "CUST_ID"), Value::Int(9));
    assert_eq!(app.calc().records_bytes("people").unwrap(), other);
    // Or make a new tab, as a dropped .blk does.
    let plan = app.plan_paste(&to_base64(&bytes)).unwrap();
    assert_eq!(plan.choice, SchemaChoice::Existing("customer".into()));
    app.import(&plan).unwrap();
    assert_eq!(at(&app, "Pasted", "CUST_ID"), Value::Int(2));
    // A dump is read on any tab; other text on a tab of records is not.
    app.show_tab("customer");
    let dump = BinaryText::Dump.write(&bytes, 8, CodePage::Cp037);
    assert_eq!(app.plan_paste(&dump).unwrap().format, ImportFormat::Blk);
    app.show_tab("people");
    let plan = app.plan_paste("name,age\nAnn,3\n").unwrap();
    assert_eq!(plan.format, ImportFormat::Csv);
}

#[test]
fn holding_another_record_asks_before_deleting_formulas() {
    let mut app = with_copybook();
    let plan = app.plan_import("people.blk", &ebcdic("0001ABCD")).unwrap();
    app.import(&plan).unwrap();
    assert_eq!(
        app.calc().record_of("people").as_deref(),
        Some("CUSTOMER-REC")
    );
    let why = app.begin_set_record("people", "SHORT-REC").unwrap_err();
    assert!(why.contains("4 bytes"), "{why}");

    app.session
        .calc_mut()
        .set_formula("people", &p("CUST_ID"), "7")
        .unwrap();
    app.begin_set_record("people", "PAIR-REC").unwrap();
    let change = app.tab_dialogs.record.clone().unwrap();
    assert_eq!(change.formulas, 1);
    assert_eq!(
        app.calc().record_of("people").as_deref(),
        Some("CUSTOMER-REC")
    );
    app.set_record("people", "PAIR-REC").unwrap();
    assert_eq!(app.calc().record_of("people").as_deref(), Some("PAIR-REC"));
    assert!(matches!(at(&app, "people", "PAIR"), Value::Str(s) if s.ends_with("ABCD")));
}

#[test]
fn floating_point_is_set_on_the_copybook_tab() {
    use gossia_core::copybook::FloatFormat;
    let mut app = sample();
    let text = "       01  R.\n           05  A COMP-1.\n";
    let plan = app.plan_import("floats.cpy", text.as_bytes()).unwrap();
    app.import(&plan).unwrap();
    let plan = app
        .plan_import("one.blk", &[0x3F, 0x80, 0x00, 0x00])
        .unwrap();
    app.import(&plan).unwrap();
    // IBM hexadecimal: 0.5 times 16 to the power -1.
    assert_eq!(at(&app, "one", "A"), Value::Float(0.03125));
    app.set_float_format("floats", FloatFormat::Ieee).unwrap();
    let calc = app.calc();
    assert_eq!(
        calc.spec("one").unwrap().copybook().unwrap().float,
        FloatFormat::Ieee
    );
    assert_eq!(calc.records_bytes("one").unwrap().len(), 4);
}

#[test]
fn a_file_with_a_record_descriptor_word_is_a_variable_length_record() {
    let mut app = sample();
    let text = "       01  R.\n           05  N PIC 9.\n           05  T PIC X OCCURS 0 TO 5 DEPENDING ON N.\n";
    let plan = app.plan_import("vb.cpy", text.as_bytes()).unwrap();
    app.import(&plan).unwrap();
    let file = [0x00, 0x07, 0x00, 0x00, 0xF2, 0xC1, 0xC2];
    let plan = app.plan_import("two.blk", &file).unwrap();
    app.import(&plan).unwrap();
    assert!(app.calc().is_variable("two"));
    assert_eq!(app.calc().records_bytes("two").unwrap(), file);
    app.set_variable("two", false).unwrap();
    assert_eq!(app.calc().records_bytes("two").unwrap().len(), 6);
}

#[test]
fn copybook_text_pasted_on_a_copybook_tab_can_be_its_text() {
    let mut app = with_copybook();
    app.set_code_page("customer", CodePage::Cp500).unwrap();
    let text = "       01  NEW-REC.\n           05  X PIC X.\n";
    // On another tab it is a new copybook schema tab.
    let other = app
        .calc()
        .sheet_names()
        .find(|n| *n != "customer")
        .unwrap()
        .to_owned();
    app.show_tab(&other);
    let plan = app.plan_paste(text).unwrap();
    assert_eq!(plan.format, ImportFormat::Copybook);
    assert_eq!(plan.replace, None);
    app.show_tab("customer");
    let mut plan = app.plan_paste(text).unwrap();
    assert_eq!(plan.replace.as_deref(), Some("customer"));
    assert_eq!(plan.choice, SchemaChoice::SchemaTab);
    plan.choice = SchemaChoice::Replace("customer".into());
    app.import(&plan).unwrap();
    assert_eq!(app.calc().copybook_records("customer"), ["NEW-REC"]);
    // The code page is a setting of the tab, which the text does not say.
    assert_eq!(
        at(&app, "customer", "codePage"),
        Value::Str(CodePage::Cp500.code().into())
    );
}

#[test]
fn copy_members_are_read_from_the_folder_of_the_file() {
    let dir = std::env::temp_dir().join(format!(
        "gossia-copy-members-{}-{}",
        std::process::id(),
        crate::workbook_file::new_workbook_id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("ADDR.cpy"),
        "           05  :P:-STREET     PIC X(10).\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("main.cpy"),
        "       01  CUSTOMER.\n           05  ID PIC 9(4).\n           COPY ADDR REPLACING ==:P:== BY ==HOME==.\n",
    )
    .unwrap();
    let mut app = sample();
    app.begin_import_path(&dir.join("main.cpy")).unwrap();
    let plan = app.imports.pop_front().unwrap();
    app.import(&plan).unwrap();
    let spec = app.calc().schema_spec("main").unwrap();
    assert_eq!(spec.copybook().unwrap().length, 14);
    assert!(spec.resolve(&p("HOME_STREET")).is_some());
    // A member that is not there says so.
    std::fs::remove_file(dir.join("ADDR.cpy")).unwrap();
    let why = app.begin_import_path(&dir.join("main.cpy")).unwrap_err();
    assert!(why.contains("no member ADDR"), "{why}");
    let _ = std::fs::remove_dir_all(&dir);
}
