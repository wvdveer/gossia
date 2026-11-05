//! Tests of placing the controls of a report.

use std::collections::HashMap;

use gossia_core::form::{Alignment, Control, ControlKind, GroupLayout, TrackSize};
use gossia_core::report::{Paper, ReportDesign, ReportKind, add_section};
use gossia_core::{Node, Value};

use crate::report::{
    BAND_GAP, Got, HIDDEN_BAND, Laid, Paint, Prop, Source, UNTRUSTED, bands, canvas, pages,
    single_page, to_canvas,
};

/// Values by control, items and expression; columns by control, column, row
/// and expression. What is not there is no expression.
#[derive(Default)]
struct Fake {
    controls: HashMap<(Vec<usize>, Vec<usize>, Prop), Got>,
    columns: HashMap<(Vec<usize>, String, Option<usize>, Prop), Got>,
}

impl Fake {
    fn set(mut self, at: &[usize], rows: &[usize], prop: Prop, got: Got) -> Fake {
        self.controls
            .insert((at.to_vec(), rows.to_vec(), prop), got);
        self
    }

    fn value(self, at: &[usize], rows: &[usize], prop: Prop, value: Value) -> Fake {
        self.set(at, rows, prop, Got::Value(value, false))
    }
}

impl Source for Fake {
    fn control(&self, at: &[usize], rows: &[usize], prop: Prop) -> Got {
        self.controls
            .get(&(at.to_vec(), rows.to_vec(), prop))
            .cloned()
            .unwrap_or(Got::None)
    }

    fn column(
        &self,
        at: &[usize],
        _rows: &[usize],
        column: &str,
        row: Option<usize>,
        prop: Prop,
    ) -> Got {
        self.columns
            .get(&(at.to_vec(), column.to_owned(), row, prop))
            .cloned()
            .unwrap_or(Got::None)
    }
}

fn report(controls: Vec<Control>) -> ReportDesign {
    let mut design = ReportDesign::new(ReportKind::SinglePage);
    design.layout.controls = controls;
    design
}

fn label(text: &str) -> Control {
    let mut c = Control::new(ControlKind::Label);
    c.text = Some(text.to_owned());
    c
}

fn field(label: &str) -> Control {
    let mut c = Control::new(ControlKind::Field);
    c.label = Some(label.to_owned());
    c.value = Some("x".to_owned());
    c
}

fn s(text: &str) -> Value {
    Value::Str(text.to_owned())
}

/// Where a text was painted: its left and top.
fn place(laid: &Laid, text: &str) -> (f32, f32) {
    laid.paints
        .iter()
        .find_map(|p| match p {
            Paint::Text { x, y, text: t, .. } if t == text => Some((*x, *y)),
            _ => None,
        })
        .unwrap_or_else(|| panic!("{text:?} is not painted; there is {:?}", laid.texts()))
}

#[test]
fn controls_go_one_under_the_other_and_fields_show_their_values() {
    let design = report(vec![label("Sales report"), field("Total")]);
    let fake = Fake::default().value(&[1], &[], Prop::Value, Value::Int(1234));
    let laid = single_page(&design, 600.0, &fake);
    assert_eq!(laid.texts(), ["Sales report", "Total", "1234"]);
    let (_, title_y) = place(&laid, "Sales report");
    let (label_x, label_y) = place(&laid, "Total");
    let (value_x, value_y) = place(&laid, "1234");
    assert!(label_y > title_y);
    assert_eq!(label_y, value_y);
    // A number is on the right of its box.
    assert!(value_x > label_x + 300.0, "{value_x}");
    assert!(laid.height > 0.0 && laid.width == 600.0);
    // Each control has its area, for finding it again.
    assert_eq!(laid.areas.len(), 2);
}

