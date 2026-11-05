//! The report designer (DESIGN.md, Reports, "The report designer"): layout
//! mode on the canvas. The report is drawn by `gossia-layout` exactly as
//! the report view draws it; the selection, the resize handles, the guide
//! lines and what is dropped from the palette are drawn over it, from where
//! the layout says each control is.
//!
//! Places are kept in report points, not on the screen, so the zoom changes
//! nothing; the property page and the align tools read them as they read a
//! form's ([`FormRects`]).
//!
//! A multipage report is its stack of band canvases, each with its name on
//! its left. Dragging the line under a band changes its height, and a
//! control is dragged from one band to another.

use egui::{Pos2, Rect, Sense, Stroke, Vec2, pos2, vec2};
use gossia_calc::REPORT_FORM;
use gossia_core::form::{Address, CONTROL_HEIGHTS, Control, ControlKind};
use gossia_core::report::{BandKind, ReportDesign};
use gossia_layout::report::{self, BAND_GAP, Laid};

use crate::app::GossiaApp;
use crate::form_free::{FormRects, GRID, Guide, Placement, keep_rects, snap_move};
use crate::form_layout::{ControlAction, DragItem, new_control};
use crate::report_view::{MARGIN, paint};
use crate::style::Palette;

/// The size of a resize handle on the screen.
const HANDLE: f32 = 7.0;

/// The room under the canvas for dragging its bottom edge down.
const BELOW: f32 = 48.0;

/// The room on the left of the bands of a multipage report for their names,
/// on the screen.
const STRIP: f32 = 140.0;

/// What is being dragged on the canvas, in report points.
#[derive(Debug, Clone, PartialEq)]
pub enum CanvasDrag {
    /// A control, from where it was.
    Move {
        at: Address,
        start: Rect,
        grab: Pos2,
    },
    /// An edge or a corner of a control.
    Resize {
        at: Address,
        start: Rect,
        grab: Pos2,
        handle: Handle,
    },
    /// The bottom edge of the canvas.
    Height { start: f32, grab: f32 },
    /// The line under a band of a multipage report.
    Band { band: usize, start: f32, grab: f32 },
}

/// A resize handle: which edges it moves, -1 the left or top, 1 the right or
/// bottom, 0 neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Handle(i8, i8);

impl Handle {
    const ALL: [Handle; 8] = [
        Handle(-1, -1),
        Handle(0, -1),
        Handle(1, -1),
        Handle(1, 0),
        Handle(1, 1),
        Handle(0, 1),
        Handle(-1, 1),
        Handle(-1, 0),
    ];

    fn point(self, r: Rect) -> Pos2 {
        let x = match self.0 {
            -1 => r.left(),
            1 => r.right(),
            _ => r.center().x,
        };
        let y = match self.1 {
            -1 => r.top(),
            1 => r.bottom(),
            _ => r.center().y,
        };
        pos2(x, y)
    }

    fn cursor(self) -> egui::CursorIcon {
        use egui::CursorIcon as C;
        match (self.0, self.1) {
            (0, _) => C::ResizeVertical,
            (_, 0) => C::ResizeHorizontal,
            (a, b) if a == b => C::ResizeNwSe,
            _ => C::ResizeNeSw,
        }
    }
}

fn rect_of(r: &report::Rect) -> Rect {
    Rect::from_min_size(pos2(r.x, r.y), vec2(r.w, r.h))
}

/// Where each control was drawn (the first item of each repeater around it)
/// and the inside of each Free group and of the canvas, in report points.
pub(crate) fn canvas_rects(laid: &Laid) -> FormRects {
    let mut rects = FormRects::default();
    let first = |rows: &[usize]| rows.iter().all(|r| *r == 0);
    for a in laid.areas.iter().filter(|a| first(&a.rows)) {
        rects
            .controls
            .entry(a.at.clone())
            .or_insert_with(|| rect_of(&a.rect));
    }
    for a in laid.insides.iter().filter(|a| first(&a.rows)) {
        rects
            .free
            .entry(a.at.clone())
            .or_insert_with(|| rect_of(&a.rect));
    }
    rects
}

