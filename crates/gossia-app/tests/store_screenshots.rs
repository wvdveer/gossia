//! Screenshots for the Microsoft Store listing, which is kept with them
//! outside the repository, since the repository holds no binary files.
//! Rendered as the snapshot tests are, with wgpu, at 1920 x 1080 pixels (a
//! 1280 x 720 window at 1.5 pixels per point), and written to the folder
//! named by `GOSSIA_STORE_SCREENSHOTS`; without it the test does nothing:
//!
//! ```text
//! GOSSIA_STORE_SCREENSHOTS=C:\projects\rust\gossia-store cargo test -p gossia-app --test store_screenshots
//! ```

use std::path::PathBuf;
use std::time::Duration;

use egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use gossia_app::GossiaApp;
use gossia_core::form::{Control, ControlKind};

fn folder() -> Option<PathBuf> {
    let dir = std::env::var_os("GOSSIA_STORE_SCREENSHOTS").filter(|v| !v.is_empty())?;
    let dir = PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    Some(dir)
}

fn window(app: GossiaApp) -> Harness<'static, GossiaApp> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 720.0))
        .with_pixels_per_point(1.5)
        .wgpu()
        .build_ui_state(|ui, app: &mut GossiaApp| app.show(ui), app);
    settle(&mut harness);
    harness
}

fn settle(harness: &mut Harness<'_, GossiaApp>) {
    for _ in 0..3 {
        assert!(
            harness
                .state_mut()
                .session
                .wait_idle(Duration::from_secs(60))
        );
        harness.run();
    }
}

fn click(harness: &mut Harness<'_, GossiaApp>, label: &str) {
    harness.get_by_label(label).click();
    harness.run();
}

fn show_as(harness: &mut Harness<'_, GossiaApp>, view: &str) {
    click(harness, "View");
    click(harness, "Show as");
    click(harness, view);
}

/// No blinking cursor or focus ring in the picture.
fn unfocus(harness: &mut Harness<'_, GossiaApp>) {
    harness.ctx.memory_mut(|m| {
        if let Some(id) = m.focused() {
            m.surrender_focus(id);
        }
    });
    harness.run();
}

fn save(harness: &mut Harness<'_, GossiaApp>, dir: &std::path::Path, name: &str) {
    unfocus(harness);
    shoot(harness, dir, name);
}

/// The picture as it is, the focus kept (for a popup that needs it), without
/// the mouse pointer.
fn shoot(harness: &mut Harness<'_, GossiaApp>, dir: &std::path::Path, name: &str) {
    harness.event(egui::Event::PointerGone);
    harness.run();
    let image = harness.render().expect("a graphics adapter renders it");
    assert_eq!((image.width(), image.height()), (1920, 1080));
    image.save(dir.join(format!("{name}.png"))).unwrap();
}

/// The light or the dark theme, for the next pictures.
fn theme(harness: &mut Harness<'_, GossiaApp>, light: bool) {
    harness.ctx.set_theme(if light {
        egui::Theme::Light
    } else {
        egui::Theme::Dark
    });
    harness.run();
}

fn chart(name: &str, args: &[(&str, &str)], at: [u32; 4]) -> Control {
    let mut c = Control::new(ControlKind::Chart);
    c.chart = Some(name.into());
    for (k, v) in args {
        c.chart_args.insert((*k).into(), (*v).into());
    }
    let [x, y, w, h] = at;
    c.x = Some(x);
    c.y = Some(y);
    c.width = Some(w);
    c.height = Some(h);
    c
}

