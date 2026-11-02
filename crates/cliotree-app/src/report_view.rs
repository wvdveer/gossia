//! Drawing a report in its tab (DESIGN.md, Reports, "The report designer"):
//! `cliotree-layout` places the controls and breaks the lines; egui only
//! draws each line where it was put, without wrapping it again, so the
//! screen and the PDF break lines in the same places.
//!
//! A single page report follows the width of the window. Its values are the
//! results of its expressions, which run while it is shown
//! ([`cliotree_calc::Calc::show_report`]).

use cliotree_calc::{Calc, ExprKind, ExprState};
use cliotree_layout::report::{self, Got, Ink, Laid, Paint, Prop, Source};
use egui::{Color32, Sense, Stroke};

use crate::app::ClioApp;
use crate::style::Palette;
use crate::view::named;

/// The space around a report in its tab, in points.
pub const MARGIN: f32 = 16.0;

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

/// What a report tab keeps between frames: the report as it was last laid
/// out, and what it was laid out for.
#[derive(Debug, Default)]
pub struct ReportView {
    key: Option<LaidKey>,
    pub laid: Option<Laid>,
}

/// What a layout depends on: the report, the width, the workbook's data and
/// results, the designs, trust, and what is waiting to run.
type LaidKey = (String, u32, u64, u64, bool, usize);

impl ClioApp {
    /// The current report laid out `width` points wide, made again only when
    /// something it shows has changed.
    pub fn lay_out_report(&mut self, width: f32) -> &Laid {
        let calc = self.session.calc();
        let key: LaidKey = (
            self.sheet.clone(),
            width.round() as u32,
            calc.revision(),
            calc.forms_version(),
            calc.is_trusted(),
            calc.pending_expressions(),
        );
        if self.report.key.as_ref() != Some(&key) || self.report.laid.is_none() {
            let laid = match calc.report(&self.sheet) {
                Some(design) => report::single_page(
                    design,
                    width,
                    &CalcSource {
                        calc,
                        report: &self.sheet,
                    },
                ),
                None => Laid::default(),
            };
            self.report.laid = Some(laid);
            self.report.key = Some(key);
        }
        self.report.laid.as_ref().expect("laid out above")
    }

    /// The page of a report tab: the report, drawn from its design and the
    /// results of its expressions.
    pub(crate) fn report_view(&mut self, ui: &mut egui::Ui) {
        let name = self.sheet.clone();
        let Some(design) = self.calc().report(&name).cloned() else {
            return;
        };
        let problems = design.problems();
        if design.layout.controls.is_empty() || !problems.is_empty() {
            ui.add_space(MARGIN);
            ui.horizontal(|ui| {
                ui.add_space(MARGIN);
                ui.vertical(|ui| self.report_notes(ui, &name, &problems));
            });
            if design.layout.controls.is_empty() {
                return;
            }
        }
        egui::ScrollArea::both()
            .id_salt(("report", name.as_str()))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let width = (ui.available_width() - 2.0 * MARGIN).max(100.0);
                let laid = self.lay_out_report(width).clone();
                let size = egui::vec2(laid.width + 2.0 * MARGIN, laid.height + 2.0 * MARGIN);
                let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                let origin = rect.min + egui::vec2(MARGIN, MARGIN);
                paint(ui, &laid, origin);
            });
    }

    /// What is said over a report that is empty or has problems.
    fn report_notes(
        &mut self,
        ui: &mut egui::Ui,
        name: &str,
        problems: &[(cliotree_core::form::Address, String)],
    ) {
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
            ui.weak("Its design can be edited in the code editor, under Reports.");
            let open = ui.button("Open the design in the code editor");
            named(&open, "Open report design");
            if open.clicked() {
                self.open_code_editor();
                self.open_report_design(name);
            }
        }
    }
}

/// The colour of an ink in the window's theme.
fn colour(ink: Ink, palette: &Palette) -> Color32 {
    match ink {
        Ink::Text => palette.text,
        Ink::Weak => palette.text_weak,
        Ink::Error => palette.error,
        Ink::Rule => palette.border,
        Ink::Header => palette.stripe,
        Ink::Rgba([r, g, b, a]) => Color32::from_rgba_unmultiplied(r, g, b, a),
    }
}

/// Paints a laid-out report with its top left at `origin`, and gives its
/// hovers their text.
pub fn paint(ui: &mut egui::Ui, laid: &Laid, origin: egui::Pos2) {
    let palette = Palette::of_ui(ui);
    let painter = ui.painter().clone();
    let at = |x: f32, y: f32| origin + egui::vec2(x, y);
    let rect_of = |r: &report::Rect| egui::Rect::from_min_size(at(r.x, r.y), egui::vec2(r.w, r.h));
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
                // Lines out of sight are not laid out by egui.
                if pos.y > clip.bottom() || pos.y + style.size * 2.0 < clip.top() {
                    continue;
                }
                let family = if style.bold {
                    crate::style::bold_family(style.typeface)
                } else {
                    crate::style::family(style.typeface)
                };
                let font = egui::FontId::new(style.size, family);
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
                    Stroke::new(*width, colour(*ink, palette)),
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
                    Stroke::new(*width, colour(*ink, palette)),
                );
            }
            Paint::Pending { x, y } => {
                painter.circle_filled(at(x - 5.0, y + 5.0), 3.0, palette.pending);
            }
        }
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
