use crate::form::{Control, ControlKind};
use crate::jsonschema;
use crate::report::{
    BandKind, DETAIL_LIMIT, Orientation, Paging, Paper, ReportDesign, ReportKind, SectionOptions,
    add_section, paginate, remove_section, starting_arrays, starting_design,
};
use crate::{Path, TypeSpec};

fn orders_spec() -> TypeSpec {
    jsonschema::lower_str(
        r#"{
          "type": "object",
          "properties": {
            "customer": {"type": "string"},
            "items": {
              "type": "array",
              "items": {
                "type": "object",
                "properties": {
                  "sku": {"type": "string"},
                  "qty": {"type": "integer"},
                  "price": {"type": "object", "properties": {"amount": {"type": "number"}}}
                }
              }
            },
            "orders": {
              "type": "array",
              "items": {
                "type": "object",
                "properties": {
                  "no": {"type": "integer"},
                  "first-name": {"type": "string"},
                  "lines": {"type": "array", "items": {"type": "object", "properties": {"sku": {"type": "string"}}}}
                }
              }
            },
            "tags": {"type": "array", "items": {"type": "string"}}
          }
        }"#,
    )
    .unwrap()
}

#[test]
fn a_report_reads_back_as_it_was_written() {
    let mut design = ReportDesign::new(ReportKind::Multipage);
    design.page.paper = Paper::Letter;
    design.page.orientation = Orientation::Landscape;
    design.page.margins.left = 72;
    let mut field = Control::new(ControlKind::Field);
    field.value = Some("SUM(o.total for o in Orders.items)".into());
    design.layout.controls.push(field);
    let text = design.to_json();
    assert!(text.contains("\"kind\": \"multipage\""), "{text}");
    assert!(text.contains("\"paper\": \"letter\""), "{text}");
    assert_eq!(ReportDesign::from_json(&text).unwrap(), design);
}

#[test]
fn builtin_report_takes_its_controls_from_builtin_form() {
    let ok = r#"{"version": 1, "kind": "single_page", "layout": {"version": 1, "controls": []}}"#;
    let design = ReportDesign::from_json(ok).unwrap();
    assert_eq!(design.kind, ReportKind::SinglePage);
    assert_eq!(design.page.paper, Paper::A4);
    let bad_kind = r#"{"version": 1, "kind": "poster", "layout": {"version": 1, "controls": []}}"#;
    assert!(
        ReportDesign::from_json(bad_kind)
            .unwrap_err()
            .contains("kind")
    );
    // A control is checked as a form's is.
    let bad_font = r#"{"version": 1, "kind": "single_page", "layout": {"version": 1,
        "controls": [{"type": "label", "font": "gothic"}]}}"#;
    assert!(ReportDesign::from_json(bad_font).is_err());
    let no_layout = r#"{"version": 1, "kind": "single_page"}"#;
    assert!(ReportDesign::from_json(no_layout).is_err());
    let newer =
        r#"{"version": 9, "kind": "single_page", "layout": {"version": 1, "controls": []}}"#;
    assert!(
        ReportDesign::from_json(newer)
            .unwrap_err()
            .contains("newer")
    );
}

#[test]
fn the_page_turns_with_its_orientation() {
    let mut design = ReportDesign::new(ReportKind::SinglePage);
    assert_eq!(design.page.size(), (595.28, 841.89));
    design.page.orientation = Orientation::Landscape;
    assert_eq!(design.page.size(), (841.89, 595.28));
    assert!((design.page.content_width() - (841.89 - 72.0)).abs() < 0.01);
}

/// The fields of the detail band, as (expression, x, y).
fn detail_fields(design: &ReportDesign) -> Vec<(String, u32, u32)> {
    let detail = &design.layout.controls[BandKind::Detail.index(0)];
    detail
        .children
        .iter()
        .map(|c| {
            (
                c.value.clone().unwrap_or_default(),
                c.x.unwrap(),
                c.y.unwrap(),
            )
        })
        .collect()
}