/// The control drawn at `p`: the innermost, and of those the smallest.
fn control_at(rects: &FormRects, p: Pos2) -> Option<Address> {
    rects
        .controls
        .iter()
        .filter(|(_, r)| r.contains(p))
        .max_by(|(a, r), (b, s)| a.len().cmp(&b.len()).then((s.area()).total_cmp(&r.area())))
        .map(|(at, _)| at.clone())
}

fn snap(v: f32, grid: bool) -> f32 {
    if grid {
        (v / GRID).round() * GRID
    } else {
        v.round()
    }
}

/// A new control as the canvas adds it: with a box, since every control on
/// a canvas keeps the box it was given.
pub(crate) fn sized_control(kind: ControlKind) -> Control {
    let mut c = new_control(kind);
    let (w, h) = match kind {
        ControlKind::Label => (160, 20),
        ControlKind::Field => (320, 20),
        ControlKind::Group => (240, 80),
        ControlKind::Repeater => (400, 120),
        ControlKind::Separator => (240, 16),
        ControlKind::Button => (100, 24),
        ControlKind::Chart => gossia_core::form::CHART_SIZE,
    };
    if kind == ControlKind::Field {
        c.label = Some("Field".to_owned());
    }
    c.width = Some(w);
    c.height = Some(h);
    c
}

