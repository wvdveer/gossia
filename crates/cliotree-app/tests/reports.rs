//! Reports in the window (DESIGN.md, "Reports"): a report tab draws its
//! controls from the results of its expressions, which run while it is
//! shown and follow the data.

use std::time::Duration;

use cliotree_app::ClioApp;
use cliotree_app::control;
use cliotree_core::form::{Control, ControlKind};
use cliotree_core::report::{BandKind, ReportDesign, ReportKind};
use cliotree_core::{Path, Value};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

fn window() -> Harness<'static, ClioApp> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 700.0))
        .build_ui_state(|ui, app: &mut ClioApp| app.show(ui), ClioApp::sample());
    settle(&mut harness);
    harness
}

/// Draws until nothing is waiting: a frame shows the report, the evaluation
/// runs its expressions, and the next frame draws what they came to.
fn settle(harness: &mut Harness<'_, ClioApp>) {
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

/// The text the report tab drew.
fn texts(harness: &Harness<'_, ClioApp>) -> Vec<String> {
    harness
        .state()
        .report
        .laid
        .as_ref()
        .map(|l| l.texts().into_iter().map(str::to_owned).collect())
        .unwrap_or_default()
}

fn new_report(harness: &mut Harness<'_, ClioApp>, args: serde_json::Value) {
    control::run(harness.state_mut(), "new_tab", &args).unwrap();
    settle(harness);
}

fn p(text: &str) -> Path {
    text.parse().unwrap()
}

#[test]
fn a_report_follows_the_data() {
    let mut harness = window();
    report_with_field(&mut harness, "First", "Orders.items[0].sku");
    assert_eq!(texts(&harness), ["First", "A-100"]);
    harness
        .state_mut()
        .session
        .calc_mut()
        .set_value("Orders", &p("items[0].sku"), Value::Str("Z-999".into()))
        .unwrap();
    settle(&mut harness);
    assert_eq!(texts(&harness), ["First", "Z-999"]);
}

#[test]
fn the_palette_of_a_report_has_no_repeater_and_a_repeater_is_a_problem() {
    let mut harness = window();
    report_with_field(&mut harness, "Total", "Orders.total");
    click(&mut harness, "Edit report");
    assert!(harness.query_by_label("Add repeater").is_none());
    let mut design = harness.state().calc().report("Summary").unwrap().clone();
    design
        .layout
        .controls
        .push(Control::new(ControlKind::Repeater));
    let problems = design.problems();
    assert!(problems[0].1.contains("detail band"), "{problems:?}");
}

// ------------------------------------------------------- multipage

/// A multipage report of the items of Orders, started from the array.
fn lines_report(harness: &mut Harness<'_, ClioApp>) {
    new_report(
        harness,
        json!({"name": "Lines", "report": true, "start_tab": "Orders", "start_path": "items"}),
    );
    settle(harness);
}

/// The text of the page of a multipage report being shown.
fn page_texts(harness: &Harness<'_, ClioApp>) -> Vec<String> {
    let app = harness.state();
    app.report
        .paged
        .as_ref()
        .and_then(|p| p.pages.get(app.report.page))
        .map(|l| l.texts().into_iter().map(str::to_owned).collect())
        .unwrap_or_default()
}

fn lines(harness: &Harness<'_, ClioApp>) -> ReportDesign {
    harness.state().calc().report("Lines").unwrap().clone()
}

fn set_lines(harness: &mut Harness<'_, ClioApp>, design: ReportDesign) {
    harness
        .state_mut()
        .session
        .calc_mut()
        .set_report("Lines", design)
        .unwrap();
    settle(harness);
}

#[test]
fn a_report_started_from_an_array_shows_each_item_page_by_page() {
    let mut harness = window();
    lines_report(&mut harness);
    let design = lines(&harness);
    assert_eq!(design.kind, ReportKind::Multipage);
    assert_eq!(design.bands(), Some(0));
    let texts = page_texts(&harness);
    for expected in [
        "sku",
        "qty",
        "A-100",
        "B-220",
        "C-310",
        "gift wrap",
        "Page 1 of 1",
    ] {
        assert!(texts.iter().any(|t| t == expected), "{expected}: {texts:?}");
    }
    assert!(harness.query_by_label("Page 1 of 1").is_some());
}

#[test]
fn the_pages_turn_with_the_buttons_and_the_page_keys() {
    let mut harness = window();
    lines_report(&mut harness);
    // Two rows of 300 points to a page: the third item is on page 2.
    let mut design = lines(&harness);
    design.layout.controls[BandKind::Detail.index(0)].height = Some(300);
    set_lines(&mut harness, design);
    assert!(harness.query_by_label("Page 1 of 2").is_some());
    assert!(!page_texts(&harness).iter().any(|t| t == "C-310"));
    click(&mut harness, "Next page");
    harness.run();
    assert_eq!(harness.state().report.page, 1);
    let texts = page_texts(&harness);
    assert!(texts.iter().any(|t| t == "C-310"), "{texts:?}");
    assert!(texts.iter().any(|t| t == "Page 2 of 2"), "{texts:?}");
    harness.key_press(egui::Key::PageUp);
    harness.run();
    assert_eq!(harness.state().report.page, 0);
}

#[test]
fn edit_report_shows_the_stack_of_bands_and_a_band_is_resized_by_its_line() {
    let mut harness = window();
    lines_report(&mut harness);
    click(&mut harness, "Edit report");
    harness.state_mut().report.zoom = Some(1.0);
    harness.run();
    let laid = harness.state().report.laid.clone().unwrap();
    assert_eq!(laid.bands.len(), 5);
    // The detail band shows the first item.
    assert!(laid.texts().contains(&"A-100"), "{:?}", laid.texts());
    let detail = BandKind::Detail.index(0);
    let band = laid.bands[detail].1;
    let before = lines(&harness).layout.controls[detail].height.unwrap();
    let from = on_screen(&harness, 300.0, band.y + band.h + 1.0);
    let to = on_screen(&harness, 300.0, band.y + band.h + 41.0);
    drag(&mut harness, from, to);
    let after = lines(&harness).layout.controls[detail].height.unwrap();
    // Forty points down, snapped to the 4-point grid.
    assert_eq!(after, (before + 40).div_ceil(4) * 4);
    click(&mut harness, "Undo layout");
    assert_eq!(
        lines(&harness).layout.controls[detail].height.unwrap(),
        before
    );
}

#[test]
fn the_palette_adds_to_the_detail_band_and_a_band_is_not_deleted() {
    let mut harness = window();
    lines_report(&mut harness);
    click(&mut harness, "Edit report");
    let detail = BandKind::Detail.index(0);
    let before = lines(&harness).layout.controls[detail].clone();
    click(&mut harness, "Add label");
    let after = lines(&harness).layout.controls[detail].clone();
    assert_eq!(after.children.len(), before.children.len() + 1);
    // The band grew to hold it.
    let added = after.children.last().unwrap();
    assert!(after.height.unwrap() >= added.y.unwrap() + added.height.unwrap());
    assert_eq!(
        harness.state().selected_control(),
        Some(vec![detail, after.children.len() - 1])
    );
    // A band selected is not deleted.
    harness.state_mut().select_control(Some(vec![detail]));
    harness.run();
    harness.key_press(egui::Key::Delete);
    harness.run();
    assert_eq!(lines(&harness).bands(), Some(0));
}

#[test]
fn a_control_is_dragged_from_one_band_to_another() {
    let mut harness = window();
    lines_report(&mut harness);
    click(&mut harness, "Edit report");
    harness.state_mut().report.zoom = Some(1.0);
    harness.run();
    let laid = harness.state().report.laid.clone().unwrap();
    let detail = BandKind::Detail.index(0);
    let header = BandKind::PageHeader.index(0);
    let (from_band, to_band) = (laid.bands[detail].1, laid.bands[header].1);
    let headings = lines(&harness).layout.controls[header].children.len();
    let from = on_screen(&harness, 10.0, from_band.y + 6.0);
    let to = on_screen(&harness, 210.0, to_band.y + 2.0);
    drag(&mut harness, from, to);
    let design = lines(&harness);
    assert_eq!(design.layout.controls[header].children.len(), headings + 1);
    let moved = design.layout.controls[header].children.last().unwrap();
    assert_eq!(moved.value.as_deref(), Some("order.sku"));
    assert!(moved.x.unwrap() >= 196);
}

#[test]
fn the_property_page_sets_up_the_page_and_adds_sections() {
    let mut harness = window();
    lines_report(&mut harness);
    click(&mut harness, "Edit report");
    harness.state_mut().select_control(None);
    harness.run();
    assert!(harness.query_by_label("Top margin").is_some());
    click(&mut harness, "Add section");
    settle(&mut harness);
    let design = lines(&harness);
    assert_eq!(design.bands(), Some(1));
    // Its header band is selected, and says it needs items.
    assert_eq!(harness.state().selected_control(), Some(vec![2]));
    assert!(design.problems()[0].1.contains("items"));
    click(&mut harness, "Undo layout");
    assert_eq!(lines(&harness).bands(), Some(0));
}

#[test]
fn a_section_groups_the_detail_bands_and_its_footer_totals_them() {
    let mut harness = window();
    lines_report(&mut harness);
    let mut design = lines(&harness);
    cliotree_core::report::add_section(&mut design.layout);
    // A section for the item with a note (B-220) and those without.
    let header = &mut design.layout.controls[2];
    header.items = Some("[True, False]".into());
    header.item_name = Some("noted".into());
    let mut name = Control::new(ControlKind::Field);
    name.value = Some("'With a note' if noted else 'Without'".into());
    name.width = Some(200);
    name.height = Some(20);
    header.children.push(name);
    let detail = &mut design.layout.controls[3];
    detail.items = Some("[o for o in Orders.items if (o.sku == 'B-220') == noted]".into());
    detail.list_name = Some("rows".into());
    let mut total = Control::new(ControlKind::Field);
    total.value = Some("f'{len(rows)} lines'".into());
    total.width = Some(200);
    total.height = Some(20);
    let footer = &mut design.layout.controls[4];
    footer.height = Some(24);
    footer.children.push(total);
    set_lines(&mut harness, design);
    settle(&mut harness);
    let texts = page_texts(&harness);
    let at = |t: &str| {
        texts
            .iter()
            .position(|x| x == t)
            .unwrap_or_else(|| panic!("{t}: {texts:?}"))
    };
    assert!(at("With a note") < at("B-220"));
    assert!(at("B-220") < at("1 lines"));
    assert!(at("1 lines") < at("Without"));
    assert!(at("Without") < at("A-100"));
    assert!(at("C-310") < at("2 lines"));
}

/// A report with one field showing `expression`, labelled `label`.
fn report_with_field(harness: &mut Harness<'_, ClioApp>, label: &str, expression: &str) {
    new_report(harness, json!({"name": "Summary", "report": true}));
    let mut design = ReportDesign::new(Default::default());
    let mut field = Control::new(ControlKind::Field);
    field.label = Some(label.to_owned());
    field.value = Some(expression.to_owned());
    design.layout.controls.push(field);
    harness
        .state_mut()
        .session
        .calc_mut()
        .set_report("Summary", design)
        .unwrap();
    settle(harness);
}

#[test]
fn a_field_shows_what_its_expression_comes_to() {
    let mut harness = window();
    report_with_field(&mut harness, "Total", "Orders.total");
    assert_eq!(texts(&harness), ["Total", "62.92"]);
}

#[test]
fn a_report_runs_only_while_its_tab_is_shown() {
    let mut harness = window();
    report_with_field(&mut harness, "Total", "Orders.total");
    assert_eq!(harness.state().calc().shown_report(), Some("Summary"));
    harness.state_mut().switch_to("Orders");
    settle(&mut harness);
    assert_eq!(harness.state().calc().shown_report(), None);
}

#[test]
fn an_untrusted_workbook_shows_the_mark_instead_of_running_the_report() {
    let mut harness = window();
    harness.state_mut().session.calc_mut().set_trusted(false);
    report_with_field(&mut harness, "Total", "Orders.total");
    assert_eq!(texts(&harness), ["Total", "#UNTRUSTED!"]);
}

#[test]
fn an_error_in_an_expression_shows_in_its_control_and_the_others_still_show() {
    let mut harness = window();
    report_with_field(&mut harness, "Broken", "1 / 0");
    let mut design = harness.state().calc().report("Summary").unwrap().clone();
    let mut fine = Control::new(ControlKind::Field);
    fine.label = Some("Fine".into());
    fine.value = Some("2 + 2".into());
    design.layout.controls.push(fine);
    harness
        .state_mut()
        .session
        .calc_mut()
        .set_report("Summary", design)
        .unwrap();
    settle(&mut harness);
    assert_eq!(texts(&harness), ["Broken", "#ERROR!", "Fine", "4"]);
    let laid = harness.state().report.laid.clone().unwrap();
    assert!(laid.hovers.iter().any(|h| h.text.contains("ZeroDivision")));
}

fn click(harness: &mut Harness<'_, ClioApp>, label: &str) {
    harness.get_by_label(label).click();
    harness.run();
}

#[test]
fn edit_report_is_layout_mode_with_a_palette_without_buttons() {
    let mut harness = window();
    report_with_field(&mut harness, "Total", "Orders.total");
    click(&mut harness, "Edit report");
    assert!(harness.state().layout_active());
    assert!(harness.query_by_label("Add field").is_some());
    assert!(harness.query_by_label("Add button").is_none());
    // The canvas is drawn as the report is, with its value.
    assert_eq!(texts(&harness), ["Total", "62.92"]);
    assert!(harness.state().report.canvas_at.is_some());
    click(&mut harness, "Edit report");
    assert!(!harness.state().layout_active());
    assert_eq!(texts(&harness), ["Total", "62.92"]);
}

#[test]
fn a_control_added_in_layout_mode_is_in_the_report_and_undo_layout_takes_it_back() {
    let mut harness = window();
    report_with_field(&mut harness, "Total", "Orders.total");
    let undo_before = harness.state().calc().undo_label().map(str::to_owned);
    click(&mut harness, "Edit report");
    click(&mut harness, "Add label");
    let controls = |h: &Harness<'_, ClioApp>| {
        h.state()
            .calc()
            .report("Summary")
            .unwrap()
            .layout
            .controls
            .len()
    };
    assert_eq!(controls(&harness), 2);
    // Not an edit of the workbook.
    assert_eq!(
        harness.state().calc().undo_label().map(str::to_owned),
        undo_before
    );
    click(&mut harness, "Undo layout");
    assert_eq!(controls(&harness), 1);
    click(&mut harness, "Redo layout");
    assert_eq!(controls(&harness), 2);
}

