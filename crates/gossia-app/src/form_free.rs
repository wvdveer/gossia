//! Placing the controls of Free and Grid groups in layout mode (DESIGN.md,
//! "Forms", "The designer for sizes and places"): where the controls were
//! drawn, the snapping of a moved control to the grid and to the others, the
//! cells of a Grid group, and the align tools for several selected controls.

use gossia_i18n::tr;
use std::collections::HashMap;

use egui::{Pos2, Rect, Vec2};
use gossia_core::form::{Address, Anchor};

/// The grid that sizes and places snap to, in points.
pub(crate) const GRID: f32 = 4.0;

/// How close an edge or centre must come to another's to snap to it.
const SNAP_DISTANCE: f32 = 5.0;

/// Where the controls of a form were drawn in layout mode, kept from one
/// frame to the next: each control's box, and the inside of each Free group.
#[derive(Debug, Clone, Default)]
pub(crate) struct FormRects {
    pub controls: HashMap<Address, Rect>,
    pub free: HashMap<Address, Rect>,
    pub grids: HashMap<Address, GridGeom>,
}

/// Where the rows and columns of a Grid group were drawn: each from where to
/// where, on the screen.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GridGeom {
    pub region: Rect,
    pub columns: Vec<(f32, f32)>,
    pub rows: Vec<(f32, f32)>,
}

/// How much room a row or column added past the last one is shown with.
const NEW_TRACK: f32 = 40.0;

impl GridGeom {
    /// The row and column of the cell at `pos`: the nearest, and one past
    /// the last when beyond it, which adds a row or a column.
    pub fn cell_at(&self, pos: Pos2) -> (usize, usize) {
        fn track(tracks: &[(f32, f32)], at: f32) -> usize {
            match tracks.iter().position(|&(_, end)| at < end) {
                Some(i) => i,
                None if tracks.last().is_some_and(|&(_, end)| at < end + NEW_TRACK) => tracks.len(),
                None => tracks.len().saturating_sub(1),
            }
        }
        (track(&self.rows, pos.y), track(&self.columns, pos.x))
    }

    /// Where the cell at a row and a column is, one past the last included.
    pub fn cell_rect(&self, row: usize, column: usize) -> Rect {
        fn span(tracks: &[(f32, f32)], i: usize) -> (f32, f32) {
            match tracks.get(i) {
                Some(&t) => t,
                None => {
                    let end = tracks.last().map_or(0.0, |t| t.1);
                    (end, end + NEW_TRACK)
                }
            }
        }
        let (x0, x1) = span(&self.columns, column);
        let (y0, y1) = span(&self.rows, row);
        Rect::from_min_max(Pos2::new(x0, y0), Pos2::new(x1, y1))
    }
}

fn rects_id(sheet: &str, form: &str) -> egui::Id {
    egui::Id::unique(("form_rects", sheet, form))
}

/// Where the controls of a form were drawn in the frame before.
pub(crate) fn last_rects(ctx: &egui::Context, sheet: &str, form: &str) -> FormRects {
    ctx.data(|d| d.get_temp::<FormRects>(rects_id(sheet, form)))
        .unwrap_or_default()
}

pub(crate) fn keep_rects(ctx: &egui::Context, sheet: &str, form: &str, rects: FormRects) {
    ctx.data_mut(|d| d.insert_temp(rects_id(sheet, form), rects));
}

/// A change to where a control is or how big, as it is seen: a position in
/// points from the top left of the inside of its Free group (which is
/// `size` big), a width and a height, and its anchors. What is `None` is
/// left alone.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Placement {
    pub at: Address,
    pub x: Option<f32>,
    pub y: Option<f32>,
    pub width: Option<f32>,
    pub height: Option<f32>,
    pub anchor_x: Option<Anchor>,
    pub anchor_y: Option<Anchor>,
    /// The size of the inside of its Free group, when it is in one.
    pub size: Option<(f32, f32)>,
    /// In a Grid group: the row and column of its cell, from 0.
    pub cell: Option<(usize, usize)>,
}

/// A line that a moved control snapped to: across (`vertical` false) at a
/// height, or down at a distance from the left, inside its group.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Guide {
    pub vertical: bool,
    pub at: f32,
}

