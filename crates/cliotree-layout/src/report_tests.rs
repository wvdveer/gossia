//! Tests of placing the controls of a report.

use std::collections::HashMap;

use cliotree_core::form::{
    Alignment, ColumnDesign, Control, ControlKind, GroupLayout, RepeaterMode, TrackSize,
};
use cliotree_core::report::{ReportDesign, ReportKind};
use cliotree_core::{Node, Value};

use crate::report::{Got, Laid, Paint, Prop, Source, UNTRUSTED, canvas, single_page, to_canvas};

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

    fn column(
        mut self,
        at: &[usize],
        column: &str,
        row: Option<usize>,
        prop: Prop,
        got: Got,
    ) -> Fake {
        self.columns
            .insert((at.to_vec(), column.to_owned(), row, prop), got);
        self
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

fn list(values: Vec<Value>) -> Value {
    Value::Array(values.into_iter().map(Node::new).collect())
}

fn record(fields: &[(&str, Value)]) -> Value {
    Value::Object(
        fields
            .iter()
            .map(|(k, v)| ((*k).to_owned(), Node::new(v.clone())))
            .collect(),
    )
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
fn a_table_has_a_title_row_and_a_row_for_each_item() {
    let mut table = Control::new(ControlKind::Repeater);
    table.mode = Some(RepeaterMode::Table);
    table.label = Some("Lines".into());
    let mut qty = ColumnDesign::named("qty");
    qty.label = Some("Quantity".into());
    let mut twice = ColumnDesign::named("twice");
    twice.value = Some("this.qty * 2".into());
    table.table_columns = vec![ColumnDesign::named("sku"), qty, twice];
    let items = list(vec![
        record(&[("sku", s("a")), ("qty", Value::Int(2))]),
        record(&[("sku", s("b")), ("qty", Value::Int(5))]),
    ]);
    let fake = Fake::default()
        .value(&[0], &[], Prop::Items, items)
        .column(
            &[0],
            "twice",
            Some(0),
            Prop::Value,
            Got::Value(Value::Int(4), false),
        )
        .column(
            &[0],
            "twice",
            Some(1),
            Prop::Value,
            Got::Value(Value::Int(10), false),
        );
    let laid = single_page(&report(vec![table]), 600.0, &fake);
    assert_eq!(
        laid.texts(),
        [
            "Lines", "sku", "Quantity", "twice", "a", "2", "4", "b", "5", "10"
        ]
    );
    // The rows go down, and the cells of a row line up.
    assert!(place(&laid, "b").1 > place(&laid, "a").1);
    assert_eq!(place(&laid, "a").1, place(&laid, "2").1);
}

#[test]
fn a_table_with_no_columns_shows_the_fields_of_its_first_item() {
    let mut table = Control::new(ControlKind::Repeater);
    table.mode = Some(RepeaterMode::Table);
    let items = list(vec![record(&[("name", s("Ada")), ("age", Value::Int(36))])]);
    let fake = Fake::default().value(&[0], &[], Prop::Items, items);
    let laid = single_page(&report(vec![table]), 600.0, &fake);
    assert_eq!(laid.texts(), ["name", "age", "Ada", "36"]);
}

#[test]
fn a_wide_table_is_shrunk_to_fit() {
    let mut table = Control::new(ControlKind::Repeater);
    table.mode = Some(RepeaterMode::Table);
    let long = "a long text in a cell ".repeat(10);
    let items = list(vec![record(&[("text", s(&long)), ("n", Value::Int(1))])]);
    let fake = Fake::default().value(&[0], &[], Prop::Items, items);
    let laid = single_page(&report(vec![table]), 300.0, &fake);
    let right = laid
        .paints
        .iter()
        .filter_map(|p| match p {
            Paint::Line { to, .. } => Some(to.0),
            _ => None,
        })
        .fold(0.0, f32::max);
    assert!(right <= 300.5, "{right}");
}

#[test]
fn designed_items_are_each_shown_one_under_the_other() {
    let mut stacked = Control::new(ControlKind::Repeater);
    stacked.mode = Some(RepeaterMode::Stacked);
    stacked.children = vec![field("Order")];
    let fake = Fake::default()
        .value(
            &[0],
            &[],
            Prop::Items,
            list(vec![Value::Int(1), Value::Int(2)]),
        )
        .value(&[0, 0], &[0], Prop::Value, s("first"))
        .value(&[0, 0], &[1], Prop::Value, s("second"));
    let laid = single_page(&report(vec![stacked]), 500.0, &fake);
    assert_eq!(laid.texts(), ["Order", "first", "Order", "second"]);
    assert!(place(&laid, "second").1 > place(&laid, "first").1);
    assert!(laid.areas.iter().any(|a| a.at == [0, 0] && a.rows == [1]));
}

#[test]
fn items_that_are_not_a_list_say_so() {
    let mut stacked = Control::new(ControlKind::Repeater);
    stacked.mode = Some(RepeaterMode::Stacked);
    let fake = Fake::default().value(&[0], &[], Prop::Items, Value::Int(3));
    let laid = single_page(&report(vec![stacked]), 500.0, &fake);
    assert!(
        laid.texts().iter().any(|t| t.contains("not a list")),
        "{:?}",
        laid.texts()
    );
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
fn a_table_in_a_box_shows_the_rows_that_fit_and_how_many_more() {
    let mut table = Control::new(ControlKind::Repeater);
    table.items = Some("items".into());
    let items: Vec<Value> = (0..30).map(|i| record(&[("n", Value::Int(i))])).collect();
    let design = on_canvas(vec![at(table, 0, 0, 200, Some(100))]);
    let laid = canvas(
        &design,
        &Fake::default().value(&[0], &[], Prop::Items, list(items)),
    );
    let texts = laid.texts();
    assert!(
        texts
            .iter()
            .any(|t| t.starts_with("and ") && t.ends_with(" more rows")),
        "{texts:?}"
    );
    let shown = texts.iter().filter(|t| t.parse::<i64>().is_ok()).count();
    assert!(shown > 0 && shown < 30);
    assert!(texts.contains(&format!("and {} more rows", 30 - shown).as_str()));
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