#[test]
fn starting_from_flat_items_makes_a_detail_band_of_their_fields() {
    let spec = orders_spec();
    let design = starting_design(&spec, "Orders", &"items".parse().unwrap()).unwrap();
    assert_eq!(design.kind, ReportKind::Multipage);
    assert_eq!(design.bands(), Some(0));
    let detail = &design.layout.controls[BandKind::Detail.index(0)];
    assert_eq!(detail.items.as_deref(), Some("Orders.items"));
    assert_eq!(detail.item_name.as_deref(), Some("order"));
    let fields = detail_fields(&design);
    let values: Vec<&str> = fields.iter().map(|f| f.0.as_str()).collect();
    // None of the fields is required, so each may be absent.
    assert_eq!(
        values,
        [
            r#"order.get("sku")"#,
            r#"order.get("qty")"#,
            r#"order.get("price").get("amount")"#
        ]
    );
    // Side by side, each under its heading in the page header.
    assert_eq!(fields[0].1, 0);
    assert!(fields[1].1 > fields[0].1 && fields[1].2 == fields[0].2);
    let header = &design.layout.controls[BandKind::PageHeader.index(0)];
    let headings: Vec<&str> = header
        .children
        .iter()
        .map(|c| c.text.as_deref().unwrap())
        .collect();
    assert_eq!(headings, ["sku", "qty", "price.amount"]);
    assert_eq!(header.children[1].x, Some(fields[1].1));
    // The page footer numbers the pages.
    let footer = &design.layout.controls[BandKind::PageFooter.index(0)];
    assert!(footer.children[0].is_template());
    assert!(design.problems().is_empty(), "{:?}", design.problems());
}

#[test]
fn scalar_items_are_one_field_of_the_item() {
    let spec = orders_spec();
    let design = starting_design(&spec, "Orders", &"tags".parse().unwrap()).unwrap();
    let detail = &design.layout.controls[BandKind::Detail.index(0)];
    assert_eq!(detail.item_name.as_deref(), Some("tag"));
    assert_eq!(detail_fields(&design)[0].0, "tag");
}

