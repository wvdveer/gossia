//! The question asked when a formula would erase what is inside its node
//! (DESIGN.md, "A formula over what is inside"): *Yes* enters the formula and
//! erases the values and formulas inside, *No* keeps them and drops the
//! formula.

use cliotree_calc::{Detail, Loc, entry, entry::Entered};
use cliotree_core::Path;

use crate::app::ClioApp;

/// How many of the formulas inside the question names.
const NAMED: usize = 3;

/// A formula waiting for the answer.
#[derive(Debug, Clone)]
pub struct ReplaceInside {
    pub sheet: String,
    pub target: Target,
    /// What was typed, with its `=`.
    pub text: String,
    /// What the formula would erase.
    pub inside: Detail,
    /// The formula came from the form designer's formula box, not the cell.
    pub from_form: bool,
}

/// Where the formula goes.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    Node(Path),
    /// The field of every item of the array: a column formula.
    Column(Path, String),
}

fn plural(count: usize, one: &str) -> String {
    if count == 1 {
        format!("1 {one}")
    } else {
        format!("{count} {one}s")
    }
}

impl ReplaceInside {
    /// The node the formula goes on, as the tab names it.
    pub fn place(&self) -> String {
        match &self.target {
            Target::Node(path) => Loc::new(self.sheet.clone(), path.clone()).to_string(),
            Target::Column(array, field) => {
                format!("{}[].{field}", Loc::new(self.sheet.clone(), array.clone()))
            }
        }
    }

    /// What the dialog asks.
    pub fn question(&self) -> String {
        let place = self.place();
        let mut parts = Vec::new();
        if self.inside.values > 0 {
            parts.push(plural(self.inside.values, "value"));
        }
        let formulas = &self.inside.formulas;
        if !formulas.is_empty() {
            let mut names: Vec<String> = formulas
                .iter()
                .take(NAMED)
                .map(|p| Loc::new(self.sheet.clone(), p.clone()).to_string())
                .collect();
            if formulas.len() > NAMED {
                names.push(format!("{} more", formulas.len() - NAMED));
            }
            parts.push(format!(
                "{} ({})",
                plural(formulas.len(), "formula"),
                names.join(", ")
            ));
        }
        let what = parts.join(" and ");
        let column = matches!(self.target, Target::Column(..));
        let holds = if column { "The items hold" } else { "It holds" };
        format!(
            "A formula on {place} computes everything {}, so what is there now would be \
             erased. {holds} {what}.\n\n\
             Yes enters the formula and erases them. \
             No keeps them and drops the formula.",
            if column { "in it" } else { "inside it" }
        )
    }
}

impl ClioApp {
    /// What a formula typed for `target` would erase, or `None` when the text
    /// is not a formula or nothing would be lost.
    pub(crate) fn would_erase(&self, sheet: &str, target: &Target, text: &str) -> Option<Detail> {
        let spec = self.calc().spec(sheet)?;
        let at = match target {
            Target::Node(path) => path.clone(),
            Target::Column(array, field) => array.index(0).key(field.clone()),
        };
        if !matches!(entry::classify(spec, &at, text), Ok(Entered::Formula(_))) {
            return None;
        }
        let detail = match target {
            Target::Node(path) => self.calc().detail_inside(sheet, path),
            Target::Column(array, field) => self.calc().detail_inside_column(sheet, array, field),
        };
        (!detail.is_empty()).then_some(detail)
    }

    /// Asks about the formula before entering it, when it would erase what is
    /// inside its node. Returns whether it asked: the caller then waits.
    pub(crate) fn ask_before_erasing(
        &mut self,
        sheet: &str,
        target: Target,
        text: &str,
        from_form: bool,
    ) -> bool {
        let Some(inside) = self.would_erase(sheet, &target, text) else {
            return false;
        };
        self.notice = None;
        self.replace_inside = Some(ReplaceInside {
            sheet: sheet.to_owned(),
            target,
            text: text.to_owned(),
            inside,
            from_form,
        });
        true
    }

    /// *Yes*: the formula goes on and what was inside goes, as one entry that
    /// undo takes back whole.
    pub fn replace_inside_yes(&mut self) {
        let Some(ask) = self.replace_inside.take() else {
            return;
        };
        let calc = self.session.calc_mut();
        let result = match &ask.target {
            Target::Node(path) => entry::enter_replacing(calc, &ask.sheet, path, &ask.text),
            Target::Column(array, field) => {
                entry::enter_column_replacing(calc, &ask.sheet, array, field, &ask.text)
            }
        };
        match result {
            Ok(()) => {
                self.notice = None;
                if ask.from_form {
                    self.clear_form_formula();
                } else {
                    self.editing = None;
                    self.completion = Default::default();
                    if let Target::Node(path) = &ask.target {
                        self.pin_row_of(path);
                    }
                    self.note_unevaluated_formula(&ask.text);
                }
                self.session.tick();
            }
            Err(e) => self.notice = Some(e.to_string()),
        }
    }

    /// *No*: what is inside stays and the formula is dropped.
    pub fn replace_inside_no(&mut self) {
        let Some(ask) = self.replace_inside.take() else {
            return;
        };
        if ask.from_form {
            self.clear_form_formula();
        } else {
            self.cancel_editing();
        }
    }

    pub(crate) fn replace_inside_window(&mut self, ctx: &egui::Context) {
        let Some(ask) = self.replace_inside.clone() else {
            return;
        };
        let (mut yes, mut no) = (false, false);
        egui::Window::new("Erase what is inside?")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .default_width(420.0)
            .show(ctx, |ui| {
                ui.label(ask.question());
                ui.label("Edit > Undo takes back a Yes.");
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    yes = ui.button("Yes").clicked();
                    no = ui.button("No").clicked();
                });
            });
        if yes {
            self.replace_inside_yes();
        } else if no {
            self.replace_inside_no();
        }
    }
}