#[test]
fn a_control_that_is_not_visible_takes_no_room() {
    let design = report(vec![label("A"), label("B"), label("C")]);
    let shown = single_page(&design, 400.0, &Fake::default());
    let fake = Fake::default().value(&[1], &[], Prop::Visible, Value::Bool(false));
    let hidden = single_page(&design, 400.0, &fake);
    assert_eq!(hidden.texts(), ["A", "C"]);
    assert_eq!(place(&hidden, "C").1, place(&shown, "B").1);
    assert!(hidden.height < shown.height);
}

#[test]
fn errors_and_untrusted_values_are_marked_with_the_reason_on_hover() {
    let design = report(vec![field("Bad"), field("Locked")]);
    let fake = Fake::default()
        .set(
            &[0],
            &[],
            Prop::Value,
            Got::Error("ZeroDivisionError".into()),
        )
        .set(&[1], &[], Prop::Value, Got::Untrusted);
    let laid = single_page(&design, 500.0, &fake);
    assert!(laid.texts().contains(&"#ERROR!"));
    assert!(laid.texts().contains(&UNTRUSTED));
    assert!(
        laid.hovers
            .iter()
            .any(|h| h.text.contains("ZeroDivisionError"))
    );
    assert!(laid.hovers.iter().any(|h| h.text.contains("trusted")));
}

#[test]
fn a_value_waiting_to_be_calculated_has_the_pending_marker() {
    let design = report(vec![field("Total")]);
    let fake = Fake::default().set(&[0], &[], Prop::Value, Got::Value(Value::Int(3), true));
    let laid = single_page(&design, 500.0, &fake);
    assert!(
        laid.paints
            .iter()
            .any(|p| matches!(p, Paint::Pending { .. }))
    );
}

#[test]
fn a_template_shows_what_it_came_to_or_its_text_with_marks() {
    let mut t = label("{x} orders");
    t.template = true;
    let design = report(vec![t]);
    let laid = single_page(
        &design,
        400.0,
        &Fake::default().value(&[0], &[], Prop::Template, s("3 orders")),
    );
    assert_eq!(laid.texts(), ["3 orders"]);
    let locked = single_page(
        &design,
        400.0,
        &Fake::default().set(&[0], &[], Prop::Template, Got::Untrusted),
    );
    assert_eq!(locked.texts(), ["#UNTRUSTED! orders"]);
}

#[test]
fn long_text_breaks_into_lines_in_a_narrow_report() {
    let text = "a sentence long enough to need more than one line";
    let design = report(vec![label(text)]);
    let wide = single_page(&design, 2000.0, &Fake::default());
    let narrow = single_page(&design, 120.0, &Fake::default());
    assert_eq!(wide.texts().len(), 1);
    assert!(narrow.texts().len() > 2, "{:?}", narrow.texts());
    assert!(narrow.height > wide.height);
}

#[test]
fn a_group_of_two_columns_puts_controls_side_by_side() {
    let mut group = Control::new(ControlKind::Group);
    group.columns = Some(2);
    group.children = vec![label("Left"), label("Right"), label("Below")];
    let laid = single_page(&report(vec![group]), 400.0, &Fake::default());
    let (lx, ly) = place(&laid, "Left");
    let (rx, ry) = place(&laid, "Right");
    let (_, by) = place(&laid, "Below");
    assert_eq!(ly, ry);
    assert!(rx >= lx + 190.0, "{rx}");
    assert!(by > ly);
}

#[test]
fn a_group_with_a_border_and_title_pads_its_controls() {
    let mut group = Control::new(ControlKind::Group);
    group.label = Some("Totals".into());
    group.border_width = Some(1);
    group.children = vec![label("Inside")];
    let laid = single_page(&report(vec![group]), 400.0, &Fake::default());
    let (x, y) = place(&laid, "Inside");
    assert!(x >= 8.0 && y > place(&laid, "Totals").1);
    assert!(
        laid.paints
            .iter()
            .any(|p| matches!(p, Paint::Border { .. }))
    );
}