/// Snaps one side of a moved box: its start, centre and end are tried
/// against `targets`, the nearest within reach wins; else the start goes to
/// the grid. Returns the start, and the target it snapped to.
fn snap_axis(start: f32, length: f32, targets: &[f32], grid: bool) -> (f32, Option<f32>) {
    let mut best: Option<(f32, f32, f32)> = None;
    for offset in [0.0, length / 2.0, length] {
        for &target in targets {
            let distance = (target - (start + offset)).abs();
            if distance <= SNAP_DISTANCE && best.is_none_or(|(d, _, _)| distance < d) {
                best = Some((distance, target - offset, target));
            }
        }
    }
    match best {
        Some((_, start, target)) => (start.max(0.0), Some(target)),
        None if grid => (((start / GRID).round() * GRID).max(0.0), None),
        None => (start.round().max(0.0), None),
    }
}

/// Where a control moved to `moved` (inside its group, of `area`) goes:
/// snapped to the edges and centres of the `others` and of the group, else
/// to the grid when `grid`, and the guides of what it snapped to.
pub(crate) fn snap_move(
    moved: Rect,
    others: &[Rect],
    area: Vec2,
    grid: bool,
) -> (Pos2, Vec<Guide>) {
    let mut across = vec![0.0, area.x / 2.0, area.x];
    let mut down = vec![0.0];
    for r in others {
        across.extend([r.left(), r.center().x, r.right()]);
        down.extend([r.top(), r.center().y, r.bottom()]);
    }
    let (x, gx) = snap_axis(moved.left(), moved.width(), &across, grid);
    let (y, gy) = snap_axis(moved.top(), moved.height(), &down, grid);
    let guides = gx
        .map(|at| Guide { vertical: true, at })
        .into_iter()
        .chain(gy.map(|at| Guide {
            vertical: false,
            at,
        }))
        .collect();
    (Pos2::new(x, y), guides)
}

/// The align tools for several selected controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arrange {
    Left,
    Right,
    Top,
    Bottom,
    CentresAcross,
    CentresDown,
    SameWidth,
    SameHeight,
    SpreadAcross,
    SpreadDown,
}

impl Arrange {
    pub const ALL: [Arrange; 10] = [
        Arrange::Left,
        Arrange::Right,
        Arrange::Top,
        Arrange::Bottom,
        Arrange::CentresAcross,
        Arrange::CentresDown,
        Arrange::SameWidth,
        Arrange::SameHeight,
        Arrange::SpreadAcross,
        Arrange::SpreadDown,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Arrange::Left => tr!("Align left"),
            Arrange::Right => tr!("Align right"),
            Arrange::Top => tr!("Align top"),
            Arrange::Bottom => tr!("Align bottom"),
            Arrange::CentresAcross => tr!("Align centres across"),
            Arrange::CentresDown => tr!("Align centres down"),
            Arrange::SameWidth => tr!("Same width"),
            Arrange::SameHeight => tr!("Same height"),
            Arrange::SpreadAcross => tr!("Distribute across"),
            Arrange::SpreadDown => tr!("Distribute down"),
        }
    }

    /// Whether it moves controls, so that they must all be in one Free group,
    /// rather than only sizing them.
    fn moves(self) -> bool {
        !matches!(self, Arrange::SameWidth | Arrange::SameHeight)
    }

    /// How many controls it needs.
    fn least(self) -> usize {
        match self {
            Arrange::SpreadAcross | Arrange::SpreadDown => 3,
            _ => 2,
        }
    }
}

