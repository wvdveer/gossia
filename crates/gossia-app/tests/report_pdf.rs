//! *Export as PDF...* (DESIGN.md, Reports, "PDF export", and Testing,
//! "Reports"): the dialog, the file, and the PDF's pages holding the lines
//! the page view shows.

use std::path::{Path, PathBuf};
use std::time::Duration;

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use gossia_app::GossiaApp;
use gossia_app::control;
use gossia_core::form::{Control, ControlKind};
use gossia_core::report::{ParamKind, ReportDesign, ReportParam};
use serde_json::json;

fn window() -> Harness<'static, GossiaApp> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 700.0))
        .with_max_steps(8)
        .build_ui_state(|ui, app: &mut GossiaApp| app.show(ui), GossiaApp::sample());
    harness.state_mut().files.browse = false;
    settle(&mut harness);
    harness
}

fn settle(harness: &mut Harness<'_, GossiaApp>) {
    for _ in 0..3 {
        harness.run();
        assert!(
            harness
                .state_mut()
                .session
                .wait_idle(Duration::from_secs(60)),
            "the workbook did not settle"
        );
    }
    harness.run();
}

fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("gossia-pdf-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn has(harness: &Harness<'_, GossiaApp>, label: &str) -> bool {
    harness.query_all_by_label_contains(label).next().is_some()
}

/// The report with a field for each expression.
fn report(harness: &mut Harness<'_, GossiaApp>, fields: &[(&str, &str)], params: Vec<ReportParam>) {
    control::run(
        harness.state_mut(),
        "new_tab",
        &json!({"name": "Summary", "report": true}),
    )
    .unwrap();
    settle(harness);
    let mut design = ReportDesign::new(Default::default());
    for (label, value) in fields {
        let mut field = Control::new(ControlKind::Field);
        field.label = Some((*label).to_owned());
        field.value = Some((*value).to_owned());
        design.layout.controls.push(field);
    }
    design.params = params;
    harness
        .state_mut()
        .session
        .calc_mut()
        .set_report("Summary", design)
        .unwrap();
    settle(harness);
}

/// Opens the dialog for `tab`, chooses `path` and waits for the PDF.
fn export(harness: &mut Harness<'_, GossiaApp>, tab: &str, path: &Path) {
    harness.state_mut().begin_export_pdf(tab);
    harness.run();
    harness.get_by_label("Export PDF").click();
    harness.run();
    harness.get_by_label("File name").click();
    harness.run();
    harness.key_combination_modifiers(egui::Modifiers::COMMAND, &[egui::Key::A]);
    harness
        .get_by_label("File name")
        .type_text(&path.display().to_string());
    harness.run();
    harness.get_by_label("Export").click();
    settle(harness);
}

/// The words of each page of a PDF.
fn pdf_words(path: &Path) -> Vec<Vec<String>> {
    let bytes = std::fs::read(path).unwrap();
    assert!(bytes.starts_with(b"%PDF-"));
    pdf_extract::extract_text_from_mem_by_pages(&bytes)
        .unwrap()
        .iter()
        .map(|p| p.split_whitespace().map(str::to_owned).collect())
        .collect()
}

