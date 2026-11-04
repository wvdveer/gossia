//! `cliotree-cli report` (DESIGN.md, "Batch export from the command line",
//! and Testing, "Reports"): each exit code from a run that ends that way,
//! that a failed run leaves no file, that the workbook is never changed, and
//! that the results cache and remembered values are not used.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::Arc;

use cliotree_calc::python::PythonSyntax;
use cliotree_calc::sources::{SOURCES_FILE, SourceDef, SourceKind, SourceStore};
use cliotree_calc::{Calc, Meta, Setting};
use cliotree_core::form::{Control, ControlKind};
use cliotree_core::report::{ParamKind, ReportDesign, ReportKind, ReportParam};
use cliotree_core::{Value, json, jsonschema};
use cliotree_user::file::{new_workbook_id, read_zip, write_zip};
use cliotree_user::prefs::{Config, User};

const SCHEMA: &str = r#"{
  "type": "object",
  "properties": {
    "customer": {"type": "string"},
    "items": {"type": "array", "items": {"type": "object", "properties": {
      "sku": {"type": "string"}, "qty": {"type": "integer"}, "price": {"type": "number"}
    }}}
  }
}"#;

const ORDERS: &str = r#"{
  "customer": "Acme Ltd",
  "items": [
    {"sku": "A1", "qty": 2, "price": 9.5},
    {"sku": "B2", "qty": 1, "price": 24.5},
    {"sku": "C3", "qty": 12, "price": 0.75}
  ]
}"#;

/// A workbook on disk, the configuration folder its runs use, and its
/// trust digest.
struct Book {
    dir: PathBuf,
    file: PathBuf,
    config: PathBuf,
    id: String,
    digest: String,
}

