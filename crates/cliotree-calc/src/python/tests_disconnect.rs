//! Tests of Disconnect from Data Sources (DESIGN.md, "Data sources",
//! "Disconnect from Data Sources"), against the real interpreter and a SQLite
//! source.

use std::time::Duration;

use cliotree_core::form::{Control, ControlKind, FormDesign};
use cliotree_core::{Path, Value};

use super::tests::{Book, p};
use super::{PythonEngine, PythonSettings};
use crate::Disconnect;
use crate::sources::SourceStore;

fn book(name: &str) -> (Book, std::path::PathBuf) {
    let (dir, db) = crate::sqlite::sqlite_tests::shop(name);
    let store = SourceStore::in_memory();
    store
        .save(crate::sqlite::sqlite_tests::def(&db, false), None, false)
        .unwrap();
    let mut b = Book::new();
    b.engine = PythonEngine::with_settings(PythonSettings {
        sources: store,
        source_timeout: Duration::from_secs(5),
        ..PythonSettings::default()
    });
    (b, dir)
}

/// Plans, runs the probes and reads them back.
fn disconnect(b: &mut Book) -> Disconnect {
    b.run();
    let plan = b.calc.plan_disconnect().unwrap();
    b.run();
    b.calc.finish_disconnect(&plan).expect("the probes ran")
}

fn text(b: &Book, path: &str) -> String {
    b.calc.formula_text("S", &p(path)).unwrap()
}