/// Where the boxes go: the first is the one the others follow; distributing
/// keeps the outermost two where they are and makes the gaps between all of
/// them equal.
pub(crate) fn arrange(how: Arrange, rects: &[Rect]) -> Vec<Rect> {
    let Some(first) = rects.first().copied() else {
        return Vec::new();
    };
    let moved = |r: &Rect, x: Option<f32>, y: Option<f32>| {
        Rect::from_min_size(
            Pos2::new(x.unwrap_or(r.left()), y.unwrap_or(r.top())),
            r.size(),
        )
    };
    match how {
        Arrange::Left => rects
            .iter()
            .map(|r| moved(r, Some(first.left()), None))
            .collect(),
        Arrange::Right => rects
            .iter()
            .map(|r| moved(r, Some(first.right() - r.width()), None))
            .collect(),
        Arrange::Top => rects
            .iter()
            .map(|r| moved(r, None, Some(first.top())))
            .collect(),
        Arrange::Bottom => rects
            .iter()
            .map(|r| moved(r, None, Some(first.bottom() - r.height())))
            .collect(),
        Arrange::CentresAcross => rects
            .iter()
            .map(|r| moved(r, Some(first.center().x - r.width() / 2.0), None))
            .collect(),
        Arrange::CentresDown => rects
            .iter()
            .map(|r| moved(r, None, Some(first.center().y - r.height() / 2.0)))
            .collect(),
        Arrange::SameWidth => rects
            .iter()
            .map(|r| Rect::from_min_size(r.min, Vec2::new(first.width(), r.height())))
            .collect(),
        Arrange::SameHeight => rects
            .iter()
            .map(|r| Rect::from_min_size(r.min, Vec2::new(r.width(), first.height())))
            .collect(),
        Arrange::SpreadAcross | Arrange::SpreadDown => {
            let across = how == Arrange::SpreadAcross;
            let start = |r: &Rect| if across { r.left() } else { r.top() };
            let length = |r: &Rect| if across { r.width() } else { r.height() };
            let mut order: Vec<usize> = (0..rects.len()).collect();
            order.sort_by(|&a, &b| start(&rects[a]).total_cmp(&start(&rects[b])));
            let (lo, hi) = (rects[order[0]], rects[order[order.len() - 1]]);
            let span = start(&hi) + length(&hi) - start(&lo);
            let total: f32 = rects.iter().map(length).sum();
            let gap = (span - total) / (rects.len() - 1) as f32;
            let mut out = rects.to_vec();
            let mut at = start(&lo);
            for &i in &order {
                let r = &rects[i];
                out[i] = if across {
                    moved(r, Some(at), None)
                } else {
                    moved(r, None, Some(at))
                };
                at += length(r) + gap;
            }
            out
        }
    }
}