impl Drop for Book {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn field(label: &str, value: &str) -> Control {
    let mut c = Control::new(ControlKind::Field);
    c.label = Some(label.to_owned());
    c.value = Some(value.to_owned());
    c
}

/// The Orders tab and a report `Summary` of `controls` with `params`;
/// `change` changes the workbook before it is saved.
fn book(
    tag: &str,
    controls: Vec<Control>,
    params: Vec<ReportParam>,
    change: impl FnOnce(&mut Calc),
) -> Book {
    let mut calc = Calc::new(Arc::new(PythonSyntax));
    calc.add_sheet(
        "Orders",
        jsonschema::lower_str(SCHEMA).unwrap(),
        json::parse(ORDERS).unwrap().node,
    )
    .unwrap();
    calc.add_schema_sheet("OrdersSchema", json::parse(SCHEMA).unwrap().node)
        .unwrap();
    calc.bind_schema("Orders", "OrdersSchema").unwrap();
    let mut design = ReportDesign::new(ReportKind::SinglePage);
    design.layout.controls = controls;
    design.params = params;
    calc.create_report("Summary", design).unwrap();
    change(&mut calc);
    let id = new_workbook_id();
    let meta = Meta {
        workbook_id: id.clone(),
        app_version: "0.2.0".to_owned(),
        views: BTreeMap::new(),
        workbook_view: None,
        unknown: Vec::new(),
    };
    let dir = std::env::temp_dir().join(format!("cliotree-cli-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("book.clio");
    std::fs::write(&file, write_zip(&calc.to_package(&meta).unwrap()).unwrap()).unwrap();
    let config = dir.join("config");
    std::fs::create_dir_all(&config).unwrap();
    Book {
        file,
        config,
        id,
        digest: calc.fingerprint().hash,
        dir,
    }
}

/// The Summary report with the total and the customer.
fn summary(tag: &str) -> Book {
    book(
        tag,
        vec![
            field("Customer", "Orders.customer"),
            field("Total", "sum(i.qty * i.price for i in Orders.items)"),
        ],
        Vec::new(),
        |_| {},
    )
}

impl Book {
    fn pdf(&self) -> PathBuf {
        self.dir.join("out.pdf")
    }

    /// Runs `cliotree-cli` with `args`, `{book}`, `{pdf}` and `{digest}`
    /// standing for this workbook's.
    fn run(&self, args: &[&str]) -> Output {
        let args: Vec<String> = args
            .iter()
            .map(|a| {
                a.replace("{book}", &self.file.display().to_string())
                    .replace("{pdf}", &self.pdf().display().to_string())
                    .replace("{digest}", &self.digest)
            })
            .collect();
        Command::new(env!("CARGO_BIN_EXE_cliotree-cli"))
            .args(&args)
            .env("CLIOTREE_CONFIG_DIR", &self.config)
            .output()
            .unwrap()
    }

    /// Runs the Summary report to `{pdf}`, trusted by its digest.
    fn report(&self, more: &[&str]) -> Output {
        let mut args = vec![
            "report",
            "{book}",
            "Summary",
            "--out",
            "{pdf}",
            "--trust-digest",
            "{digest}",
        ];
        args.extend_from_slice(more);
        self.run(&args)
    }

    fn text(&self) -> String {
        pdf_extract::extract_text_from_mem(&std::fs::read(self.pdf()).unwrap()).unwrap()
    }
}

fn code(output: &Output) -> i32 {
    output.status.code().unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn squeezed(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn a_trusted_report_is_written_and_its_path_printed_and_the_workbook_is_unchanged() {
    let b = summary("written");
    let before = std::fs::read(&b.file).unwrap();
    let output = b.report(&[]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert_eq!(stdout(&output).trim(), b.pdf().display().to_string());
    let text = squeezed(&b.text());
    assert!(text.contains("Acme Ltd"), "{text}");
    assert!(text.contains("52.5"), "{text}");
    assert_eq!(std::fs::read(&b.file).unwrap(), before);
    // No temporary file is left beside it.
    let left: Vec<_> = std::fs::read_dir(&b.dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(left.len(), 3, "{left:?}");
}

#[test]
fn a_workbook_in_the_users_trust_record_needs_no_digest() {
    let b = summary("record");
    let (mut user, _) = User::load(Config::at(&b.config));
    let opened = Calc::from_package(
        Arc::new(PythonSyntax),
        &cliotree_user::file::read_zip(&std::fs::read(&b.file).unwrap(), &Default::default())
            .unwrap(),
    )
    .unwrap();
    let fingerprint = opened.calc.fingerprint();
    user.change_trust(|t| t.trust(&b.id, fingerprint)).unwrap();
    let output = b.run(&["report", "{book}", "Summary", "--out", "{pdf}"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(b.pdf().exists());
}

#[test]
fn an_untrusted_workbook_runs_nothing_and_writes_nothing() {
    let b = summary("untrusted");
    let output = b.run(&["report", "{book}", "Summary", "--out", "{pdf}"]);
    assert_eq!(code(&output), 3);
    assert!(
        stderr(&output).contains("is not trusted"),
        "{}",
        stderr(&output)
    );
    assert!(!b.pdf().exists());
    let wrong = b.run(&[
        "report",
        "{book}",
        "Summary",
        "--out",
        "{pdf}",
        "--trust-digest",
        "00ff",
    ]);
    assert_eq!(code(&wrong), 3);
    assert!(stderr(&wrong).contains("not this workbook's"));
    assert!(!b.pdf().exists());
}

#[test]
fn a_wrong_command_line_is_code_1_with_the_usage() {
    let b = summary("usage");
    let output = b.run(&["report", "{book}", "Summary"]);
    assert_eq!(code(&output), 1);
    assert!(stderr(&output).contains("needs --out"));
    assert!(stderr(&output).contains("Usage:"));
    // An existing file is kept unless --overwrite.
    std::fs::write(b.pdf(), b"mine").unwrap();
    let kept = b.report(&[]);
    assert_eq!(code(&kept), 1);
    assert!(stderr(&kept).contains("--overwrite"));
    assert_eq!(std::fs::read(b.pdf()).unwrap(), b"mine");
    let replaced = b.report(&["--overwrite"]);
    assert_eq!(code(&replaced), 0, "{}", stderr(&replaced));
    assert!(std::fs::read(b.pdf()).unwrap().starts_with(b"%PDF"));
}

#[test]
fn a_missing_workbook_or_report_is_code_2() {
    let b = summary("missing");
    let output = b.run(&[
        "report",
        "{book}",
        "Sales",
        "--out",
        "{pdf}",
        "--trust-digest",
        "{digest}",
    ]);
    assert_eq!(code(&output), 2);
    assert!(
        stderr(&output).contains("has no report Sales; its reports are Summary"),
        "{}",
        stderr(&output)
    );
    let gone = b.run(&["report", "nowhere.clio", "Summary", "--out", "{pdf}"]);
    assert_eq!(code(&gone), 2);
    assert!(!b.pdf().exists());
}

fn choice(name: &str, list: &str) -> ReportParam {
    let mut p = ReportParam::new(name, ParamKind::Choice);
    p.choices = Some(list.to_owned());
    p
}

/// A report showing its parameters.
fn with_params(tag: &str, change: impl FnOnce(&mut Calc)) -> Book {
    let mut since = ReportParam::new("since", ParamKind::Date);
    since.default = Some("_date('2026-01-01')".to_owned());
    let mut units = ReportParam::new("units", ParamKind::Choices);
    units.choices = Some("[('N', 'North'), ('S', 'South')]".to_owned());
    let mut who = ReportParam::new("who", ParamKind::Text);
    who.required = true;
    let mut big = ReportParam::new("big", ParamKind::YesNo);
    big.default = Some("False".to_owned());
    book(
        tag,
        vec![
            field("Since", "'since ' + str(params.since)"),
            field("Unit", "params.unit or '-'"),
            field("Units", "'+'.join(params.units or [])"),
            field("Who", "params.who"),
            field("Big", "'big' if params.big else 'small'"),
            field("Cap", "(params.cap or 0) * 2"),
            field(
                "Span",
                "str(params.span.start) + '/' + str(params.span.end) if params.span else '-'",
            ),
        ],
        vec![
            since,
            choice("unit", "['East', 'West']"),
            units,
            who,
            big,
            ReportParam::new("cap", ParamKind::Number),
            ReportParam::new("span", ParamKind::Period),
        ],
        change,
    )
}

#[test]
fn parameters_are_read_by_their_kind_and_the_rest_take_their_defaults() {
    let b = with_params("params", |_| {});
    let output = b.report(&[
        "--param",
        "unit=West",
        "--param",
        "units=S",
        "--param",
        "units=N",
        "--param",
        "who=Ward",
        "--param",
        "big=yes",
        "--param",
        "cap=2.5",
        "--param",
        "span=2026-10-01..2026-10-31",
    ]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let text = squeezed(&b.text());
    for shown in [
        "since 2026-01-01",
        "West",
        "S+N",
        "Ward",
        "big",
        "5.0",
        "2026-10-01/2026-10-31",
    ] {
        assert!(text.contains(shown), "{shown} in {text}");
    }
}

#[test]
fn a_remembered_value_is_not_used() {
    let b = with_params("remembered", |calc| {
        calc.set_report_param("Summary", "who", Some(Value::Str("Remembered".into())));
        calc.set_report_param("Summary", "unit", Some(Value::Str("East".into())));
    });
    let output = b.report(&["--param", "who=Given"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let text = squeezed(&b.text());
    assert!(text.contains("Given"), "{text}");
    assert!(!text.contains("Remembered"), "{text}");
    assert!(!text.contains("East"), "{text}");
}

#[test]
fn a_parameter_error_names_the_parameter_and_writes_nothing() {
    let b = with_params("param-errors", |_| {});
    let cases: [(&[&str], &str); 6] = [
        (
            &["--param", "who=x", "--param", "colour=red"],
            "has no parameter colour",
        ),
        (
            &["--param", "who=x", "--param", "since=31/10/2026"],
            "--param since: 31/10/2026 is not a date",
        ),
        (
            &["--param", "who=x", "--param", "unit=North"],
            "--param unit: North is not in its list: East, West",
        ),
        (
            &["--param", "who=x", "--param", "units=North"],
            "--param units: North is not in its list: N, S",
        ),
        (
            &["--param", "who=x", "--param", "span=next-week"],
            "--param span: next-week is neither",
        ),
        (&[], "needs who, which has no value and no default"),
    ];
    for (args, said) in cases {
        let output = b.report(args);
        assert_eq!(code(&output), 4, "{args:?}: {}", stderr(&output));
        assert!(
            stderr(&output).contains(said),
            "{args:?}: {}",
            stderr(&output)
        );
        assert!(!b.pdf().exists());
    }
}

#[test]
fn a_period_takes_a_preset_by_name() {
    let b = with_params("preset", |_| {});
    let output = b.report(&["--param", "who=x", "--param", "span=year-to-date"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let year = cliotree_calc::today().format("%Y").to_string();
    assert!(squeezed(&b.text()).contains(&format!("{year}-01-01/")));
}

#[test]
fn list_params_prints_kinds_defaults_and_choices() {
    let b = with_params("list", |_| {});
    let output = b.run(&[
        "report",
        "{book}",
        "Summary",
        "--list-params",
        "--trust-digest",
        "{digest}",
    ]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let listed = stdout(&output);
    for line in [
        "since (Date)\n  default: 2026-01-01\n",
        "unit (Choice)\n  default: none\n  choices: East, West\n",
        "units (Choices)\n  default: none\n  choices: N (North), S (South)\n",
        "who (Text, required)\n  default: none\n",
        "big (Yes/no)\n  default: no\n",
        "span (Period)\n",
        "  presets: this-month, last-month, this-quarter, last-quarter, this-year, last-year, year-to-date\n",
    ] {
        assert!(listed.contains(line), "{line:?} in\n{listed}");
    }
    assert!(!b.pdf().exists());
}

#[test]
fn error_values_are_listed_and_the_report_is_still_written_with_code_5() {
    let b = book(
        "errors",
        vec![
            field("Fine", "1 + 1"),
            field("Broken", "1 / 0"),
            field("Missing", "Orders.nothing"),
        ],
        Vec::new(),
        |_| {},
    );
    let output = b.report(&[]);
    assert_eq!(code(&output), 5, "{}", stderr(&output));
    assert!(b.pdf().exists());
    let said = stderr(&output);
    assert!(said.contains("2 controls show error values"), "{said}");
    assert!(said.contains("Field 1 / 0: "), "{said}");
    assert!(said.contains("Field Orders.nothing: "), "{said}");
    assert!(!said.contains("1 + 1"), "{said}");
}

#[test]
fn the_timeout_stops_the_run_with_code_6_and_writes_nothing() {
    let b = book(
        "timeout",
        vec![field("Slow", "sum(i for i in iter(int, 1))")],
        Vec::new(),
        |calc| {
            let mut settings = calc.settings().clone();
            settings.set(Setting::FormulaTimeLimit, 60_000);
            calc.set_settings(settings);
        },
    );
    let output = b.report(&["--timeout", "2"]);
    assert_eq!(code(&output), 6, "{}", stderr(&output));
    assert!(stderr(&output).contains("the time ran out"));
    assert!(!b.pdf().exists());
}

#[test]
fn the_limit_of_detail_rows_is_code_6_and_writes_nothing() {
    let b = book("limit", Vec::new(), Vec::new(), |calc| {
        let spec = calc.spec("Orders").unwrap().clone();
        let path: cliotree_core::Path = "items".parse().unwrap();
        let design = cliotree_core::report::starting_design(&spec, "Orders", &path).unwrap();
        calc.create_report("Lines", design).unwrap();
        let mut settings = calc.settings().clone();
        settings.set(Setting::MaxDetailRows, 2);
        calc.set_settings(settings);
    });
    let output = b.run(&[
        "report",
        "{book}",
        "Lines",
        "--out",
        "{pdf}",
        "--trust-digest",
        "{digest}",
    ]);
    assert_eq!(code(&output), 6, "{}", stderr(&output));
    assert!(
        stderr(&output).contains("limit of 2 detail rows"),
        "{}",
        stderr(&output)
    );
    assert!(!b.pdf().exists());
}

#[test]
fn help_and_version_answer_on_stdout() {
    let b = summary("help");
    let help = b.run(&["--help"]);
    assert_eq!(code(&help), 0);
    assert!(stdout(&help).contains("cliotree-cli report <workbook.clio> <Report>"));
    let version = b.run(&["--version"]);
    assert_eq!(stdout(&version).trim(), "cliotree-cli 0.2.0");
}

// `cliotree-cli disconnect` (DESIGN.md, "Disconnecting from the command
// line"), with a Beancount journal as the data source.

const JOURNAL: &str = r#"option "operating_currency" "AUD"

2026-01-01 open Assets:Bank AUD
2026-01-01 open Revenue:Sales AUD

2026-01-05 * "Sale"
  Assets:Bank   100.00 AUD
  Revenue:Sales

2026-02-05 * "Sale"
  Assets:Bank    50.00 AUD
  Revenue:Sales
"#;

/// The balance of the bank, until the date chosen if there is one.
const UNTIL: &str = "str(source('Books').balance('Assets:Bank', until=params.until)['AUD'])";

/// A workbook whose customer is a formula reading the journal, and whose
/// Summary report shows the balance until a date parameter; the journal is
/// a data source of this user. `formula` is the customer's formula.
fn journal_book(tag: &str, formula: &str) -> Book {
    let formula = formula.to_owned();
    let b = book(
        tag,
        vec![field("Bank", UNTIL)],
        vec![ReportParam::new("until", ParamKind::Date)],
        move |calc| {
            calc.set_formula("Orders", &"customer".parse().unwrap(), &formula)
                .unwrap();
        },
    );
    let journal = b.dir.join("books.beancount");
    std::fs::write(&journal, JOURNAL).unwrap();
    SourceStore::at(b.config.join(SOURCES_FILE))
        .save(
            SourceDef {
                path: Some(journal.display().to_string()),
                ..SourceDef::new("Books", SourceKind::Beancount)
            },
            None,
            false,
        )
        .unwrap();
    b
}

const BALANCE: &str = "str(source('Books').balance('Assets:Bank')['AUD'])";

impl Book {
    fn copy(&self) -> PathBuf {
        self.dir.join("copy.clio")
    }

    /// Runs `disconnect` to `copy.clio`, trusted by its digest.
    fn disconnect(&self, more: &[&str]) -> Output {
        let copy = self.copy().display().to_string();
        let mut args = vec![
            "disconnect",
            "{book}",
            "--out",
            copy.as_str(),
            "--trust-digest",
            "{digest}",
        ];
        args.extend_from_slice(more);
        self.run(&args)
    }
}

/// The workbook in `file`, as it was saved, and its workbook id.
fn read_back(file: &std::path::Path) -> (Calc, String) {
    let package = read_zip(&std::fs::read(file).unwrap(), &Default::default()).unwrap();
    let opened = Calc::from_package(Arc::new(PythonSyntax), &package).unwrap();
    (opened.calc, opened.workbook_id)
}

#[test]
fn disconnect_writes_a_copy_without_the_reads_that_runs_without_the_source() {
    let b = journal_book("disconnect", BALANCE);
    let before = std::fs::read(&b.file).unwrap();
    let output = b.disconnect(&["--report", "Summary", "--param", "until=2026-01-31"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let (copy, id) = read_back(&b.copy());
    // The path of the copy, then its trust digest, which is new.
    let lines: Vec<String> = stdout(&output).lines().map(str::to_owned).collect();
    assert_eq!(
        lines,
        [b.copy().display().to_string(), copy.fingerprint().hash]
    );
    assert_ne!(lines[1], b.digest);
    assert_ne!(id, b.id);
    assert!(stderr(&output).contains("replaced 2 reads of data sources"));
    // The input is never changed.
    assert_eq!(std::fs::read(&b.file).unwrap(), before);
    assert_eq!(
        copy.formula_text("Orders", &"customer".parse().unwrap())
            .as_deref(),
        Some("str(_decimal('150.00'))")
    );
    // The report's read became what it gave with the value given, and its
    // run with that value is in the copy's results cache.
    let design = copy.report("Summary").unwrap();
    assert_eq!(
        design.layout.controls[0].value.as_deref(),
        Some("str(_decimal('100.00'))")
    );
    let cache = copy
        .report_cache("Summary")
        .expect("a cache for the new design");
    assert_eq!(
        cache.chosen.get("until"),
        Some(&Value::Date(
            chrono::NaiveDate::from_ymd_opt(2026, 1, 31).unwrap()
        ))
    );
    // Without the source, and trusted by the digest printed, the copy runs.
    std::fs::remove_file(b.config.join(SOURCES_FILE)).unwrap();
    let pdf = b.pdf().display().to_string();
    let copy_path = b.copy().display().to_string();
    let output = b.run(&[
        "report",
        &copy_path,
        "Summary",
        "--out",
        &pdf,
        "--trust-digest",
        &lines[1],
    ]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(squeezed(&b.text()).contains("100.00"), "{}", b.text());
}

#[test]
fn without_a_report_the_reads_take_the_values_chosen_in_the_workbook() {
    let b = journal_book("disconnect-plain", BALANCE);
    let output = b.disconnect(&[]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let (copy, _) = read_back(&b.copy());
    // No value is chosen for `until`, so the balance is the whole journal's.
    let design = copy.report("Summary").unwrap();
    assert_eq!(
        design.layout.controls[0].value.as_deref(),
        Some("str(_decimal('150.00'))")
    );
}

#[test]
fn a_use_of_a_source_that_stays_is_listed_with_code_5_and_the_copy_is_written() {
    let b = journal_book(
        "disconnect-stays",
        "str(source('Bo' + 'oks').balance('Assets:Bank')['AUD'])",
    );
    let output = b.disconnect(&[]);
    assert_eq!(code(&output), 5, "{}", stderr(&output));
    assert!(
        stderr(&output).contains("1 use still needs a data source:"),
        "{}",
        stderr(&output)
    );
    assert!(
        stderr(&output).contains("Orders.customer"),
        "{}",
        stderr(&output)
    );
    let (copy, _) = read_back(&b.copy());
    assert!(
        copy.formula_text("Orders", &"customer".parse().unwrap())
            .unwrap()
            .contains("source(")
    );
}

#[test]
fn disconnect_refuses_the_workbook_itself_an_existing_copy_and_an_untrusted_workbook() {
    let b = journal_book("disconnect-refused", BALANCE);
    let book = b.file.display().to_string();
    let same = b.run(&["disconnect", "{book}", "--out", &book, "--overwrite"]);
    assert_eq!(code(&same), 1, "{}", stderr(&same));
    assert!(
        stderr(&same).contains("the workbook itself"),
        "{}",
        stderr(&same)
    );
    std::fs::write(b.copy(), b"there").unwrap();
    let there = b.disconnect(&[]);
    assert_eq!(code(&there), 1, "{}", stderr(&there));
    assert!(stderr(&there).contains("--overwrite"), "{}", stderr(&there));
    std::fs::remove_file(b.copy()).unwrap();
    let copy = b.copy().display().to_string();
    let untrusted = b.run(&["disconnect", "{book}", "--out", &copy]);
    assert_eq!(code(&untrusted), 3, "{}", stderr(&untrusted));
    assert!(!b.copy().exists());
    let missing = b.disconnect(&["--report", "Nope"]);
    assert_eq!(code(&missing), 2, "{}", stderr(&missing));
    let param = b.disconnect(&["--report", "Summary", "--param", "until=soon"]);
    assert_eq!(code(&param), 4, "{}", stderr(&param));
    assert!(!b.copy().exists());
}