#[test]
fn the_property_page_of_a_report_field_has_its_value_and_nothing_for_editing() {
    let mut harness = window();
    report_with_field(&mut harness, "Total", "Orders.total");
    click(&mut harness, "Edit report");
    harness.state_mut().select_control(Some(vec![0]));
    settle(&mut harness);
    assert!(harness.query_all_by_label("Value").next().is_some());
    assert!(harness.query_all_by_label("Prefix").next().is_some());
    for gone in [
        "Source of the field",
        "Editor of the field",
        "Write procedure of the field",
        "Help",
        "Enabled when",
        "Choices",
    ] {
        assert!(
            harness.query_all_by_label(gone).next().is_none(),
            "{gone} is shown"
        );
    }
}

#[test]
fn edit_report_on_the_tab_menu_shows_the_report_in_layout_mode() {
    let mut harness = window();
    report_with_field(&mut harness, "Total", "Orders.total");
    harness.state_mut().switch_to("Orders");
    settle(&mut harness);
    harness.get_by_label("Summary").click_secondary();
    harness.run();
    click(&mut harness, "Edit report");
    assert_eq!(harness.state().sheet, "Summary");
    assert!(harness.state().layout_active());
}

// ------------------------------------------------------------ the canvas

fn drag(harness: &mut Harness<'_, ClioApp>, from: egui::Pos2, to: egui::Pos2) {
    harness.hover_at(from);
    harness.run();
    harness.drag_at(from);
    harness.run();
    for step in 1..=5 {
        harness.hover_at(from.lerp(to, step as f32 / 5.0));
        harness.run();
    }
    harness.event(egui::Event::PointerButton {
        pos: to,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness.run();
    harness.remove_cursor();
    harness.run();
}

fn click_at(harness: &mut Harness<'_, ClioApp>, pos: egui::Pos2) {
    harness.hover_at(pos);
    harness.run();
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
    }
}

/// Where a point of the canvas, in report points, is on the screen.
fn on_screen(harness: &Harness<'_, ClioApp>, x: f32, y: f32) -> egui::Pos2 {
    let (origin, zoom) = harness.state().report.canvas_at.unwrap();
    origin + egui::vec2(x * zoom, y * zoom)
}

fn first_control(harness: &Harness<'_, ClioApp>) -> Control {
    harness
        .state()
        .calc()
        .report("Summary")
        .unwrap()
        .layout
        .controls[0]
        .clone()
}

#[test]
fn a_report_made_before_the_canvas_is_turned_into_one_when_shown() {
    let mut harness = window();
    report_with_field(&mut harness, "Total", "Orders.total");
    let design = harness.state().calc().report("Summary").unwrap().clone();
    assert!(design.canvas);
    let field = &design.layout.controls[0];
    assert_eq!((field.x, field.y), (Some(0), Some(0)));
    assert!(field.width.is_some() && field.height.is_some());
    assert_eq!(texts(&harness), ["Total", "62.92"]);
}

#[test]
fn a_control_is_moved_by_dragging_it_on_the_canvas_and_undo_layout_puts_it_back() {
    let mut harness = window();
    report_with_field(&mut harness, "Total", "Orders.total");
    click(&mut harness, "Edit report");
    harness.state_mut().report.zoom = Some(1.0);
    harness.run();
    let from = on_screen(&harness, 20.0, 8.0);
    let to = on_screen(&harness, 120.0, 88.0);
    drag(&mut harness, from, to);
    let moved = first_control(&harness);
    assert_eq!((moved.x, moved.y), (Some(100), Some(80)));
    assert_eq!(harness.state().selected_control(), Some(vec![0]));
    click(&mut harness, "Undo layout");
    let back = first_control(&harness);
    assert_eq!((back.x, back.y), (Some(0), Some(0)));
}

#[test]
fn a_handle_resizes_the_selected_control_to_the_grid() {
    let mut harness = window();
    report_with_field(&mut harness, "Total", "Orders.total");
    click(&mut harness, "Edit report");
    harness.state_mut().report.zoom = Some(1.0);
    harness.state_mut().select_control(Some(vec![0]));
    harness.run();
    let c = first_control(&harness);
    let (w, h) = (c.width.unwrap() as f32, c.height.unwrap() as f32);
    // The corner at the bottom right, dragged 41 points right and 30 down.
    let from = on_screen(&harness, w, h);
    let to = on_screen(&harness, w + 41.0, h + 30.0);
    drag(&mut harness, from, to);
    let resized = first_control(&harness);
    assert_eq!(resized.width.unwrap() % 4, 0);
    assert!(resized.width.unwrap() as f32 >= w + 38.0);
    assert!(resized.height.unwrap() as f32 >= h + 28.0);
}

#[test]
fn the_palette_adds_a_control_with_a_box_under_the_lowest_one() {
    let mut harness = window();
    report_with_field(&mut harness, "Total", "Orders.total");
    click(&mut harness, "Edit report");
    click(&mut harness, "Add label");
    let design = harness.state().calc().report("Summary").unwrap().clone();
    let (first, added) = (&design.layout.controls[0], &design.layout.controls[1]);
    assert!(added.y.unwrap() >= first.y.unwrap() + first.height.unwrap());
    assert!(added.width.is_some() && added.height.is_some());
    assert_eq!(harness.state().selected_control(), Some(vec![1]));
    // Delete takes the selected control away.
    harness.key_press(egui::Key::Delete);
    harness.run();
    let count = harness
        .state()
        .calc()
        .report("Summary")
        .unwrap()
        .layout
        .controls
        .len();
    assert_eq!(count, 1);
}

#[test]
fn a_click_on_the_canvas_selects_and_on_the_empty_page_selects_nothing() {
    let mut harness = window();
    report_with_field(&mut harness, "Total", "Orders.total");
    click(&mut harness, "Edit report");
    harness.state_mut().report.zoom = Some(1.0);
    harness.run();
    let on = on_screen(&harness, 10.0, 8.0);
    click_at(&mut harness, on);
    harness.run();
    assert_eq!(harness.state().selected_control(), Some(vec![0]));
    let off = on_screen(&harness, 300.0, 400.0);
    let mut design = harness.state().calc().report("Summary").unwrap().clone();
    design.height = Some(600);
    harness
        .state_mut()
        .session
        .calc_mut()
        .set_report("Summary", design)
        .unwrap();
    harness.run();
    click_at(&mut harness, off);
    harness.run();
    assert_eq!(harness.state().selected_control(), None);
}

#[test]
fn show_expressions_shows_each_field_s_expression() {
    let mut harness = window();
    report_with_field(&mut harness, "Total", "Orders.total");
    click(&mut harness, "Edit report");
    click(&mut harness, "Show expressions");
    assert_eq!(texts(&harness), ["Total", "[Orders.total]"]);
    click(&mut harness, "Show expressions");
    assert_eq!(texts(&harness), ["Total", "62.92"]);
}
