//! Drawing a report in its tab (DESIGN.md, Reports, "Single page" and "The
//! report designer"): `cliotree-layout` places the controls on the canvas and
//! breaks the lines; egui only draws each line where it was put, without
//! wrapping it again, so the screen, the designer and the PDF break lines in
//! the same places.
//!
//! A single page report is a canvas as wide as the paper less its margins.
//! Its values are the results of its expressions, which run while it is
//! shown ([`cliotree_calc::Calc::show_report`]). A design made before the
//! canvas is turned into one when it is first shown, once its results are in.

use cliotree_calc::{Calc, ExprKind, ExprState, REPORT_FORM};
use cliotree_core::Value;
use cliotree_core::form::{Address, FormDesign};
use cliotree_core::report::ReportDesign;
use cliotree_layout::report::{self, Got, Ink, Laid, Paint, Prop, Source};
use egui::{Color32, Sense, Stroke};

use crate::app::ClioApp;
use crate::style::Palette;
use crate::view::named;

/// The space around a report in its tab, in points.
pub const MARGIN: f32 = 16.0;

/// The zooms offered, besides fitting the width.
pub const ZOOMS: [f32; 7] = [0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0];

/// The values of a report's expressions, as the workbook has them.
pub struct CalcSource<'a> {
    pub calc: &'a Calc,
    pub report: &'a str,
}

fn kind(prop: Prop) -> ExprKind {
    match prop {
        Prop::Visible => ExprKind::Visible,
        Prop::TextColour => ExprKind::TextColour,
        Prop::FillColour => ExprKind::FillColour,
        Prop::Value => ExprKind::Value,
        Prop::Template => ExprKind::Template,
        Prop::Items => ExprKind::Items,
    }
}

/// What an expression came to, as the report shows it: nothing runs in a
/// workbook that is not trusted.
fn got(trusted: bool, state: Option<ExprState>) -> Got {
    let Some(state) = state else {
        return Got::None;
    };
    if !trusted {
        return Got::Untrusted;
    }
    match state.result {
        Some(Ok(value)) => Got::Value(value, state.pending),
        Some(Err(error)) => Got::Error(error),
        None => Got::Waiting,
    }
}

impl Source for CalcSource<'_> {
    fn control(&self, at: &[usize], rows: &[usize], prop: Prop) -> Got {
        got(
            self.calc.is_trusted(),
            self.calc
                .report_expression(self.report, at, rows, kind(prop)),
        )
    }

    fn column(
        &self,
        at: &[usize],
        rows: &[usize],
        column: &str,
        row: Option<usize>,
        prop: Prop,
    ) -> Got {
        got(
            self.calc.is_trusted(),
            self.calc
                .report_column_expression(self.report, at, rows, column, row, kind(prop)),
        )
    }
}

/// The expressions of a report in place of their values (*Show
/// expressions*, as Crystal's design view): a field shows `[its value]`, and
/// a template label its text.
pub struct ExprSource<'a> {
    pub design: &'a FormDesign,
}

impl Source for ExprSource<'_> {
    fn control(&self, at: &[usize], _rows: &[usize], prop: Prop) -> Got {
        let Some(c) = self.design.get(at) else {
            return Got::None;
        };
        let shown = |v: Value| Got::Value(v, false);
        match prop {
            Prop::Value => c
                .value
                .as_deref()
                .map_or(Got::None, |v| shown(Value::Str(format!("[{}]", v.trim())))),
            Prop::Template => c.text.clone().map_or(Got::None, |t| shown(Value::Str(t))),
            _ => Got::None,
        }
    }

    fn column(&self, _: &[usize], _: &[usize], _: &str, _: Option<usize>, _: Prop) -> Got {
        Got::None
    }
}

/// What a report tab keeps between frames: the report as it was last laid
/// out and what for, the zoom, and the designer's state.
#[derive(Debug, Default)]
pub struct ReportView {
    key: Option<LaidKey>,
    pub laid: Option<Laid>,
    /// A zoom chosen, or fitting the width (None).
    pub zoom: Option<f32>,
    /// In layout mode: the expressions shown in place of the values.
    pub show_expressions: bool,
    /// In layout mode: what is being dragged on the canvas.
    pub drag: Option<crate::report_design::CanvasDrag>,
    /// In layout mode: where the top left of the canvas was drawn, and the
    /// zoom, in the last frame.
    pub canvas_at: Option<(egui::Pos2, f32)>,
}

/// What a layout depends on: the report, the data and results, the designs,
/// trust, what is waiting to run, and whether expressions are shown.
type LaidKey = (String, u64, u64, bool, usize, bool);