#[test]
fn a_grid_group_places_controls_in_its_cells() {
    let mut group = Control::new(ControlKind::Group);
    group.layout = Some(GroupLayout::Grid);
    group.grid_columns = vec![TrackSize::Points(100), TrackSize::Share(1)];
    let mut b = label("B");
    b.grid_row = Some(1);
    b.grid_column = Some(2);
    let mut c = label("C");
    c.grid_row = Some(2);
    c.grid_column = Some(1);
    group.children = vec![label("A"), b, c];
    let laid = single_page(&report(vec![group]), 400.0, &Fake::default());
    let (ax, ay) = place(&laid, "A");
    let (bx, by) = place(&laid, "B");
    let (cx, cy) = place(&laid, "C");
    assert_eq!(ay, by);
    assert!(bx >= ax + 100.0);
    assert_eq!(ax, cx);
    assert!(cy > ay);
}

#[test]
fn a_free_group_places_controls_where_they_were_put() {
    let mut group = Control::new(ControlKind::Group);
    group.layout = Some(GroupLayout::Free);
    let mut a = label("Here");
    a.x = Some(120);
    a.y = Some(40);
    group.children = vec![a];
    let laid = single_page(&report(vec![group]), 400.0, &Fake::default());
    assert_eq!(place(&laid, "Here"), (120.0, 40.0));
}

#[test]
fn colours_of_the_design_and_of_expressions_paint_the_control() {
    let mut fixed = label("Fixed");
    fixed.fill_colour = Some("#336699".into());
    let mut computed = label("Computed");
    computed.text_colour = Some("'red' if True else None".into());
    let fake = Fake::default().value(&[1], &[], Prop::TextColour, s("red"));
    let laid = single_page(&report(vec![fixed, computed]), 400.0, &fake);
    assert!(laid.paints.iter().any(|p| matches!(
        p,
        Paint::Fill {
            ink: crate::report::Ink::Rgba([0x33, 0x66, 0x99, 255]),
            ..
        }
    )));
    // On the dark fill the text is white.
    assert!(laid.paints.iter().any(|p| matches!(
        p,
        Paint::Text { text, ink: crate::report::Ink::Rgba([255, 255, 255, 255]), .. } if text == "Fixed"
    )));
    assert!(laid.paints.iter().any(|p| matches!(
        p,
        Paint::Text { text, ink: crate::report::Ink::Rgba([255, 0, 0, 255]), .. } if text == "Computed"
    )));
}

#[test]
fn a_label_can_be_centred() {
    let mut c = label("Middle");
    c.align = Some(Alignment::Center);
    let laid = single_page(&report(vec![c]), 400.0, &Fake::default());
    let (x, _) = place(&laid, "Middle");
    assert!(x > 150.0 && x < 200.0, "{x}");
}

// ------------------------------------------------------------ the canvas

/// A control placed on the canvas at `x`, `y`, `w` wide and `h` tall.
fn at(mut c: Control, x: u32, y: u32, w: u32, h: Option<u32>) -> Control {
    c.x = Some(x);
    c.y = Some(y);
    c.width = Some(w);
    c.height = h;
    c
}

fn on_canvas(controls: Vec<Control>) -> ReportDesign {
    let mut design = report(controls);
    design.canvas = true;
    design
}

#[test]
fn the_canvas_is_as_wide_as_the_paper_and_controls_are_at_their_places() {
    let design = on_canvas(vec![
        at(label("Left"), 0, 0, 100, Some(20)),
        at(label("Far"), 300, 200, 100, Some(20)),
    ]);
    let laid = canvas(&design, &Fake::default());
    // A4 less half an inch each side.
    assert!((laid.width - (595.28 - 72.0)).abs() < 0.01);
    assert_eq!(place(&laid, "Far"), (300.0, 200.0));
    assert_eq!(laid.height, 220.0);
    let area = laid.areas.iter().find(|a| a.at == [1]).unwrap();
    assert_eq!((area.rect.w, area.rect.h), (100.0, 20.0));
    assert_eq!(laid.insides[0].at, Vec::<usize>::new());
}