#[test]
fn export_as_pdf_writes_the_report_as_it_is_shown() {
    let mut harness = window();
    report(&mut harness, &[("Total", "Orders.total")], Vec::new());
    let dir = folder("single");
    // Without an extension, it is given .pdf.
    export(&mut harness, "Summary", &dir.join("summary"));
    assert!(
        harness.state().pdf_export.is_none(),
        "the dialog closes: {:?} {:?}",
        harness.state().pdf_export,
        harness.state().files.dialog
    );
    assert_eq!(
        pdf_words(&dir.join("summary.pdf")),
        [vec!["Total", "62.92"]]
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_pdf_of_a_multipage_report_has_the_lines_of_each_page() {
    let mut harness = window();
    control::run(
        harness.state_mut(),
        "new_tab",
        &json!({"name": "Lines", "report": true, "start_tab": "Orders", "start_path": "items"}),
    )
    .unwrap();
    settle(&mut harness);
    let dir = folder("pages");
    let path = dir.join("lines.pdf");
    export(&mut harness, "Lines", &path);
    let paged = harness.state_mut().lay_out_pages().clone();
    let shown: Vec<Vec<String>> = paged
        .pages
        .iter()
        .map(|page| {
            page.texts()
                .iter()
                .flat_map(|t| t.split_whitespace().map(str::to_owned))
                .collect()
        })
        .collect();
    assert!(!shown.is_empty());
    assert_eq!(pdf_words(&path), shown);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_untrusted_workbook_exports_what_its_report_shows_and_the_dialog_says_so() {
    let mut harness = window();
    harness.state_mut().session.calc_mut().set_trusted(false);
    report(&mut harness, &[("Total", "Orders.total")], Vec::new());
    harness.state_mut().begin_export_pdf("Summary");
    harness.run();
    assert!(has(&harness, "This workbook is not trusted"));
    harness.state_mut().pdf_export = None;
    harness.run();
    let dir = folder("untrusted");
    let path = dir.join("summary.pdf");
    export(&mut harness, "Summary", &path);
    assert_eq!(pdf_words(&path), [vec!["Total", "#UNTRUSTED!"]]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_required_parameter_left_empty_stops_the_export() {
    let mut harness = window();
    let mut sku = ReportParam::new("sku", ParamKind::Text);
    sku.required = true;
    report(&mut harness, &[("Product", "params.sku")], vec![sku]);
    harness.state_mut().begin_export_pdf("Summary");
    harness.run();
    assert!(has(&harness, "Fill in"));
    assert!(
        harness
            .get_by_label("Export PDF")
            .accesskit_node()
            .is_disabled()
    );
    let dir = folder("missing");
    let path = dir.join("summary.pdf");
    let error = harness.state_mut().export_report_to(&path).unwrap_err();
    assert!(error.starts_with("Fill in"), "{error}");
    assert!(!path.exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_report_tab_menu_offers_export_as_pdf() {
    let mut harness = window();
    report(&mut harness, &[("Total", "Orders.total")], Vec::new());
    harness.state_mut().switch_to("Orders");
    settle(&mut harness);
    harness.state_mut().begin_export_pdf("Summary");
    harness.run();
    // The report is shown, so that it is calculated.
    assert_eq!(harness.state().sheet, "Summary");
    assert!(has(&harness, "Export Summary as PDF"));
}

#[test]
fn the_control_channel_exports_a_report_as_pdf() {
    let mut harness = window();
    report(&mut harness, &[("Total", "Orders.total")], Vec::new());
    harness.state_mut().switch_to("Orders");
    settle(&mut harness);
    let dir = folder("control");
    let path = dir.join("summary.pdf");
    control::run(
        harness.state_mut(),
        "export_pdf",
        &json!({"report": "Summary", "path": path.display().to_string()}),
    )
    .unwrap();
    assert_eq!(pdf_words(&path), [vec!["Total", "62.92"]]);
    let error = control::run(
        harness.state_mut(),
        "export_pdf",
        &json!({"report": "Nope", "path": path.display().to_string()}),
    )
    .unwrap_err();
    assert!(error.contains("no report called Nope"), "{error}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_file_menu_exports_the_report_shown_as_pdf() {
    let mut harness = window();
    report(&mut harness, &[("Total", "Orders.total")], Vec::new());
    harness.get_by_label("File").click();
    harness.run();
    harness.get_by_label("Export as PDF...").click();
    harness.run();
    assert!(has(&harness, "Export Summary as PDF"));
    // On a data tab it is greyed.
    harness.state_mut().pdf_export = None;
    harness.state_mut().switch_to("Orders");
    settle(&mut harness);
    harness.get_by_label("File").click();
    harness.run();
    assert!(
        harness
            .get_by_label("Export as PDF...")
            .accesskit_node()
            .is_disabled()
    );
}

#[test]
fn export_as_html_writes_the_report_as_one_page_with_its_text() {
    let mut harness = window();
    report(&mut harness, &[("Total", "Orders.total")], Vec::new());
    let dir = folder("html");
    harness.state_mut().begin_export_html("Summary");
    harness.run();
    assert!(has(&harness, "Export Summary as HTML"));
    harness.get_by_label("Export HTML").click();
    harness.run();
    harness.get_by_label("File name").click();
    harness.run();
    harness.key_combination_modifiers(egui::Modifiers::COMMAND, &[egui::Key::A]);
    // Without an extension, it is given .html.
    harness
        .get_by_label("File name")
        .type_text(&dir.join("summary").display().to_string());
    harness.run();
    harness.get_by_label("Export").click();
    settle(&mut harness);
    assert!(harness.state().pdf_export.is_none(), "the dialog closes");
    let html = std::fs::read_to_string(dir.join("summary.html")).unwrap();
    assert!(html.starts_with("<!DOCTYPE html>"));
    assert!(html.contains("<title>Summary</title>"));
    assert!(html.contains(">Total</text>"));
    assert!(html.contains(">62.92</text>"));
    assert!(!html.contains("<script"));
    // The control command writes it too.
    let path = dir.join("again.html");
    control::run(
        harness.state_mut(),
        "export_html",
        &json!({"report": "Summary", "path": path.display().to_string()}),
    )
    .unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), html);
}