impl ClioApp {
    /// The current report laid out, made again only when something it shows
    /// has changed.
    pub fn lay_out_report(&mut self) -> &Laid {
        let calc = self.session.calc();
        let expressions = self.report.show_expressions && self.layout_active();
        let key: LaidKey = (
            self.sheet.clone(),
            calc.revision(),
            calc.forms_version(),
            calc.is_trusted(),
            calc.pending_expressions(),
            expressions,
        );
        if self.report.key.as_ref() != Some(&key) || self.report.laid.is_none() {
            let laid = match calc.report(&self.sheet) {
                Some(design) if expressions => report::canvas(
                    design,
                    &ExprSource {
                        design: &design.layout,
                    },
                ),
                Some(design) => {
                    let source = CalcSource {
                        calc,
                        report: &self.sheet,
                    };
                    if design.canvas {
                        report::canvas(design, &source)
                    } else {
                        report::single_page(design, design.page.content_width(), &source)
                    }
                }
                None => Laid::default(),
            };
            self.report.laid = Some(laid);
            self.report.key = Some(key);
        }
        self.report.laid.as_ref().expect("laid out above")
    }

    /// The zoom a report is drawn at, `available` points being the width of
    /// the tab.
    pub fn report_zoom(&self, available: f32, width: f32) -> f32 {
        self.report.zoom.unwrap_or_else(|| {
            ((available - 2.0 * MARGIN) / width.max(1.0)).clamp(ZOOMS[0], ZOOMS[ZOOMS.len() - 1])
        })
    }

    /// Turns a report made before the canvas into one, when its results are
    /// in (an untrusted workbook has none, so its heights stay automatic).
    fn turn_into_canvas(&mut self, name: &str, design: &ReportDesign) {
        let calc = self.session.calc();
        if !self.session.is_idle() || calc.shown_report() != Some(name) {
            return;
        }
        let trusted = calc.is_trusted();
        if trusted && (calc.pending_expressions() > 0 || !results_in(calc, name, design)) {
            return;
        }
        let source = CalcSource { calc, report: name };
        let turned = report::to_canvas(design, &source, trusted);
        // The report is there: it was just read.
        let _ = self.session.calc_mut().set_report(name, turned);
    }

    /// The page of a report tab: the strip, then the report drawn from its
    /// design and the results of its expressions, or in layout mode the
    /// designer.
    pub(crate) fn report_view(&mut self, ui: &mut egui::Ui) {
        let name = self.sheet.clone();
        let Some(design) = self.calc().report(&name).cloned() else {
            return;
        };
        if !design.canvas {
            self.turn_into_canvas(&name, &design);
        }
        if let Some(notice) = &self.notice {
            let color = crate::view::error_color(ui);
            ui.colored_label(color, notice);
        }
        self.form_bar(ui, REPORT_FORM);
        let problems = design.problems();
        let layout = self.layout_active();
        if (design.layout.controls.is_empty() && !layout) || !problems.is_empty() {
            ui.add_space(MARGIN);
            ui.horizontal(|ui| {
                ui.add_space(MARGIN);
                ui.vertical(|ui| self.report_notes(ui, &name, &problems));
            });
            if design.layout.controls.is_empty() && !layout {
                return;
            }
        }
        if layout {
            self.report_designer(ui);
            return;
        }
        egui::ScrollArea::both()
            .id_salt(("report", name.as_str()))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let laid = self.lay_out_report().clone();
                let zoom = self.report_zoom(ui.available_width(), laid.width);
                let size = egui::vec2(
                    laid.width * zoom + 2.0 * MARGIN,
                    laid.height * zoom + 2.0 * MARGIN,
                );
                let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                let origin = rect.min + egui::vec2(MARGIN, MARGIN);
                paint(ui, &laid, origin, zoom, true);
            });
    }

    /// The zoom, and in layout mode *Show expressions*: on the strip above a
    /// report.
    pub(crate) fn report_tools(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        let text = match self.report.zoom {
            None => "Fit width".to_owned(),
            Some(z) => format!("{:.0}%", z * 100.0),
        };
        let response = egui::ComboBox::from_id_salt("report_zoom")
            .selected_text(text)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut self.report.zoom, None, "Fit width");
                for z in ZOOMS {
                    ui.selectable_value(
                        &mut self.report.zoom,
                        Some(z),
                        format!("{:.0}%", z * 100.0),
                    );
                }
            })
            .response;
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, "Zoom"));
        if self.layout_active() {
            ui.checkbox(&mut self.report.show_expressions, "Show expressions")
                .on_hover_text(
                    "Each field shows its expression, as [Orders.total], in place of its value",
                );
        }
    }

    /// What is said over a report that is empty or has problems.
    fn report_notes(&mut self, ui: &mut egui::Ui, name: &str, problems: &[(Address, String)]) {
        for (at, why) in problems {
            let place: Vec<String> = at.iter().map(usize::to_string).collect();
            ui.colored_label(
                crate::view::error_color(ui),
                format!("Control {}: {why}", place.join(".")),
            );
        }
        if self
            .calc()
            .report(name)
            .is_some_and(|d| d.layout.controls.is_empty())
        {
            ui.label("This report has no controls yet.");
            ui.weak(
                "Turn on Edit report to add controls, or edit its design in the code editor, \
                 under Reports.",
            );
            let open = ui.button("Open the design in the code editor");
            named(&open, "Open report design");
            if open.clicked() {
                self.open_code_editor();
                self.open_report_design(name);
            }
        }
    }
}

