//! Comments in the window (DESIGN.md, "Comments in the UI"): a small triangle
//! in the top-right corner of a cell that has a comment, the comment in a
//! popup when the pointer rests on the cell, and *Insert Comment*, *Edit
//! Comment* and *Delete Comment* in the cell's menu, on XML tabs only.

use std::time::Duration;

use cliotree_core::Path;
use egui::{Key, Modifiers, Pos2, Rect, RichText, Shape, Stroke, Vec2};

use crate::app::ClioApp;
use crate::style::Palette;
use crate::view::{Action, error_color, named};

/// The comment being written, while its box is open.
#[derive(Debug, Clone, PartialEq)]
pub struct CommentEdit {
    pub sheet: String,
    pub path: Path,
    pub text: String,
    /// Why the text was refused, shown under it.
    pub error: Option<String>,
    /// Where the box is: beside the cell it was opened from.
    pub at: Pos2,
    /// The box's place on the last frame, so that a click outside it
    /// commits.
    rect: Option<Rect>,
    /// Focus the text on the next frame.
    focus: bool,
}

/// The size of the triangle, in points.
const MARK: f32 = 7.0;

impl ClioApp {
    /// *Insert Comment* or *Edit Comment*: opens the box at `at` with the
    /// comment the node has. Nothing opens on a node that cannot have one.
    pub fn begin_comment(&mut self, path: &Path, at: Pos2) -> bool {
        if let Some(why) = self.calc().comment_refusal(&self.sheet, path) {
            self.notice = Some(why);
            return false;
        }
        let text = self
            .calc()
            .comment(&self.sheet, path)
            .unwrap_or_default()
            .to_owned();
        self.comment_edit = Some(CommentEdit {
            sheet: self.sheet.clone(),
            path: path.clone(),
            text,
            error: None,
            at,
            rect: None,
            focus: true,
        });
        true
    }

    /// Ctrl+Enter or a click away: the comment is set, or the box stays with
    /// the reason. Says whether it was set.
    pub fn commit_comment(&mut self) -> bool {
        let Some(mut edit) = self.comment_edit.take() else {
            return false;
        };
        match self
            .session
            .calc_mut()
            .set_comment(&edit.sheet, &edit.path, Some(&edit.text))
        {
            Ok(()) => {
                self.notice = None;
                true
            }
            Err(e) => {
                edit.error = Some(e.to_string());
                edit.focus = true;
                self.comment_edit = Some(edit);
                false
            }
        }
    }

    /// Esc: the box closes and nothing changes.
    pub fn cancel_comment(&mut self) {
        self.comment_edit = None;
    }

    /// *Delete Comment*.
    pub fn delete_comment(&mut self, path: &Path) {
        let sheet = self.sheet.clone();
        match self.session.calc_mut().set_comment(&sheet, path, None) {
            Ok(()) => self.notice = None,
            Err(e) => self.notice = Some(e.to_string()),
        }
    }

    /// The comment items of a cell's menu: none on a JSON tab; on an XML
    /// tab, *Insert Comment*, or *Edit Comment* and *Delete Comment* when the
    /// node has one. A node that cannot have one says why.
    pub(crate) fn comment_menu_items(
        &self,
        ui: &mut egui::Ui,
        path: &Path,
        at: Pos2,
        actions: &mut Vec<Action>,
    ) {
        let calc = self.calc();
        if !calc.is_xml(&self.sheet) {
            return;
        }
        ui.separator();
        if let Some(why) = calc.comment_refusal(&self.sheet, path) {
            ui.add_enabled(false, egui::Button::new("Insert Comment"))
                .on_disabled_hover_text(why);
            return;
        }
        if calc.comment(&self.sheet, path).is_some() {
            if ui.button("Edit Comment").clicked() {
                actions.push(Action::EditComment(path.clone(), at));
                ui.close();
            }
            if ui.button("Delete Comment").clicked() {
                actions.push(Action::DeleteComment(path.clone()));
                ui.close();
            }
        } else if ui.button("Insert Comment").clicked() {
            actions.push(Action::EditComment(path.clone(), at));
            ui.close();
        }
    }