impl GossiaApp {
    /// Layout mode on a report: its canvas, drawn as the report view draws
    /// it, with the selection and the handles over it.
    pub(crate) fn report_designer(&mut self, ui: &mut egui::Ui) {
        let laid = self.lay_out_report().clone();
        let rects = canvas_rects(&laid);
        keep_rects(ui.ctx(), &self.sheet, REPORT_FORM, rects.clone());
        let name = self.sheet.clone();
        let banded = self
            .calc()
            .report(&name)
            .filter(|d| d.bands().is_some())
            .cloned();
        let (left, below) = if banded.is_some() {
            (STRIP, 0.0)
        } else {
            (0.0, BELOW)
        };
        egui::ScrollArea::both()
            .id_salt(("report_design", name.as_str()))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let zoom = self.report_zoom(ui.available_width() - left, laid.width);
                let size = vec2(
                    laid.width * zoom + 2.0 * MARGIN + left,
                    laid.height * zoom + 2.0 * MARGIN + below,
                );
                let (outer, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
                let origin = outer.min + vec2(MARGIN + left, MARGIN);
                self.report.canvas_at = Some((origin, zoom));
                self.canvas(ui, &laid, &rects, &response, origin, zoom, banded.as_ref());
            });
        // Delete takes the selected controls away, the last first so that
        // the addresses of the others hold.
        let typing = ui.ctx().text_edit_focused();
        if !typing && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Delete)) {
            let mut selected = self.selected_controls();
            // A band is not deleted; hiding it gives it a height of 0.
            selected.retain(|at| !self.is_band(at));
            selected.sort();
            for at in selected.into_iter().rev() {
                self.apply_control(ControlAction::Delete(at));
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn canvas(
        &mut self,
        ui: &mut egui::Ui,
        laid: &Laid,
        rects: &FormRects,
        response: &egui::Response,
        origin: Pos2,
        zoom: f32,
        banded: Option<&ReportDesign>,
    ) {
        let palette = Palette::of_ui(ui);
        let painter = ui.painter().clone();
        let to_screen = |r: Rect| {
            Rect::from_min_max(
                origin + r.min.to_vec2() * zoom,
                origin + r.max.to_vec2() * zoom,
            )
        };
        let to_points = |pos: Pos2| pos2((pos.x - origin.x) / zoom, (pos.y - origin.y) / zoom);
        let page = Rect::from_min_size(Pos2::ZERO, vec2(laid.width, laid.height));
        let canvas = to_screen(page);

        // The page, or each band, with a line of the grid every four steps.
        let faint = Stroke::new(0.5, palette.border.gamma_multiply(0.4));
        let step = GRID * 4.0;
        let sheet = |area: Rect, hidden: bool| {
            let screen = to_screen(area);
            if hidden {
                painter.rect_filled(screen, 0.0, palette.border.gamma_multiply(0.3));
            } else {
                painter.rect_filled(screen, 0.0, palette.field);
                let mut x = step;
                while x < area.width() {
                    let sx = screen.left() + x * zoom;
                    painter
                        .line_segment([pos2(sx, screen.top()), pos2(sx, screen.bottom())], faint);
                    x += step;
                }
                let mut y = step;
                while y < area.height() {
                    let sy = screen.top() + y * zoom;
                    painter
                        .line_segment([pos2(screen.left(), sy), pos2(screen.right(), sy)], faint);
                    y += step;
                }
            }
            painter.rect_stroke(
                screen,
                0.0,
                Stroke::new(1.0, palette.border),
                egui::StrokeKind::Outside,
            );
        };
        match banded {
            Some(design) => {
                let small = egui::FontId::proportional(11.0);
                for (i, r) in &laid.bands {
                    let hidden = design.layout.controls[*i].height.unwrap_or(0) == 0;
                    sheet(rect_of(r), hidden);
                    // Its name, on the left.
                    let screen = to_screen(rect_of(r));
                    let mut name = design.band_name(*i);
                    if hidden {
                        name.push_str(" (hidden)");
                    }
                    let strip = Rect::from_min_max(
                        pos2(screen.left() - STRIP + 4.0, screen.top()),
                        pos2(
                            screen.left() - 6.0,
                            screen.bottom().max(screen.top() + 13.0),
                        ),
                    );
                    painter.with_clip_rect(strip).text(
                        pos2(strip.right(), strip.top()),
                        egui::Align2::RIGHT_TOP,
                        name,
                        small.clone(),
                        if hidden {
                            palette.text_weak
                        } else {
                            palette.text
                        },
                    );
                }
            }
            None => sheet(page, false),
        }
        paint(ui, laid, origin, zoom, false);

        // A cut control has a mark at the middle of its bottom edge.
        for r in &laid.cuts {
            let r = to_screen(rect_of(r));
            let (cx, b) = (r.center().x, r.bottom());
            painter.add(egui::Shape::convex_polygon(
                vec![
                    pos2(cx - 5.0, b - 1.0),
                    pos2(cx + 5.0, b - 1.0),
                    pos2(cx, b - 7.0),
                ],
                palette.error,
                Stroke::NONE,
            ));
        }

        let selected = self.selected_controls();
        // A band has no handles: its height is the line under it.
        let primary = selected
            .first()
            .filter(|at| banded.is_none() || at.len() > 1)
            .and_then(|at| rects.controls.get(at).map(|r| (at.clone(), *r)));
        let handle_at = |pos: Pos2| {
            let (at, r) = primary.as_ref()?;
            let r = to_screen(*r);
            Handle::ALL
                .into_iter()
                .find(|h| {
                    Rect::from_center_size(h.point(r), Vec2::splat(HANDLE + 4.0)).contains(pos)
                })
                .map(|h| (at.clone(), h))
        };
        // The bottom edge of a single page canvas; the line under each band.
        let edge = Rect::from_min_max(
            pos2(canvas.left(), canvas.bottom() - 3.0),
            pos2(canvas.right(), canvas.bottom() + 5.0),
        );
        let edge = if banded.is_some() {
            Rect::NOTHING
        } else {
            edge
        };
        let band_lines: Vec<(usize, Rect, Rect)> = if banded.is_some() {
            laid.bands
                .iter()
                .map(|(i, r)| {
                    let area = rect_of(r);
                    let screen = to_screen(area);
                    let line = Rect::from_min_max(
                        pos2(screen.left(), screen.bottom() - 3.0),
                        pos2(screen.right(), screen.bottom() + BAND_GAP * zoom + 1.0),
                    );
                    (*i, area, line)
                })
                .collect()
        } else {
            Vec::new()
        };
        let band_line_at = |pos: Pos2| {
            band_lines
                .iter()
                .find(|(_, _, line)| line.contains(pos))
                .map(|(i, area, _)| (*i, *area))
        };
        if let Some(pos) = response.hover_pos() {
            if let Some((_, h)) = handle_at(pos) {
                ui.ctx().set_cursor_icon(h.cursor());
            } else if edge.contains(pos) || band_line_at(pos).is_some() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
            }
        }

        // A drag starts on a handle, on the bottom edge, or on a control.
        // It starts where the button went down, not where the pointer is
        // once it has moved far enough to count as a drag.
        if response.drag_started()
            && let Some(pos) = ui
                .input(|i| i.pointer.press_origin())
                .or(response.interact_pointer_pos())
        {
            let p = to_points(pos);
            let drag = if let Some((at, handle)) = handle_at(pos) {
                let start = rects.controls[&at];
                Some(CanvasDrag::Resize {
                    at,
                    start,
                    grab: p,
                    handle,
                })
            } else if edge.contains(pos) {
                Some(CanvasDrag::Height {
                    start: laid.height,
                    grab: p.y,
                })
            } else if let Some((band, _)) = band_line_at(pos) {
                let start = banded
                    .and_then(|d| d.layout.controls.get(band))
                    .and_then(|c| c.height)
                    .unwrap_or(0) as f32;
                Some(CanvasDrag::Band {
                    band,
                    start,
                    grab: p.y,
                })
            } else if let Some(at) = control_at(rects, p) {
                if !selected.contains(&at) {
                    self.select_control(Some(at.clone()));
                }
                // Only a control in a Free group, or on the canvas, moves.
                let parent = at.split_last().map(|(_, p)| p.to_vec()).unwrap_or_default();
                rects.free.contains_key(&parent).then(|| CanvasDrag::Move {
                    start: rects.controls[&at],
                    at,
                    grab: p,
                })
            } else {
                None
            };
            if drag.is_some() {
                self.apply_control(ControlAction::ResizeStart);
            }
            self.report.drag = drag;
        }

        // Where the dragged thing would go, snapped.
        let grid = !ui.input(|i| i.modifiers.alt);
        // On the frame it is let go, the pointer is where it was last seen.
        let pointer = response
            .interact_pointer_pos()
            .or_else(|| ui.input(|i| i.pointer.latest_pos()))
            .map(to_points);
        let mut preview: Option<(Address, Rect, Rect)> = None;
        let mut guides: Vec<(Rect, Guide)> = Vec::new();
        let mut new_height = None;
        let mut band_height: Option<(usize, f32, f32)> = None;
        // Where a moved control goes: the Free group or band under the
        // pointer, which may be another than its own.
        let mut target: Option<Address> = None;
        if let (Some(drag), Some(p)) = (self.report.drag.clone(), pointer) {
            match drag {
                CanvasDrag::Move { at, start, grab } => {
                    let own = at.split_last().map(|(_, p)| p.to_vec()).unwrap_or_default();
                    let parent = rects
                        .free
                        .iter()
                        .filter(|(a, r)| r.contains(p) && !a.starts_with(&at))
                        .max_by_key(|(a, _)| a.len())
                        .map_or(own, |(a, _)| a.clone());
                    target = Some(parent.clone());
                    let region = rects.free.get(&parent).copied().unwrap_or(page);
                    let moved = start.translate(p - grab).translate(-region.min.to_vec2());
                    let others: Vec<Rect> = rects
                        .controls
                        .iter()
                        .filter(|(a, _)| {
                            a.split_last().map(|(_, q)| q) == Some(parent.as_slice()) && **a != at
                        })
                        .map(|(_, r)| r.translate(-region.min.to_vec2()))
                        .collect();
                    let (pos, snapped) = snap_move(moved, &others, region.size(), grid);
                    guides = snapped.into_iter().map(|g| (region, g)).collect();
                    preview = Some((
                        at,
                        Rect::from_min_size(region.min + pos.to_vec2(), start.size()),
                        region,
                    ));
                }
                CanvasDrag::Resize {
                    at,
                    start,
                    grab,
                    handle,
                } => {
                    let parent = at.split_last().map(|(_, p)| p.to_vec()).unwrap_or_default();
                    let region = rects.free.get(&parent).copied().unwrap_or(page);
                    let d = p - grab;
                    let (mut l, mut t, mut r, mut b) =
                        (start.left(), start.top(), start.right(), start.bottom());
                    // An edge moves with the pointer, snapped inside its group.
                    let rel = |v: f32, delta: f32| snap(v + delta, grid);
                    match handle.0 {
                        -1 => l = rel(l - region.left(), d.x) + region.left(),
                        1 => r = rel(r - region.left(), d.x) + region.left(),
                        _ => {}
                    }
                    match handle.1 {
                        -1 => t = rel(t - region.top(), d.y) + region.top(),
                        1 => b = rel(b - region.top(), d.y) + region.top(),
                        _ => {}
                    }
                    let (min_w, min_h) = (24.0, 16.0);
                    if r - l < min_w {
                        if handle.0 == -1 {
                            l = r - min_w;
                        } else {
                            r = l + min_w;
                        }
                    }
                    if b - t < min_h {
                        if handle.1 == -1 {
                            t = b - min_h;
                        } else {
                            b = t + min_h;
                        }
                    }
                    preview = Some((at, Rect::from_min_max(pos2(l, t), pos2(r, b)), region));
                }
                CanvasDrag::Height { start, grab } => {
                    let lowest = rects
                        .controls
                        .iter()
                        .filter(|(a, _)| a.len() == 1)
                        .map(|(_, r)| r.bottom())
                        .fold(0.0, f32::max);
                    let h = snap(start + p.y - grab, grid).max(lowest.ceil());
                    new_height = Some(h);
                }
                CanvasDrag::Band { band, start, grab } => {
                    // Not above the lowest control on it.
                    let top = laid
                        .bands
                        .iter()
                        .find(|(i, _)| *i == band)
                        .map_or(0.0, |(_, r)| r.y);
                    let lowest = rects
                        .controls
                        .iter()
                        .filter(|(a, _)| a.len() == 2 && a[0] == band)
                        .map(|(_, r)| r.bottom() - top)
                        .fold(0.0, f32::max);
                    let h = snap(start + p.y - grab, grid)
                        .max(lowest.ceil())
                        .clamp(0.0, *CONTROL_HEIGHTS.end() as f32);
                    band_height = Some((band, top, h));
                }
            }
        }
        let accent = palette.accent;
        if let Some((_, r, _)) = &preview {
            painter.rect_stroke(
                to_screen(*r),
                0.0,
                Stroke::new(1.5, accent),
                egui::StrokeKind::Outside,
            );
        }
        for (region, g) in &guides {
            let line = if g.vertical {
                let x = region.left() + g.at;
                [pos2(x, region.top()), pos2(x, region.bottom())]
            } else {
                let y = region.top() + g.at;
                [pos2(region.left(), y), pos2(region.right(), y)]
            };
            let a = origin + line[0].to_vec2() * zoom;
            let b = origin + line[1].to_vec2() * zoom;
            painter.line_segment([a, b], Stroke::new(1.0, palette.warning));
        }
        if let Some(h) = new_height {
            let y = origin.y + h * zoom;
            painter.line_segment(
                [pos2(canvas.left(), y), pos2(canvas.right(), y)],
                Stroke::new(1.5, accent),
            );
        }
        if let Some((_, top, h)) = band_height {
            let y = origin.y + (top + h) * zoom;
            painter.line_segment(
                [pos2(canvas.left(), y), pos2(canvas.right(), y)],
                Stroke::new(1.5, accent),
            );
            painter.text(
                pos2(canvas.right() + 6.0, y),
                egui::Align2::LEFT_CENTER,
                format!("{h:.0} pt"),
                egui::FontId::proportional(11.0),
                accent,
            );
        }

        // Let go: the change, as one step of Undo layout.
        if response.drag_stopped()
            && let Some(drag) = self.report.drag.take()
        {
            match (drag, preview) {
                // Into another band or Free group: moved there, at its place.
                (CanvasDrag::Move { at, .. }, Some((_, r, region)))
                    if target
                        .as_ref()
                        .is_some_and(|t| at.split_last().map(|(_, p)| p) != Some(t.as_slice())) =>
                {
                    let target = target.unwrap_or_default();
                    let (x, y) = (r.left() - region.left(), r.top() - region.top());
                    self.edit_design(|d| {
                        let index = d.get(&target).map_or(0, |g| g.children.len());
                        let moved = d.move_to(&at, &target, index)?;
                        let c = d.get_mut(&moved)?;
                        c.x = Some(x.round().max(0.0) as u32);
                        c.y = Some(y.round().max(0.0) as u32);
                        Some(Some(moved))
                    });
                }
                (CanvasDrag::Move { at, start, .. }, Some((_, r, region)))
                    if r.min != start.min =>
                {
                    let placement = Placement {
                        at,
                        x: Some(r.left() - region.left()),
                        y: Some(r.top() - region.top()),
                        size: Some((region.width(), region.height())),
                        ..Placement::default()
                    };
                    self.apply_control(ControlAction::Place(vec![placement], Some("move".into())));
                }
                (CanvasDrag::Resize { at, start, .. }, Some((_, r, region))) if r != start => {
                    let placement = Placement {
                        at,
                        x: Some(r.left() - region.left()),
                        y: Some(r.top() - region.top()),
                        width: Some(r.width()),
                        height: Some(r.height()),
                        size: Some((region.width(), region.height())),
                        ..Placement::default()
                    };
                    self.apply_control(ControlAction::Place(
                        vec![placement],
                        Some("resize".into()),
                    ));
                }
                (CanvasDrag::Height { .. }, _) => {
                    if let Some(h) = new_height {
                        self.set_canvas_height(h);
                    }
                }
                (CanvasDrag::Band { band, start, .. }, _) => {
                    if let Some((_, _, h)) = band_height
                        && h != start
                    {
                        self.resize_control(&vec![band], None, Some(Some(h as u32)), None);
                    }
                }
                _ => {}
            }
        }

        // A click selects what is under it; with Ctrl, adds it or takes it
        // out; on the empty page, selects nothing.
        if response.clicked()
            && let Some(pos) = response.interact_pointer_pos()
        {
            let hit = control_at(rects, to_points(pos));
            let ctrl = ui.input(|i| i.modifiers.command);
            match hit {
                Some(at) if ctrl => self.toggle_control(at),
                hit => self.select_control(hit),
            }
        }

        // The selection, and the handles of the one the others follow.
        let dragging = self.report.drag.is_some();
        for (i, at) in self.selected_controls().iter().enumerate() {
            let Some(r) = rects.controls.get(at) else {
                continue;
            };
            let r = to_screen(*r);
            let width = if i == 0 { 1.5 } else { 1.0 };
            painter.rect_stroke(
                r,
                0.0,
                Stroke::new(width, palette.selection_outline),
                egui::StrokeKind::Outside,
            );
            if i == 0 && !dragging && primary.is_some() {
                for h in Handle::ALL {
                    let square = Rect::from_center_size(h.point(r), Vec2::splat(HANDLE));
                    painter.rect_filled(square, 1.0, palette.selection_outline);
                }
            }
        }

        // Something dragged from the palette goes where it is let go.
        let ctx = ui.ctx().clone();
        if let Some(payload) = egui::DragAndDrop::payload::<(DragItem, String)>(&ctx)
            && let Some(pos) = ctx.pointer_latest_pos()
            && canvas.expand(4.0).contains(pos)
        {
            let (item, label) = payload.as_ref().clone();
            let p = to_points(pos);
            let parent = drop_parent(rects, p);
            if let Some(region) = rects.free.get(&parent) {
                painter.rect_stroke(
                    to_screen(*region),
                    0.0,
                    Stroke::new(1.0, accent),
                    egui::StrokeKind::Inside,
                );
            }
            painter.text(
                pos + vec2(14.0, 4.0),
                egui::Align2::LEFT_TOP,
                label,
                egui::TextStyle::Body.resolve(ui.style()),
                ui.visuals().strong_text_color(),
            );
            if ui.input(|i| i.pointer.any_released())
                && let DragItem::New(kind) = item
            {
                egui::DragAndDrop::clear_payload(&ctx);
                self.drop_on_canvas(kind, &parent, p, rects);
            }
        }
    }

    /// Adds a control of `kind` where it was dropped: in the innermost Free
    /// group at that point, or on the canvas.
    fn drop_on_canvas(&mut self, kind: ControlKind, parent: &Address, p: Pos2, rects: &FormRects) {
        let Some(region) = rects.free.get(parent).copied() else {
            return;
        };
        let mut c = sized_control(kind);
        c.x = Some(snap(p.x - region.left(), true).max(0.0) as u32);
        c.y = Some(snap(p.y - region.top(), true).max(0.0) as u32);
        let parent = parent.clone();
        self.edit_design(|d| d.insert(&parent, None, c).map(Some));
    }

    /// Adds a control of `kind` from the palette by a click: on the canvas,
    /// under the lowest control. In a multipage report it goes on the band
    /// of the selection, or the detail band, which grows to hold it.
    pub(crate) fn add_to_canvas(&mut self, kind: ControlKind) {
        if let Some(design) = self.calc().report(&self.sheet).cloned()
            && let Some(sections) = design.bands()
        {
            let band = self
                .selected_control()
                .and_then(|at| at.first().copied())
                .unwrap_or(BandKind::Detail.index(sections));
            let lowest = design.layout.controls[band]
                .children
                .iter()
                .map(|c| c.y.unwrap_or(0) + c.height.unwrap_or(20))
                .max();
            let mut c = sized_control(kind);
            c.x = Some(0);
            let y = lowest.map_or(0, |b| snap(b as f32 + 8.0, true).ceil() as u32);
            c.y = Some(y);
            let needed = y + c.height.unwrap_or(20);
            self.edit_design(|d| {
                let group = d.get_mut(&[band])?;
                if group.height.unwrap_or(0) < needed {
                    group.height = Some(needed);
                }
                d.insert(&[band], None, c).map(Some)
            });
            return;
        }
        let bottom = self.report.laid.as_ref().map_or(0.0, |l| {
            l.areas
                .iter()
                .filter(|a| a.at.len() == 1)
                .map(|a| a.rect.bottom())
                .fold(0.0, f32::max)
        });
        let mut c = sized_control(kind);
        c.x = Some(0);
        let gap = if bottom > 0.0 { 8.0 } else { 0.0 };
        c.y = Some(snap(bottom + gap, true).ceil() as u32);
        self.edit_design(|d| d.insert(&[], None, c).map(Some));
    }

    /// Sets the height of the canvas. It is not a change to the controls, so
    /// Undo layout does not take it back.
    fn set_canvas_height(&mut self, height: f32) {
        let name = self.sheet.clone();
        if let Some(mut design) = self.calc().report(&name).cloned() {
            design.height = Some(height.round().max(0.0) as u32);
            let _ = self.session.calc_mut().set_report(&name, design);
        }
    }
}

/// Where something dropped at `p` goes: the innermost Free group there, or
/// the canvas.
fn drop_parent(rects: &FormRects, p: Pos2) -> Address {
    rects
        .free
        .iter()
        .filter(|(_, r)| r.contains(p))
        .max_by_key(|(a, _)| a.len())
        .map(|(a, _)| a.clone())
        .unwrap_or_default()
}
