//! Moving between the controls of a form with the arrow keys (DESIGN.md,
//! "Forms", "Arrow keys").
//!
//! Up and Down go to the control above or below, the one most nearly in line
//! with it; Left and Right to the control beside it in the same row. In a
//! text box Left and Right move the caret first and leave the box only from
//! its start or its end; Up and Down leave it at once, except in a box of
//! several lines and in a typeahead, where they move in the text and in the
//! list. The table of a repeater is rows of fields, so the keys go from cell
//! to cell. The focus stays in the form: at its edge a key does nothing.

use egui::{Id, Key, Rect};

use crate::app::ClioApp;

/// A control the arrow keys can go to, as drawn in this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NavStop {
    pub id: Id,
    pub rect: Rect,
    /// A text box: Left and Right leave it only from its ends.
    pub text: bool,
    /// Up and Down leave it. Not for a box of several lines, nor a typeahead.
    pub vertical: bool,
}

/// Which way a key goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Way {
    Up,
    Down,
    Left,
    Right,
}

/// Whether two rectangles are in the same row: they overlap down the page by
/// at least half of the lower one.
fn same_row(a: Rect, b: Rect) -> bool {
    let overlap = a.y_range().intersection(b.y_range()).span();
    overlap >= 0.5 * a.height().min(b.height())
}

/// The control the key goes to from `from`, if there is one that way.
pub fn target(stops: &[NavStop], from: Rect, way: Way) -> Option<Id> {
    let others = stops.iter().filter(|s| s.rect != from);
    match way {
        Way::Left | Way::Right => {
            let sign = if way == Way::Right { 1.0 } else { -1.0 };
            others
                .filter(|s| same_row(s.rect, from))
                .map(|s| (s, (s.rect.center().x - from.center().x) * sign))
                .filter(|(_, d)| *d > 1.0)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(s, _)| s.id)
        }
        Way::Up | Way::Down => {
            let sign = if way == Way::Down { 1.0 } else { -1.0 };
            let candidates: Vec<(&NavStop, f32)> = others
                .filter(|s| !same_row(s.rect, from))
                .map(|s| (s, (s.rect.center().y - from.center().y) * sign))
                .filter(|(_, d)| *d > 1.0)
                .collect();
            // The nearest row that way, then the control in it most nearly
            // in line: by the left edges, so that fields under one another
            // and the cells of a column match.
            let nearest = candidates.iter().map(|(_, d)| *d).min_by(f32::total_cmp)?;
            let reach = nearest + 0.5 * from.height().max(1.0);
            candidates
                .iter()
                .filter(|(_, d)| *d <= reach)
                .min_by(|a, b| {
                    let da = (a.0.rect.left() - from.left()).abs();
                    let db = (b.0.rect.left() - from.left()).abs();
                    da.total_cmp(&db)
                })
                .map(|(s, _)| s.id)
        }
    }
}

/// Where the caret of a text box is, if nothing is selected in it.
fn caret(ctx: &egui::Context, id: Id) -> Option<usize> {
    let range = egui::text_edit::TextEditState::load(ctx, id)?
        .cursor
        .char_range()?;
    (range.primary == range.secondary).then(|| range.primary.index.into())
}

impl ClioApp {
    /// After the form is drawn: an arrow key pressed on one of its controls
    /// moves the focus to the next control that way.
    pub(crate) fn form_arrows(&mut self, ctx: &egui::Context, stops: &[NavStop]) {
        let focused = ctx.memory(|m| m.focused());
        let current = focused.and_then(|id| stops.iter().find(|s| s.id == id));
        let Some(current) = current.copied() else {
            self.form.caret = None;
            return;
        };
        let now = current.text.then(|| caret(ctx, current.id)).flatten();
        let before = self
            .form
            .caret
            .filter(|(id, _)| *id == current.id)
            .map(|(_, at)| at);
        self.form.caret = now.map(|at| (current.id, at));
        if self.form.layout.is_some() || egui::Popup::is_any_open(ctx) {
            return;
        }
        let pressed = |key| ctx.input(|i| i.modifiers.is_none() && i.key_pressed(key));
        let way = if pressed(Key::ArrowUp) {
            Way::Up
        } else if pressed(Key::ArrowDown) {
            Way::Down
        } else if pressed(Key::ArrowLeft) {
            Way::Left
        } else if pressed(Key::ArrowRight) {
            Way::Right
        } else {
            return;
        };
        let leaves = match way {
            Way::Up | Way::Down => current.vertical,
            // In a text box, only when the caret could not move: it was at
            // that end already.
            Way::Left | Way::Right => !current.text || (now.is_some() && now == before),
        };
        if !leaves {
            return;
        }
        let next = target(stops, current.rect, way);
        ctx.memory_mut(|m| {
            // egui moves the focus with the arrow keys too, over the whole
            // window; the form keeps it.
            m.move_focus(egui::FocusDirection::None);
            if let Some(next) = next {
                m.request_focus(next);
            }
        });
        if next.is_some() {
            self.form.caret = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stop(n: u64, x: f32, y: f32, w: f32) -> NavStop {
        NavStop {
            id: Id::unique(n),
            rect: Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, 20.0)),
            text: true,
            vertical: true,
        }
    }

    #[test]
    fn the_keys_go_to_the_control_most_in_line_that_way() {
        // Two fields under one another, then a row of three cells, then
        // another row of cells.
        let stops = [
            stop(1, 100.0, 0.0, 300.0),
            stop(2, 100.0, 30.0, 300.0),
            stop(3, 10.0, 80.0, 80.0),
            stop(4, 100.0, 80.0, 80.0),
            stop(5, 190.0, 80.0, 80.0),
            stop(6, 10.0, 110.0, 80.0),
            stop(7, 100.0, 110.0, 80.0),
            stop(8, 190.0, 110.0, 80.0),
        ];
        let at = |n: usize| stops[n - 1].rect;
        let id = |n: u64| Some(Id::unique(n));
        assert_eq!(target(&stops, at(1), Way::Down), id(2));
        assert_eq!(target(&stops, at(2), Way::Up), id(1));
        assert_eq!(target(&stops, at(1), Way::Up), None);
        // Down from a field to the cell under its left edge.
        assert_eq!(target(&stops, at(2), Way::Down), id(4));
        // Within a column of cells, and along a row.
        assert_eq!(target(&stops, at(4), Way::Down), id(7));
        assert_eq!(target(&stops, at(8), Way::Up), id(5));
        assert_eq!(target(&stops, at(7), Way::Left), id(6));
        assert_eq!(target(&stops, at(7), Way::Right), id(8));
        assert_eq!(target(&stops, at(8), Way::Right), None);
        assert_eq!(target(&stops, at(1), Way::Left), None);
    }
}
