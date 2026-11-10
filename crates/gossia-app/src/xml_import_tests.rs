//! Dropping XML documents that use several namespaces (DESIGN.md, XML Schemas
//! and namespaces, 0.5.0, step 15d-2): one schema tab per namespace, added
//! with the data tab, and a namespace's tab reused when the data fits it.

use std::path::PathBuf;
use std::time::Duration;

use gossia_calc::ExportFormat;
use gossia_core::dataformat::DataFormat;
use gossia_core::validate::validate;

use crate::GossiaApp;
use crate::import::{ImportPlan, Inferred, SchemaChoice};

const CORPUS: &[&str] = &[
    "sitemap.xml",
    "atom.xml",
    "kml.xml",
    "project.csproj",
    "docbook.xml",
    "page.xhtml",
    "pom.xml",
    "track.gpx",
    "model.archimate",
    "settings.plist",
    "document.xml",
    "feed.rss",
    "AndroidManifest.xml",
    "envelope.soap.xml",
    "MainWindow.xaml",
    "drawing.svg",
];

fn corpus(name: &str) -> PathBuf {
    [
        env!("CARGO_MANIFEST_DIR"),
        "..",
        "gossia-core",
        "tests",
        "xml-corpus",
        name,
    ]
    .iter()
    .collect()
}

fn blank() -> GossiaApp {
    let mut app = GossiaApp::new(crate::workbook_file::blank_workbook());
    app.files.browse = false;
    app
}

fn settle(app: &mut GossiaApp) {
    assert!(app.session.wait_idle(Duration::from_secs(60)));
}

/// Drops the file on the window, as dragging it there does, and takes the
/// import dialog it opens.
fn dropped(app: &mut GossiaApp, path: PathBuf) -> ImportPlan {
    app.drop_files(vec![path]);
    assert!(app.notice.is_none(), "{:?}", app.notice);
    app.imports.pop_front().expect("an import dialog")
}

fn other_tabs(plan: &ImportPlan) -> Vec<(String, bool)> {
    match &plan.inferred {
        Ok(Inferred::Xsd(x)) => x
            .others
            .iter()
            .map(|o| (o.name.clone(), o.existing))
            .collect(),
        other => panic!("{other:?}"),
    }
}

fn exported(app: &GossiaApp, tab: &str, pretty: bool) -> String {
    app.calc()
        .export_text(tab, ExportFormat::Data(DataFormat::Xml), pretty)
        .unwrap()
}

#[test]
fn every_document_of_the_corpus_drops_with_a_schema_tab_per_namespace() {
    for name in CORPUS {
        let mut app = blank();
        let path = corpus(name);
        let text = std::fs::read_to_string(&path)
            .unwrap()
            .replace("\r\n", "\n");
        let plan = dropped(&mut app, path.clone());
        assert_eq!(plan.choice, SchemaChoice::Inferred, "{name}");
        let others = other_tabs(&plan);
        assert!(others.iter().all(|(_, existing)| !existing), "{name}");
        let before = app.calc().tab_names().len();
        app.import(&plan).unwrap_or_else(|e| panic!("{name}: {e}"));
        settle(&mut app);
        let calc = app.calc();
        assert_eq!(
            calc.tab_names().len(),
            before + 2 + others.len(),
            "{name}: {:?}",
            calc.tab_names()
        );
        for tab in [plan.schema_name.as_str()]
            .into_iter()
            .chain(others.iter().map(|(t, _)| t.as_str()))
        {
            assert_eq!(calc.schema_error(tab), None, "{name}: {tab}");
        }
        let data_tab = plan.name.as_str();
        let spec = calc.spec(data_tab).unwrap();
        let issues = validate(spec, calc.data(data_tab).unwrap());
        assert!(issues.is_empty(), "{name}: {issues:?}");
        assert_eq!(
            exported(&app, data_tab, true).trim_end(),
            text.trim_end(),
            "{name}: written back differently"
        );
        // The same document again fits the schema tab it made.
        let again = dropped(&mut app, path);
        assert_eq!(
            again.choice,
            SchemaChoice::Existing(plan.schema_name.clone()),
            "{name}"
        );
    }
}

#[test]
fn the_rss_feed_adds_one_tab_per_namespace_and_undoes_as_one() {
    let mut app = blank();
    let plan = dropped(&mut app, corpus("feed.rss"));
    assert_eq!(plan.name, "feed");
    assert_eq!(plan.schema_name, "feedSchema");
    let mut names: Vec<String> = other_tabs(&plan).into_iter().map(|(n, _)| n).collect();
    names.sort();
    assert_eq!(names, ["feed_atom", "feed_content", "feed_dc"]);
    let before = app.calc().tab_names();
    app.import(&plan).unwrap();
    settle(&mut app);
    assert_eq!(
        app.calc().xsd_namespace("feed_dc").as_deref(),
        Some("http://purl.org/dc/elements/1.1/")
    );
    app.undo();
    settle(&mut app);
    assert_eq!(app.calc().tab_names(), before);
}

#[test]
fn a_namespace_with_a_tab_that_fits_uses_it() {
    let mut app = blank();
    let first = r#"<book xmlns:dc="urn:dc"><dc:creator>Ann</dc:creator></book>"#;
    let plan = app.plan_import("book.xml", first.as_bytes()).unwrap();
    assert_eq!(other_tabs(&plan), [("book_dc".to_owned(), false)]);
    app.import(&plan).unwrap();
    settle(&mut app);
    // Another document with dc:creator in it: book_dc fits it as it is.
    let second = r#"<song xmlns:dc="urn:dc"><dc:creator>Bo</dc:creator><len>3</len></song>"#;
    let plan = app.plan_import("song.xml", second.as_bytes()).unwrap();
    assert_eq!(plan.choice, SchemaChoice::Inferred);
    assert_eq!(other_tabs(&plan), [("book_dc".to_owned(), true)]);
    let before = app.calc().tab_names().len();
    app.import(&plan).unwrap();
    settle(&mut app);
    assert_eq!(
        app.calc().tab_names().len(),
        before + 2,
        "only song and its schema"
    );
    assert_eq!(app.calc().xsd_importers("book_dc").len(), 2);
    assert_eq!(exported(&app, "song", false).trim_end(), second);
    // A dc:creator with an attribute does not fit it, and gets a tab of its
    // own.
    let third = r#"<film xmlns:dc="urn:dc"><dc:creator role="x">Cy</dc:creator></film>"#;
    let plan = app.plan_import("film.xml", third.as_bytes()).unwrap();
    assert_eq!(other_tabs(&plan), [("film_dc".to_owned(), false)]);
}

#[test]
fn the_tabs_of_other_namespaces_need_names_of_their_own() {
    let mut app = blank();
    let text = r#"<book xmlns:dc="urn:dc"><dc:creator>Ann</dc:creator></book>"#;
    let mut plan = app.plan_import("book.xml", text.as_bytes()).unwrap();
    plan.name = "book_dc".into();
    let why = app.import_problem(&plan).unwrap();
    assert!(why.contains("book_dc"), "{why}");
    // A main schema renamed in the dialog is what the others import.
    plan.name = "Books".into();
    plan.schema_name = "BookTypes".into();
    assert_eq!(app.import_problem(&plan), None);
    app.import(&plan).unwrap();
    settle(&mut app);
    assert_eq!(app.calc().schema_error("BookTypes"), None);
    assert_eq!(app.calc().xsd_importers("book_dc"), ["BookTypes"]);
}