/// A report tab of charts on the sample's orders, and a Sankey diagram.
fn with_reports(harness: &mut Harness<'_, GossiaApp>) {
    let app = harness.state_mut();
    app.create_report_tab(
        "Dashboard",
        gossia_core::report::ReportKind::SinglePage,
        None,
    )
    .unwrap();
    let mut design = app.calc().report("Dashboard").unwrap().clone();
    let skus = "[i.sku for i in Orders.items]";
    design.canvas = true;
    design.layout.controls = vec![
        chart(
            "Bar",
            &[
                ("labels", skus),
                (
                    "values",
                    "[[i.qty * i.price for i in Orders.items], [10, 20, 5]]",
                ),
                ("series", "['Ordered', 'Budget']"),
                ("title", "'Sales by product'"),
            ],
            [0, 0, 300, 170],
        ),
        chart(
            "Pie",
            &[
                ("labels", skus),
                ("values", "[i.qty for i in Orders.items]"),
                ("title", "'Units'"),
            ],
            [310, 0, 260, 170],
        ),
        {
            let mut c = chart(
                "Line",
                &[
                    ("labels", "['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun']"),
                    ("values", "[3, 5, 4, 8, 7, 11]"),
                    ("title", "'Orders per month'"),
                ],
                [0, 180, 570, 110],
            );
            c.fill_colour = Some("#1f2a44".into());
            c
        },
    ];
    app.session
        .calc_mut()
        .set_report("Dashboard", design)
        .unwrap();

    app.create_report_tab("Budget", gossia_core::report::ReportKind::SinglePage, None)
        .unwrap();
    let mut design = app.calc().report("Budget").unwrap().clone();
    let mut c = chart(
        "Sankey",
        &[
            (
                "sources",
                "['Salary', 'Salary', 'Freelance', 'Budget', 'Budget', 'Budget', 'Budget', 'Living', 'Living']",
            ),
            (
                "targets",
                "['Budget', 'Tax', 'Budget', 'Rent', 'Living', 'Savings', 'Holidays', 'Food', 'Transport']",
            ),
            (
                "values",
                "[3200, 1100, 600, 1300, 1100, 900, 500, 700, 400]",
            ),
            ("title", "'Where the money goes'"),
        ],
        [0, 0, 570, 290],
    );
    c.fill_colour = None;
    design.canvas = true;
    design.layout.controls = vec![c];
    app.session.calc_mut().set_report("Budget", design).unwrap();
    settle(harness);
}

#[test]
fn screenshots_for_the_store() {
    let Some(dir) = folder() else {
        eprintln!("skipped: set GOSSIA_STORE_SCREENSHOTS to a folder (needs a graphics adapter)");
        return;
    };

    // Four in the light theme, three in the dark one.
    // 1. The tree with its formulas, and the items as a table (dark).
    let mut harness = window(GossiaApp::sample());
    theme(&mut harness, false);
    harness.state_mut().reveal(&"customer".parse().unwrap());
    harness
        .state_mut()
        .set_tabular(&"items".parse().unwrap(), true);
    settle(&mut harness);
    save(&mut harness, &dir, "01-tree-and-formulas");

    // 2. A formula being typed, with completion of the workbook's names (light).
    theme(&mut harness, true);
    click(&mut harness, "0.21");
    harness.event(egui::Event::Text("=round(Orders.".to_owned()));
    harness.run();
    shoot(&mut harness, &dir, "02-formula-completion");
    harness.key_combination_modifiers(Modifiers::NONE, &[Key::Escape]);
    harness.run();
    harness.key_combination_modifiers(Modifiers::NONE, &[Key::Escape]);
    harness.run();

    // 3. The same data as a form (light).
    harness
        .state_mut()
        .set_tabular(&"items".parse().unwrap(), false);
    show_as(&mut harness, "Default");
    settle(&mut harness);
    save(&mut harness, &dir, "03-form");
    show_as(&mut harness, "Data tree");

    // 4. A report of charts, as it is read (light); 5. the same in the
    // designer (dark).
    with_reports(&mut harness);
    harness.state_mut().report.zoom = Some(1.75);
    click(&mut harness, "Dashboard");
    settle(&mut harness);
    save(&mut harness, &dir, "04-report-charts");
    theme(&mut harness, false);
    harness.state_mut().set_layout(true);
    harness.state_mut().report.zoom = Some(1.25);
    harness.state_mut().select_control(Some(vec![0]));
    settle(&mut harness);
    save(&mut harness, &dir, "05-report-designer");
    harness.state_mut().report.zoom = Some(1.75);
    harness.state_mut().set_layout(false);
    harness.state_mut().select_control(None);

    // 6. A Sankey diagram (light).
    theme(&mut harness, true);
    click(&mut harness, "Budget");
    settle(&mut harness);
    save(&mut harness, &dir, "06-sankey");

    // 7. The code editor with a Python module (dark).
    let mut app = GossiaApp::sample();
    app.session
        .calc_mut()
        .add_module(
            "Tax",
            "\"\"\"Tax rules shared by the formulas of this workbook.\"\"\"\n\nimport math\n\n\ndef vat(x, rate=0.21):\n    \"\"\"The VAT on an amount.\"\"\"\n    return round(x * rate, 2)\n\n\ndef gross(x):\n    # The amount with VAT.\n    return x + vat(x)\n\n\ndef rounded_up(x, step=0.05):\n    \"\"\"An amount rounded up to the next step.\"\"\"\n    return math.ceil(x / step) * step\n",
        )
        .unwrap();
    let mut harness = window(app);
    theme(&mut harness, false);
    harness.key_combination_modifiers(Modifiers::ALT, &[Key::F11]);
    harness.run();
    save(&mut harness, &dir, "07-code-editor");
}