/// The changes an align tool makes to the selected controls (the first is
/// the one the others follow), from where they were drawn; None when it
/// cannot be used: too few controls, one not drawn, or one that it would
/// move not in the same Free group as the others.
pub(crate) fn arranged(
    how: Arrange,
    selected: &[Address],
    rects: &FormRects,
) -> Option<Vec<Placement>> {
    if selected.len() < how.least() {
        return None;
    }
    let boxes: Vec<Rect> = selected
        .iter()
        .map(|at| rects.controls.get(at).copied())
        .collect::<Option<_>>()?;
    let parent = selected[0].split_last()?.1;
    let region = rects.free.get(parent).copied();
    if how.moves()
        && (region.is_none()
            || selected
                .iter()
                .any(|at| at.split_last().map(|(_, p)| p) != Some(parent)))
    {
        return None;
    }
    let after = arrange(how, &boxes);
    let placements = selected
        .iter()
        .zip(boxes.iter().zip(&after))
        .filter(|(_, (before, after))| before != after)
        .map(|(at, (before, after))| {
            let size = at
                .split_last()
                .and_then(|(_, p)| rects.free.get(p))
                .map(|r| (r.width(), r.height()));
            let origin = at
                .split_last()
                .and_then(|(_, p)| rects.free.get(p))
                .map(|r| r.min);
            Placement {
                at: at.clone(),
                x: origin
                    .filter(|_| after.left() != before.left())
                    .map(|o| after.left() - o.x),
                y: origin
                    .filter(|_| after.top() != before.top())
                    .map(|o| after.top() - o.y),
                width: (after.width() != before.width()).then_some(after.width()),
                height: (after.height() != before.height()).then_some(after.height()),
                size,
                ..Placement::default()
            }
        })
        .collect();
    Some(placements)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cell_under_the_pointer_or_a_new_track_past_the_last() {
        let geom = GridGeom {
            region: Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(300.0, 60.0)),
            columns: vec![(0.0, 100.0), (110.0, 300.0)],
            rows: vec![(0.0, 20.0), (30.0, 60.0)],
        };
        assert_eq!(geom.cell_at(Pos2::new(50.0, 10.0)), (0, 0));
        // In the gap: the track after it.
        assert_eq!(geom.cell_at(Pos2::new(105.0, 25.0)), (1, 1));
        // Just past the last: a new row and a new column.
        assert_eq!(geom.cell_at(Pos2::new(320.0, 70.0)), (2, 2));
        // Far past: the last.
        assert_eq!(geom.cell_at(Pos2::new(900.0, 900.0)), (1, 1));
        assert_eq!(
            geom.cell_rect(2, 0),
            Rect::from_min_max(Pos2::new(0.0, 60.0), Pos2::new(100.0, 100.0))
        );
    }

    fn r(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, h))
    }

    #[test]
    fn a_moved_box_snaps_to_the_grid_or_to_an_edge_nearby() {
        let area = Vec2::new(400.0, 300.0);
        // Nothing near: the grid.
        let (at, guides) = snap_move(r(41.0, 63.0, 50.0, 20.0), &[], area, true);
        assert_eq!(at, Pos2::new(40.0, 64.0));
        assert!(guides.is_empty());
        // Without the grid, whole points.
        let (at, _) = snap_move(r(41.3, 62.6, 50.0, 20.0), &[], area, false);
        assert_eq!(at, Pos2::new(41.0, 63.0));
        // The left edge of another, 3 points off; and its bottom to the top
        // of the box.
        let other = r(100.0, 10.0, 80.0, 30.0);
        let (at, guides) = snap_move(r(103.0, 42.0, 50.0, 20.0), &[other], area, true);
        assert_eq!(at, Pos2::new(100.0, 40.0));
        assert_eq!(
            guides,
            vec![
                Guide {
                    vertical: true,
                    at: 100.0
                },
                Guide {
                    vertical: false,
                    at: 40.0
                }
            ]
        );
        // The centre of the group.
        let (at, guides) = snap_move(r(173.0, 200.0, 50.0, 20.0), &[], area, true);
        assert_eq!(at.x, 175.0);
        assert_eq!(
            guides[0],
            Guide {
                vertical: true,
                at: 200.0
            }
        );
    }

    #[test]
    fn aligning_follows_the_first_and_distributing_evens_the_gaps() {
        let boxes = [
            r(10.0, 10.0, 50.0, 20.0),
            r(30.0, 50.0, 80.0, 30.0),
            r(200.0, 90.0, 20.0, 10.0),
        ];
        let left = arrange(Arrange::Left, &boxes);
        assert!(left.iter().all(|b| b.left() == 10.0));
        let right = arrange(Arrange::Right, &boxes);
        assert!(right.iter().all(|b| b.right() == 60.0));
        let centres = arrange(Arrange::CentresDown, &boxes);
        assert!(centres.iter().all(|b| b.center().y == 20.0));
        let wide = arrange(Arrange::SameWidth, &boxes);
        assert!(wide.iter().all(|b| b.width() == 50.0));
        assert_eq!(wide[1].min, boxes[1].min);
        // From 10 to 220: 150 of boxes, so two gaps of 30.
        let spread = arrange(Arrange::SpreadAcross, &boxes);
        assert_eq!(spread[0].left(), 10.0);
        assert_eq!(spread[1].left(), 90.0);
        assert_eq!(spread[2].left(), 200.0);
    }

    #[test]
    fn an_align_tool_needs_one_free_group_to_move_controls() {
        let mut rects = FormRects::default();
        rects.free.insert(vec![0], r(100.0, 100.0, 400.0, 300.0));
        rects
            .controls
            .insert(vec![0, 0], r(110.0, 120.0, 50.0, 20.0));
        rects
            .controls
            .insert(vec![0, 1], r(150.0, 160.0, 80.0, 20.0));
        rects.controls.insert(vec![1], r(10.0, 10.0, 60.0, 20.0));
        let both = vec![vec![0, 0], vec![0, 1]];
        let placed = arranged(Arrange::Left, &both, &rects).unwrap();
        // Only the one that moves, at the first's left inside the group.
        assert_eq!(placed.len(), 1);
        assert_eq!(placed[0].at, vec![0, 1]);
        assert_eq!(placed[0].x, Some(10.0));
        assert_eq!(placed[0].y, None);
        assert_eq!(placed[0].size, Some((400.0, 300.0)));
        // One outside the group: moving is not offered, sizing is.
        let mixed = vec![vec![0, 0], vec![1]];
        assert!(arranged(Arrange::Top, &mixed, &rects).is_none());
        let sized = arranged(Arrange::SameWidth, &mixed, &rects).unwrap();
        assert_eq!(sized[0].width, Some(50.0));
        assert_eq!(sized[0].size, None);
        // Distributing needs three.
        assert!(arranged(Arrange::SpreadAcross, &both, &rects).is_none());
    }
}