/// Whether every expression of a report has a result: one that was not
/// looked at yet has none, and the places it gives would be wrong.
fn results_in(calc: &Calc, name: &str, design: &ReportDesign) -> bool {
    struct Probe<'a> {
        inner: CalcSource<'a>,
        design: &'a FormDesign,
        missing: std::cell::Cell<bool>,
    }
    impl Source for Probe<'_> {
        fn control(&self, at: &[usize], rows: &[usize], prop: Prop) -> Got {
            let g = self.inner.control(at, rows, prop);
            let has = prop == Prop::Value && self.design.get(at).is_some_and(|c| c.value.is_some());
            if matches!(g, Got::Waiting) || (has && g == Got::None) {
                self.missing.set(true);
            }
            g
        }
        fn column(
            &self,
            at: &[usize],
            rows: &[usize],
            column: &str,
            row: Option<usize>,
            prop: Prop,
        ) -> Got {
            let g = self.inner.column(at, rows, column, row, prop);
            if matches!(g, Got::Waiting) {
                self.missing.set(true);
            }
            g
        }
    }
    let probe = Probe {
        inner: CalcSource { calc, report: name },
        design: &design.layout,
        missing: std::cell::Cell::new(false),
    };
    report::single_page(design, design.page.content_width(), &probe);
    !probe.missing.get()
}

/// The colour of an ink in the window's theme.
fn colour(ink: Ink, palette: &Palette) -> Color32 {
    match ink {
        Ink::Text => palette.text,
        Ink::Weak => palette.text_weak,
        Ink::Error => palette.error,
        Ink::Rule => palette.border,
        Ink::Rgba([r, g, b, a]) => Color32::from_rgba_unmultiplied(r, g, b, a),
    }
}

/// Paints a laid-out report with its top left at `origin`, `zoom` screen
/// points to a report point, and with `hovers`, gives its hovers their text.
pub fn paint(ui: &mut egui::Ui, laid: &Laid, origin: egui::Pos2, zoom: f32, hovers: bool) {
    let palette = Palette::of_ui(ui);
    let painter = ui.painter().clone();
    let at = |x: f32, y: f32| origin + egui::vec2(x * zoom, y * zoom);
    let rect_of = |r: &report::Rect| {
        egui::Rect::from_min_size(at(r.x, r.y), egui::vec2(r.w * zoom, r.h * zoom))
    };
    let clip = ui.clip_rect();
    for p in &laid.paints {
        match p {
            Paint::Text {
                x,
                y,
                text,
                style,
                ink,
            } => {
                let pos = at(*x, *y);
                let size = style.size * zoom;
                // Lines out of sight are not laid out by egui.
                if pos.y > clip.bottom() || pos.y + size * 2.0 < clip.top() {
                    continue;
                }
                let family = if style.bold {
                    crate::style::bold_family(style.typeface)
                } else {
                    crate::style::family(style.typeface)
                };
                let font = egui::FontId::new(size, family);
                let mut job = egui::text::LayoutJob::simple_singleline(
                    text.clone(),
                    font,
                    colour(*ink, palette),
                );
                if style.italic {
                    for section in &mut job.sections {
                        section.format.italics = true;
                    }
                }
                let galley = ui.fonts_mut(|f| f.layout_job(job));
                painter.galley(pos, galley, palette.text);
            }
            Paint::Fill { rect, ink } => {
                painter.rect_filled(rect_of(rect), 0.0, colour(*ink, palette));
            }
            Paint::Border { rect, width, ink } => {
                painter.rect_stroke(
                    rect_of(rect),
                    0.0,
                    Stroke::new(width * zoom, colour(*ink, palette)),
                    egui::StrokeKind::Inside,
                );
            }
            Paint::Line {
                from,
                to,
                width,
                ink,
            } => {
                painter.line_segment(
                    [at(from.0, from.1), at(to.0, to.1)],
                    Stroke::new(width * zoom, colour(*ink, palette)),
                );
            }
            Paint::Pending { x, y } => {
                painter.circle_filled(at(*x, *y) + egui::vec2(-5.0, 5.0), 3.0, palette.pending);
            }
        }
    }
    if !hovers {
        return;
    }
    for (i, hover) in laid.hovers.iter().enumerate() {
        let rect = rect_of(&hover.rect);
        if !ui.is_rect_visible(rect) {
            continue;
        }
        ui.interact(
            rect,
            ui.scope_id().with(("report_hover", i)),
            Sense::hover(),
        )
        .on_hover_text(&hover.text);
    }
}