#[test]
fn items_that_hold_arrays_start_with_their_scalars() {
    let spec = orders_spec();
    let design = starting_design(&spec, "Shop", &"orders".parse().unwrap()).unwrap();
    let detail = &design.layout.controls[BandKind::Detail.index(0)];
    assert_eq!(detail.items.as_deref(), Some("Shop.orders"));
    let values: Vec<String> = detail_fields(&design).into_iter().map(|f| f.0).collect();
    assert_eq!(values, [r#"order.get("no")"#, r#"order.get("first-name")"#]);
}

#[test]
fn only_arrays_can_be_started_from() {
    let spec = orders_spec();
    let arrays: Vec<String> = starting_arrays(&spec)
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(arrays, ["items", "orders", "tags"]);
    assert!(starting_design(&spec, "Orders", &"customer".parse().unwrap()).is_none());
    assert!(starting_design(&spec, "Orders", &Path::root()).is_none());
}

#[test]
fn a_multipage_report_starts_as_its_stack_of_bands() {
    let design = ReportDesign::new(ReportKind::Multipage);
    assert_eq!(design.bands(), Some(0));
    let names: Vec<String> = (0..design.layout.controls.len())
        .map(|i| design.band_name(i))
        .collect();
    assert_eq!(
        names,
        [
            "Report header",
            "Page header",
            "Detail",
            "Page footer",
            "Report footer"
        ]
    );
    // The report header and footer are optional, so they start hidden.
    assert_eq!(design.layout.controls[0].height, Some(0));
    assert!(design.layout.controls[2].height.unwrap() > 0);
    assert!(design.problems().is_empty());
    assert_eq!(ReportDesign::new(ReportKind::SinglePage).bands(), None);
}

#[test]
fn sections_go_around_the_detail_band() {
    let mut design = ReportDesign::new(ReportKind::Multipage);
    assert_eq!(add_section(&mut design.layout), Some(0));
    design.layout.controls[2].item_name = Some("unit".into());
    assert_eq!(add_section(&mut design.layout), Some(1));
    design.layout.controls[3].item_name = Some("month".into());
    assert_eq!(design.bands(), Some(2));
    let names: Vec<String> = (0..design.layout.controls.len())
        .map(|i| design.band_name(i))
        .collect();
    assert_eq!(
        names,
        [
            "Report header",
            "Page header",
            "Section header: unit",
            "Section header: month",
            "Detail",
            "Section footer: month",
            "Section footer: unit",
            "Page footer",
            "Report footer"
        ]
    );
    for (i, kind) in BandKind::stack(2).into_iter().enumerate() {
        assert_eq!(kind.index(2), i, "{kind:?}");
    }
    // A section has to have items.
    let problems = design.problems();
    assert_eq!(problems.len(), 2, "{problems:?}");
    assert_eq!(problems[0].0, [2]);
    assert!(problems[0].1.contains("items"));
    assert!(remove_section(&mut design.layout, 0));
    assert_eq!(design.bands(), Some(1));
    assert_eq!(design.band_name(2), "Section header: month");
    assert!(!remove_section(&mut design.layout, 1));
}

#[test]
fn a_multipage_report_without_its_bands_says_so() {
    let mut design = ReportDesign::new(ReportKind::Multipage);
    design
        .layout
        .controls
        .push(Control::new(ControlKind::Label));
    assert_eq!(design.bands(), None);
    assert!(design.problems()[0].1.contains("bands"));
}

// ------------------------------------------------------------ paging

/// A report on a page 100 points tall inside its margins: page header 10,
/// page footer 10, so 80 for the body, with one section of `header` and
/// `footer` points and a detail band of `detail` points.
fn paged(header: u32, detail: u32, footer: u32) -> ReportDesign {
    let mut design = ReportDesign::new(ReportKind::Multipage);
    add_section(&mut design.layout);
    design.page.paper = Paper::Letter;
    design.page.margins = crate::report::Margins {
        top: 0,
        right: 0,
        bottom: (design.page.size().1 - 100.0).round() as u32,
        left: 0,
    };
    let heights = [0, 10, header, detail, footer, 10, 0];
    for (band, h) in design.layout.controls.iter_mut().zip(heights) {
        band.height = Some(h);
    }
    design
}

fn every(_: usize, _: &[usize]) -> bool {
    true
}

/// Each page as (band, rows) of what is in its body.
fn bodies(paging: &Paging) -> Vec<Vec<(usize, Vec<usize>)>> {
    paging
        .pages
        .iter()
        .map(|page| {
            page.iter()
                .filter(|p| p.band != 1 && p.band != 5)
                .map(|p| (p.band, p.rows.clone()))
                .collect()
        })
        .collect()
}

#[test]
fn bands_fill_a_page_and_go_on_to_the_next() {
    let design = paged(20, 20, 0);
    // Two groups of three items.
    let items = |level: usize, _: &[usize]| if level == 0 { 2 } else { 3 };
    let paging = paginate(&design, &items, &every, DETAIL_LIMIT);
    assert_eq!(
        bodies(&paging),
        [
            vec![
                (2, vec![0]),
                (3, vec![0, 0]),
                (3, vec![0, 1]),
                (3, vec![0, 2])
            ],
            vec![
                (2, vec![1]),
                (3, vec![1, 0]),
                (3, vec![1, 1]),
                (3, vec![1, 2])
            ],
        ]
    );
    // Every page has its header and footer, with its index.
    let page_2 = &paging.pages[1];
    assert_eq!((page_2[0].band, page_2[0].rows.clone()), (1, vec![1]));
    let last = page_2.last().unwrap();
    assert_eq!((last.band, last.rows.clone(), last.y), (5, vec![1], 90.0));
    assert_eq!(page_2[1].y, 10.0);
    assert!(paging.cut.is_empty() && !paging.limited);
}

#[test]
fn a_section_can_start_on_a_new_page_and_keep_its_header() {
    let mut design = paged(20, 20, 0);
    let items = |level: usize, _: &[usize]| if level == 0 { 3 } else { 1 };
    // Without options: 40 points each, two to a page.
    let plain = paginate(&design, &items, &every, DETAIL_LIMIT);
    assert_eq!(plain.pages.len(), 2);
    design.sections = vec![SectionOptions {
        new_page: true,
        ..SectionOptions::default()
    }];
    let paging = paginate(&design, &items, &every, DETAIL_LIMIT);
    assert_eq!(paging.pages.len(), 3);
    assert_eq!(bodies(&paging)[1][0], (2, vec![1]));

    // A header with 20 points left goes on with its first detail band.
    let mut design = paged(20, 20, 0);
    let items = |level: usize, _: &[usize]| if level == 0 { 2 } else { 1 };
    design.layout.controls[2].height = Some(30);
    // 30 + 20, then 30 more would leave the header alone at the bottom.
    let plain = paginate(&design, &items, &every, DETAIL_LIMIT);
    assert_eq!(bodies(&plain)[0].len(), 3);
    design.sections = vec![SectionOptions {
        keep_with_first: true,
        ..SectionOptions::default()
    }];
    let kept = paginate(&design, &items, &every, DETAIL_LIMIT);
    assert_eq!(bodies(&kept)[0].len(), 2);
    assert_eq!(bodies(&kept)[1][0], (2, vec![1]));
}

#[test]
fn a_header_can_repeat_on_the_pages_its_section_goes_on_to() {
    let mut design = paged(20, 20, 0);
    let items = |level: usize, _: &[usize]| if level == 0 { 1 } else { 6 };
    design.sections = vec![SectionOptions {
        repeat_header: true,
        ..SectionOptions::default()
    }];
    let paging = paginate(&design, &items, &every, DETAIL_LIMIT);
    let pages = bodies(&paging);
    assert_eq!(pages.len(), 2);
    assert_eq!(pages[1][0], (2, vec![0]));
    assert_eq!(pages[1][1], (3, vec![0, 3]));
}

#[test]
fn hidden_bands_take_no_room_and_tall_ones_are_cut() {
    let design = paged(20, 20, 0);
    let items = |level: usize, _: &[usize]| if level == 0 { 1 } else { 4 };
    // The second and third detail bands are not visible.
    let visible = |band: usize, rows: &[usize]| !(band == 3 && (rows[1] == 1 || rows[1] == 2));
    let paging = paginate(&design, &items, &visible, DETAIL_LIMIT);
    assert_eq!(
        bodies(&paging),
        [vec![(2, vec![0]), (3, vec![0, 0]), (3, vec![0, 3])]]
    );

    let tall = paged(20, 120, 0);
    let items = |_: usize, _: &[usize]| 1;
    let paging = paginate(&tall, &items, &every, DETAIL_LIMIT);
    assert_eq!(paging.cut, [3]);
    // It starts a page of its own and is cut at the page footer.
    let detail = paging.pages[1].iter().find(|p| p.band == 3).unwrap();
    assert_eq!((detail.y, detail.height), (10.0, 80.0));
}

#[test]
fn the_report_header_comes_first_and_the_footer_last() {
    let mut design = paged(0, 20, 0);
    design.layout.controls[0].height = Some(30);
    design.layout.controls[6].height = Some(10);
    let items = |level: usize, _: &[usize]| if level == 0 { 1 } else { 2 };
    let paging = paginate(&design, &items, &every, DETAIL_LIMIT);
    let first: Vec<(usize, f32)> = paging.pages[0].iter().map(|p| (p.band, p.y)).collect();
    // Report header, page header, two details, report footer, page footer.
    assert_eq!(
        first,
        [
            (0, 0.0),
            (1, 30.0),
            (3, 40.0),
            (3, 60.0),
            (6, 80.0),
            (5, 90.0)
        ]
    );
}

#[test]
fn a_report_stops_at_the_limit_of_detail_bands() {
    let design = paged(0, 10, 0);
    let items = |level: usize, _: &[usize]| if level == 0 { 3 } else { 4 };
    let paging = paginate(&design, &items, &every, 5);
    assert!(paging.limited);
    let details = paging
        .pages
        .iter()
        .flatten()
        .filter(|p| p.band == 3)
        .count();
    assert_eq!(details, 5);
}

#[test]
fn a_report_lists_what_only_a_form_can_have() {
    let mut design = ReportDesign::new(ReportKind::SinglePage);
    design.layout.controls.push(Control::bound(
        ControlKind::Field,
        &"customer".parse().unwrap(),
    ));
    design
        .layout
        .controls
        .push(Control::new(ControlKind::Button));
    let mut field = Control::new(ControlKind::Field);
    field.value = Some("1".into());
    field.enabled_when = Some("True".into());
    design.layout.controls.push(field);
    let mut label = Control::new(ControlKind::Label);
    label.text = Some("Fine".into());
    design.layout.controls.push(label);
    let mut repeater = Control::new(ControlKind::Repeater);
    repeater.items = Some("Orders.items".into());
    design.layout.controls.push(repeater);
    let problems = design.problems();
    let at: Vec<&[usize]> = problems.iter().map(|(a, _)| a.as_slice()).collect();
    assert_eq!(at, [&[0][..], &[1], &[2], &[4]]);
    assert!(problems[3].1.contains("detail band"), "{}", problems[3].1);
    assert!(problems[0].1.contains("path"), "{}", problems[0].1);
    assert!(problems[1].1.contains("buttons"));
    assert!(problems[2].1.contains("enabled when"));
}