#[test]
fn the_canvas_can_be_taller_than_its_lowest_control() {
    let mut design = on_canvas(vec![at(label("Only"), 0, 0, 100, Some(20))]);
    design.height = Some(500);
    assert_eq!(canvas(&design, &Fake::default()).height, 500.0);
}

#[test]
fn controls_below_a_box_that_is_too_small_do_not_move() {
    let long = "a long text that wraps over many lines in a narrow box ".repeat(4);
    let design = on_canvas(vec![
        at(label(&long), 0, 0, 80, Some(30)),
        at(label("Below"), 0, 40, 100, Some(20)),
    ]);
    let laid = canvas(&design, &Fake::default());
    assert_eq!(place(&laid, "Below"), (0.0, 40.0));
    // What is cut ends with an ellipsis, and the box says it was cut.
    let texts = laid.texts();
    let last_of_long = texts[texts.len() - 2];
    assert!(last_of_long.ends_with('\u{2026}'), "{texts:?}");
    assert_eq!(laid.cuts.len(), 1);
    assert!(laid.hovers.iter().any(|h| h.text.starts_with("Cut")));
    // Nothing of it is drawn below its box.
    assert!(laid.paints.iter().all(|p| match p {
        Paint::Text { y, text, .. } if text != "Below" => *y < 30.0,
        _ => true,
    }));
}

#[test]
fn a_flow_design_becomes_a_canvas_with_its_controls_where_they_were() {
    let design = report(vec![label("First"), field("Total")]);
    let source = Fake::default().value(&[1], &[], Prop::Value, Value::Int(7));
    let before = single_page(&design, 523.28, &source);
    let turned = to_canvas(&design, &source, true);
    assert!(turned.canvas);
    let second = &turned.layout.controls[1];
    let area = before.areas.iter().find(|a| a.at == [1]).unwrap();
    assert_eq!(second.y, Some(area.rect.y.floor() as u32));
    assert!(second.height.unwrap() as f32 >= area.rect.h);
    // Drawn on the canvas, it shows the same.
    let after = canvas(&turned, &source);
    assert_eq!(after.texts(), before.texts());
    assert!(after.cuts.is_empty());
    // Without final results, heights are left automatic.
    assert_eq!(
        to_canvas(&design, &source, false).layout.controls[1].height,
        None
    );
}

// ------------------------------------------------------- multipage

fn multipage() -> ReportDesign {
    let mut design = ReportDesign::new(ReportKind::Multipage);
    design.page.paper = Paper::Letter;
    design
}

fn rows(n: i64) -> Value {
    Value::Array((0..n).map(|i| Node::new(Value::Int(i))).collect())
}

#[test]
fn the_designer_stacks_the_bands_with_the_first_items() {
    let mut design = multipage();
    add_section(&mut design.layout);
    design.layout.controls[1]
        .children
        .push(at(label("Heading"), 10, 4, 100, Some(20)));
    design.layout.controls[3]
        .children
        .push(at(field(""), 0, 0, 200, Some(20)));
    // The detail field shows the first detail item of the first section.
    let fake = Fake::default().value(&[3, 0], &[0, 0], Prop::Value, s("first"));
    let laid = bands(&design, &fake);
    assert_eq!(laid.bands.len(), 7);
    // The report header is hidden: a thin strip.
    assert_eq!(laid.bands[0].1.h, HIDDEN_BAND);
    let page_header = laid.bands[1].1;
    assert_eq!(page_header.y, HIDDEN_BAND + BAND_GAP);
    assert_eq!(page_header.h, 28.0);
    assert_eq!(place(&laid, "Heading"), (10.0, page_header.y + 4.0));
    assert!(laid.texts().contains(&"first"), "{:?}", laid.texts());
    // Each band is a place to drop controls, the hidden one too.
    let insides: Vec<&[usize]> = laid.insides.iter().map(|a| a.at.as_slice()).collect();
    for i in 0..7 {
        assert!(insides.contains(&&[i][..]), "{i}: {insides:?}");
    }
    assert_eq!(laid.width, design.page.content_width());
}