    /// The triangle of a cell whose node has a comment, and the comment in a
    /// popup once the pointer has rested on the cell for the hover delay. The
    /// popup stays while the pointer is on it, so that a long comment can be
    /// scrolled.
    pub(crate) fn comment_marks(&self, ui: &egui::Ui, cell: Rect, path: &Path) {
        let Some(comment) = self.calc().comment(&self.sheet, path) else {
            return;
        };
        let corner = cell.right_top();
        ui.painter().add(Shape::convex_polygon(
            vec![
                corner,
                corner + Vec2::new(0.0, MARK),
                corner - Vec2::new(MARK, 0.0),
            ],
            Palette::of_ui(ui).warning,
            Stroke::NONE,
        ));
        let id = egui::Id::unique(("comment_popup", &self.sheet, path.to_string()));
        let editing = self
            .comment_edit
            .as_ref()
            .is_some_and(|e| e.sheet == self.sheet && &e.path == path);
        let ctx = ui.ctx();
        let last_popup: Option<Rect> = ctx.data(|d| d.get_temp(id.with("rect")));
        let pointer = ctx.pointer_hover_pos();
        let over_popup = pointer.zip(last_popup).is_some_and(|(p, r)| r.contains(p));
        // Not while a menu is open: the popup would cover it.
        let menu_open = egui::Popup::is_any_open(ctx);
        let hovered = !editing && !menu_open && (ui.rect_contains_pointer(cell) || over_popup);
        if !hovered {
            ctx.data_mut(|d| {
                d.remove::<f64>(id);
                d.remove::<Rect>(id.with("rect"));
            });
            return;
        }
        let now = ui.input(|i| i.time);
        let since = ctx.data_mut(|d| *d.get_temp_mut_or_insert_with(id, || now));
        let display = self.user.prefs.display;
        let delay = display.comment_delay as f64 / 1000.0;
        if now - since < delay {
            ctx.request_repaint_after(Duration::from_secs_f64(delay - (now - since)));
            return;
        }
        let (width, height) = (display.comment_width as f32, display.comment_height as f32);
        let shown = egui::Area::new(id.with("area"))
            .order(egui::Order::Tooltip)
            .fixed_pos(corner + Vec2::new(4.0, 0.0))
            .constrain(true)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_max_width(width);
                    egui::ScrollArea::vertical()
                        .id_salt(id.with("scroll"))
                        .max_height(height)
                        .show(ui, |ui| {
                            ui.add(egui::Label::new(comment).wrap());
                        });
                });
            });
        ctx.data_mut(|d| d.insert_temp(id.with("rect"), shown.response.rect));
    }

    /// The box a comment is written in, while it is open. Enter adds a line
    /// break; Ctrl+Enter or a click outside the box commits; Esc cancels.
    pub(crate) fn comment_window(&mut self, ctx: &egui::Context) {
        let Some(mut edit) = self.comment_edit.take() else {
            return;
        };
        let (commit_key, cancel_key) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::COMMAND, Key::Enter),
                i.consume_key(Modifiers::NONE, Key::Escape),
            )
        });
        let clicked_away = ctx.input(|i| {
            i.pointer.any_pressed()
                && i.pointer
                    .interact_pos()
                    .zip(edit.rect)
                    .is_some_and(|(p, r)| !r.contains(p))
        });
        let place = format!("{}.{}", edit.sheet, edit.path);
        let shown = egui::Area::new(egui::Id::unique("comment_editor"))
            .order(egui::Order::Foreground)
            .fixed_pos(edit.at)
            .constrain(true)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_width(300.0);
                    ui.label(
                        RichText::new(format!("Comment on {}", place.trim_end_matches('.'))).weak(),
                    );
                    let response = ui.add(
                        egui::TextEdit::multiline(&mut edit.text)
                            .desired_rows(4)
                            .desired_width(f32::INFINITY),
                    );
                    named(&response, "Comment");
                    if std::mem::take(&mut edit.focus) {
                        response.request_focus();
                    }
                    if let Some(error) = &edit.error {
                        ui.add(
                            egui::Label::new(RichText::new(error).color(error_color(ui))).wrap(),
                        );
                    }
                    ui.weak("Ctrl+Enter or a click outside to save, Esc to cancel");
                });
            });
        edit.rect = Some(shown.response.rect);
        self.comment_edit = Some(edit);
        if cancel_key {
            self.cancel_comment();
        } else if commit_key || clicked_away {
            self.commit_comment();
        }
    }
}