#[test]
fn reads_in_formulas_become_literals_of_what_they_gave() {
    let (mut b, dir) = book("disconnect");
    b.formula("a", "len(source('Shop').query('SELECT * FROM orders'))");
    b.formula(
        "name",
        "source('Shop').query('SELECT customer FROM orders WHERE id = ?', [2])[0]['customer']",
    );
    // Dates, times, bytes and None, which have to be read back the same.
    b.formula(
        "c",
        "sum(r['placed'].year + r['shipped'].hour + len(r['stamp']) \
         for r in source('Shop').query('SELECT * FROM orders WHERE id = 1'))",
    );
    b.formula(
        "tags",
        "[str(v) for v in source('Shop').query(\
         'SELECT placed, shipped, stamp, note FROM orders WHERE id = 1')[0].values()]",
    );
    // What stays: a write, a read that uses a name of a comprehension, and
    // a source whose name is not written out.
    b.formula("note", "source('Shop').execute('DELETE FROM orders')");
    b.formula(
        "nums",
        "[source('Shop').query('SELECT ? AS v', [i])[0]['v'] for i in range(2)]",
    );
    b.formula("text", "str(source(S.name).query('SELECT 1'))");
    b.run();
    let before: Vec<Option<Value>> = ["a", "name", "c", "tags"].map(|f| b.get(f)).to_vec();

    let out = disconnect(&mut b);
    assert_eq!(out.replaced, 4, "{out:#?}");
    assert_eq!(out.formulas(), 4);
    assert_eq!(out.problems.len(), 3, "{:#?}", out.problems);
    assert!(out.problems[0].contains("S.note"), "{:#?}", out.problems);
    assert!(
        out.problems[0].contains("execute: only reads"),
        "{:#?}",
        out.problems
    );
    assert!(out.problems[1].contains("S.nums"), "{:#?}", out.problems);
    assert!(out.problems[2].contains("S.text"), "{:#?}", out.problems);
    // Nothing changes until it is applied.
    assert!(text(&b, "name").contains("source"));

    b.calc.apply_disconnect(&out);
    b.run();
    assert_eq!(text(&b, "name"), "'Bob'");
    assert!(text(&b, "a").starts_with("len(["), "{}", text(&b, "a"));
    assert!(
        text(&b, "c").contains("_date('2026-10-01')"),
        "{}",
        text(&b, "c")
    );
    assert!(text(&b, "c").contains("_datetime('2026-10-02T09:30:00')"));
    for f in ["a", "name", "c", "tags"] {
        assert!(!text(&b, f).contains("source"), "{f}: {}", text(&b, f));
    }
    let after: Vec<Option<Value>> = ["a", "name", "c", "tags"].map(|f| b.get(f)).to_vec();
    assert_eq!(after, before);
    assert!(text(&b, "note").contains("source"));

    // One step of the history puts them all back.
    assert_eq!(b.calc.undo_label(), Some("Disconnect from data sources"));
    b.calc.undo().unwrap();
    assert!(text(&b, "name").contains("source"));
    assert!(text(&b, "c").contains("source"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_column_formula_is_replaced_once_when_every_item_gave_the_same() {
    let (mut b, dir) = book("disconnect-column");
    b.calc
        .set_column_formula(
            "S",
            &p("items"),
            "tag",
            "source('Shop').query('SELECT customer FROM orders WHERE id = 1')[0]['customer']",
        )
        .unwrap();
    b.calc
        .set_column_formula(
            "S",
            &p("items"),
            "total",
            "source('Shop').query('SELECT total FROM orders WHERE id = ?', [parent.qty - 1])\
             [0]['total']",
        )
        .unwrap();
    let out = disconnect(&mut b);
    assert_eq!(out.replaced, 1, "{out:#?}");
    assert_eq!(out.problems.len(), 1, "{:#?}", out.problems);
    assert!(
        out.problems[0].contains("items[].total") && out.problems[0].contains("different"),
        "{:#?}",
        out.problems
    );
    b.calc.apply_disconnect(&out);
    b.run();
    assert_eq!(
        b.calc.column_formula("S", &p("items"), "tag"),
        Some("'Ada'")
    );
    assert_eq!(text(&b, "items[2].tag"), "'Ada'");
    assert_eq!(b.get("items[1].tag"), Some(Value::Str("Ada".into())));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn expressions_of_forms_and_template_labels_are_replaced() {
    let (mut b, dir) = book("disconnect-forms");
    let mut field = Control::bound(ControlKind::Field, &Path::root());
    field.path = None;
    field.value = Some(
        "source('Shop').query('SELECT customer FROM orders WHERE id = 2')[0]['customer']".into(),
    );
    field.visible_when = Some("len(source('Shop').query('SELECT id FROM orders')) > 1".into());
    let mut label = Control::bound(ControlKind::Label, &Path::root());
    label.path = None;
    label.template = true;
    label.text = Some(
        "Top {{order}}: {source('Shop').query('SELECT MAX(total) AS m FROM orders')[0]['m']:.1f}"
            .into(),
    );
    b.calc.forms_mut().set(
        "S",
        "Entry",
        FormDesign {
            controls: vec![field, label],
            ..FormDesign::default()
        },
    );
    let out = disconnect(&mut b);
    assert_eq!(out.problems, Vec::<String>::new());
    assert_eq!(out.replaced, 3);
    assert_eq!(out.expressions(), 3);
    b.calc.apply_disconnect(&out);
    let design = b.calc.forms().get("S", "Entry").unwrap();
    assert_eq!(design.controls[0].value.as_deref(), Some("'Bob'"));
    assert!(
        design.controls[0]
            .visible_when
            .as_deref()
            .unwrap()
            .starts_with("len([")
    );
    assert_eq!(
        design.controls[1].text.as_deref(),
        Some("Top {{order}}: {12.5:.1f}")
    );
    b.run();
    assert_eq!(
        b.calc
            .control_expression("S", "Entry", &[1], crate::ExprKind::Template)
            .and_then(|s| s.result),
        Some(Ok(Value::Str("Top {order}: 12.5".into())))
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn literals_name_the_helpers_formulas_see() {
    let mut b = Book::new();
    b.formula(
        "text",
        "_as_literal([_date('2026-01-02'), _datetime('2026-01-02T03:04:05'), \
         _decimal('1.50'), _record({'a': None}), (1,), b'x', float('nan')])",
    );
    b.formula("note", "_as_literal(D)");
    b.run();
    assert_eq!(
        b.get("text"),
        Some(Value::Str(
            "[_date('2026-01-02'), _datetime('2026-01-02T03:04:05'), _decimal('1.50'), \
             _record({'a': None}), (1,), b'x', float('nan')]"
                .into()
        ))
    );
    assert!(
        b.get("note")
            .is_some_and(|v| matches!(v, Value::Str(t) if t.starts_with('{')))
    );
}

impl crate::DisconnectRunner for Book {
    fn calc_mut(&mut self) -> &mut crate::Calc {
        &mut self.calc
    }

    fn settle(&mut self) -> bool {
        self.run();
        true
    }
}

#[test]
fn reads_in_reports_are_replaced_with_the_parameters_chosen_now() {
    use cliotree_core::report::{ParamKind, ReportDesign, ReportKind, ReportParam};

    let (mut b, dir) = book("disconnect-reports");
    let mut name = Control::new(ControlKind::Field);
    name.value = Some(
        "source('Shop').query('SELECT customer FROM orders WHERE id = ?', [params.id])[0]\
         ['customer']"
            .into(),
    );
    let mut chart = Control::new(ControlKind::Chart);
    chart.chart = Some("Bar".into());
    chart.chart_args.insert(
        "labels".into(),
        "[r['customer'] for r in source('Shop').query('SELECT customer FROM orders')]".into(),
    );
    chart.chart_args.insert(
        "values".into(),
        "[r['total'] for r in source('Shop').query('SELECT total FROM orders')]".into(),
    );
    let mut id = ReportParam::new("id", ParamKind::Number);
    id.default = Some("1".into());
    let mut who = ReportParam::new("who", ParamKind::Choice);
    who.choices =
        Some("[r['customer'] for r in source('Shop').query('SELECT customer FROM orders')]".into());
    let mut design = ReportDesign::new(ReportKind::SinglePage);
    design.layout.controls = vec![name, chart];
    design.params = vec![id, who];
    b.calc.create_report("Sales", design).unwrap();
    b.calc.set_report_param("Sales", "id", Some(Value::Int(2)));
    // Not shown: the report is shown while its reads are found, then not.
    let out = crate::find_disconnect(&mut b).unwrap();
    assert_eq!(b.calc.shown_report(), None);
    assert_eq!(out.problems, Vec::<String>::new());
    assert_eq!(out.replaced, 4);
    assert_eq!((out.formulas(), out.expressions()), (0, 0));
    assert_eq!(out.report_expressions(), 4);
    assert_eq!(out.with_params, ["Sales"]);
    assert_eq!(out.reports(), ["Sales"]);
    assert!(crate::apply_and_rerun(&mut b, &out, None));
    assert_eq!(b.calc.shown_report(), None);
    let design = b.calc.report("Sales").unwrap();
    assert_eq!(design.layout.controls[0].value.as_deref(), Some("'Bob'"));
    for arg in ["labels", "values"] {
        assert!(
            design.layout.controls[1].chart_args[arg].starts_with("[r["),
            "{arg}"
        );
        assert!(
            !design.layout.controls[1].chart_args[arg].contains("source("),
            "{arg}"
        );
    }
    assert!(
        !design.params[1]
            .choices
            .as_deref()
            .unwrap()
            .contains("source(")
    );
    // The results cache is the run with these values, for the new design.
    let cache = b
        .calc
        .report_cache("Sales")
        .expect("a cache for the new design");
    assert_eq!(cache.chosen.get("id"), Some(&Value::Int(2)));
    assert_eq!(
        cache
            .results
            .get(&(vec![0], None, vec![], crate::ExprKind::Value)),
        Some(&Ok(Value::Str("Bob".into())))
    );
    let _ = std::fs::remove_dir_all(&dir);
}