#[test]
fn a_multipage_report_breaks_into_pages_between_bands() {
    let mut design = multipage();
    design.layout.controls[1]
        .children
        .push(at(label("Top"), 0, 0, 100, Some(20)));
    let detail = &mut design.layout.controls[2];
    detail.height = Some(100);
    detail.children.push(at(field(""), 0, 0, 200, Some(20)));
    let mut fake = Fake::default().value(&[2], &[], Prop::Items, rows(10));
    for i in 0..10 {
        fake = fake.value(&[2, 0], &[i], Prop::Value, s(&format!("row {i}")));
    }
    // 720 points inside the margins, less 28 for each of the page header
    // and footer: six rows of 100 to a page.
    let paged = pages(&design, &fake, 100_000);
    assert_eq!((paged.width, paged.height), (612.0, 792.0));
    assert_eq!(paged.pages.len(), 2);
    assert_eq!(place(&paged.pages[0], "row 0").1, 36.0 + 28.0);
    assert_eq!(place(&paged.pages[0], "row 5").1, 36.0 + 28.0 + 500.0);
    assert_eq!(place(&paged.pages[1], "row 6").1, 36.0 + 28.0);
    assert_eq!(place(&paged.pages[1], "Top"), (36.0, 36.0));
    assert!(paged.warnings.is_empty());

    // A band taller than the page is cut, and the report says so.
    design.layout.controls[2].height = Some(700);
    let paged = pages(&design, &fake, 100_000);
    assert!(paged.warnings[0].contains("Detail"), "{:?}", paged.warnings);
    // And it stops at the limit.
    design.layout.controls[2].height = Some(100);
    let paged = pages(&design, &fake, 3);
    assert_eq!(paged.pages.len(), 1);
    assert!(paged.pages[0].texts().iter().any(|t| t.contains("limit")));
}

#[test]
fn a_band_that_is_not_visible_is_left_out_of_the_pages() {
    let mut design = multipage();
    design.layout.controls[2]
        .children
        .push(at(field(""), 0, 0, 200, Some(20)));
    let fake = Fake::default()
        .value(&[2], &[], Prop::Items, rows(3))
        .value(&[2], &[1], Prop::Visible, Value::Bool(false))
        .value(&[2, 0], &[0], Prop::Value, s("a"))
        .value(&[2, 0], &[1], Prop::Value, s("b"))
        .value(&[2, 0], &[2], Prop::Value, s("c"));
    let paged = pages(&design, &fake, 100_000);
    assert_eq!(paged.pages[0].texts(), ["a", "c"]);
    assert_eq!(place(&paged.pages[0], "c").1, 36.0 + 28.0 + 24.0);
}

#[test]
fn an_old_multipage_design_becomes_its_bands() {
    let mut design = ReportDesign::new(ReportKind::Multipage);
    design.canvas = false;
    design.layout.controls = vec![label("Hello")];
    let turned = to_canvas(&design, &Fake::default(), true);
    assert_eq!(turned.bands(), Some(0));
    let header = &turned.layout.controls[0];
    assert_eq!(header.children.len(), 1);
    assert!(header.height.unwrap() > 0);
}

#[test]
fn a_separator_is_drawn_at_its_thickness() {
    let line = |laid: &Laid| {
        laid.paints
            .iter()
            .find_map(|p| match p {
                Paint::Line { width, from, .. } => Some((*width, from.1)),
                _ => None,
            })
            .unwrap()
    };
    let mut thin = Control::new(ControlKind::Separator);
    let laid = single_page(&report(vec![thin.clone()]), 300.0, &Fake::default());
    assert_eq!(line(&laid), (0.75, 4.0));
    assert_eq!(laid.height, 8.0);
    // A thick one takes more room, with its line in the middle.
    thin.thickness = Some(6);
    let laid = single_page(&report(vec![thin]), 300.0, &Fake::default());
    assert_eq!(line(&laid), (6.0, 5.0));
    assert_eq!(laid.height, 10.0);
}

// ------------------------------------------------------------------ charts

const SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="50"><rect width="100" height="50" fill="red"/></svg>"#;

fn chart(width: u32, height: u32) -> Control {
    let mut c = Control::new(ControlKind::Chart);
    c.chart = Some("Bar".to_owned());
    c.width = Some(width);
    c.height = Some(height);
    c
}

/// The text painted for each chart, or "svg" for one drawn.
fn chart_paints(laid: &Laid) -> Vec<String> {
    laid.paints
        .iter()
        .filter_map(|p| match p {
            Paint::Svg { .. } => Some("svg".to_owned()),
            Paint::Text { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_chart_draws_its_svg_in_its_box_or_says_why_it_cannot() {
    let design = report(vec![chart(200, 100); 6]);
    let fake = Fake::default()
        .value(&[0], &[], Prop::Chart, s(SVG))
        .value(&[1], &[], Prop::Chart, Value::Int(3))
        .value(&[2], &[], Prop::Chart, s("<svg"))
        .set(
            &[3],
            &[],
            Prop::Chart,
            Got::Error("#PYERR!: boom".to_owned()),
        )
        .set(&[4], &[], Prop::Chart, Got::Untrusted);
    let laid = single_page(&design, 600.0, &fake);
    assert_eq!(
        chart_paints(&laid),
        [
            "svg",
            "#TYPE!",
            "#VALUE!",
            "#PYERR!",
            UNTRUSTED,
            "Fill in the chart's values"
        ]
    );
    let rect = laid
        .paints
        .iter()
        .find_map(|p| match p {
            Paint::Svg { rect, .. } => Some(*rect),
            _ => None,
        })
        .unwrap();
    assert_eq!((rect.w, rect.h), (200.0, 100.0));
    // Why, on hovering: the message of the error.
    assert!(laid.hovers.iter().any(|h| h.text == "#PYERR!: boom"));
}

#[test]
fn svg_over_the_limit_is_a_value_error() {
    struct Small(Fake);
    impl Source for Small {
        fn control(&self, at: &[usize], rows: &[usize], prop: Prop) -> Got {
            self.0.control(at, rows, prop)
        }
        fn column(&self, a: &[usize], r: &[usize], c: &str, w: Option<usize>, p: Prop) -> Got {
            self.0.column(a, r, c, w, p)
        }
        fn svg_limit(&self) -> usize {
            10
        }
    }
    let design = report(vec![chart(200, 100)]);
    let fake = Small(Fake::default().value(&[0], &[], Prop::Chart, s(SVG)));
    let laid = single_page(&design, 600.0, &fake);
    assert_eq!(chart_paints(&laid), ["#VALUE!"]);
    assert!(laid.hovers[0].text.contains("more than the limit of 10"));
}

#[test]
fn a_chart_style_sheet_gives_its_text_colour_and_lets_the_fill_through() {
    use crate::chart::style_sheet;
    assert_eq!(style_sheet(None, None), None);
    let css = style_sheet(
        Some(crate::report::Ink::Rgba([255, 255, 255, 255])),
        Some(crate::report::Ink::Rgba([0, 0x33, 0x66, 255])),
    )
    .unwrap();
    assert!(
        css.contains("svg { color: rgba(255, 255, 255, 1) }"),
        "{css}"
    );
    assert!(css.contains(".background { fill: none }"), "{css}");
    assert!(css.contains(".surface { fill: #003366 }"), "{css}");
    // A see-through fill as it shows on the white page.
    let css = style_sheet(None, Some(crate::report::Ink::Rgba([0, 0, 0, 0]))).unwrap();
    assert!(css.contains(".surface-stroke { stroke: #ffffff }"), "{css}");
}
