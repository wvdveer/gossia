//! *Tools > Disconnect from Data Sources* (DESIGN.md, "Data sources",
//! "Disconnect from Data Sources"): every read of a data source in the
//! formulas, the forms and the reports is replaced by what it gives now,
//! after a dialog that says what changes and what cannot.

use std::time::{Duration, Instant};

use cliotree_calc::{Disconnect, DisconnectError, SessionRunner};

use crate::app::ClioApp;

/// How long the command waits for the sources to answer.
const WAIT: Duration = Duration::from_secs(120);

fn plural(count: usize, one: &str, many: &str) -> String {
    if count == 1 {
        format!("1 {one}")
    } else {
        format!("{count} {many}")
    }
}

impl ClioApp {
    /// Reads every source the formulas, forms and reports read, waiting for
    /// them, and asks before replacing the reads.
    pub fn begin_disconnect(&mut self) {
        if !self.calc().is_trusted() {
            self.notice = Some(
                "This workbook is not trusted, so what its data sources give cannot be read. \
                 Trust it first to disconnect it from them."
                    .to_owned(),
            );
            return;
        }
        let mut runner = SessionRunner {
            session: &mut self.session,
            deadline: Instant::now() + WAIT,
        };
        match cliotree_calc::find_disconnect(&mut runner) {
            Ok(found) => {
                self.notice = None;
                self.disconnect = Some(found);
            }
            Err(DisconnectError::Refused(e)) => {
                self.notice = Some(format!("Cannot disconnect from the data sources: {e}."));
            }
            Err(DisconnectError::Timeout) => {
                self.notice = Some(
                    "The data sources did not answer in time, so nothing was changed.".to_owned(),
                );
            }
        }
    }

    /// *Replace*: the reads go. Undo takes back the formulas. The reports
    /// that changed are calculated again for their results caches.
    pub fn disconnect_yes(&mut self) {
        let Some(found) = self.disconnect.take() else {
            return;
        };
        let mut runner = SessionRunner {
            session: &mut self.session,
            deadline: Instant::now() + WAIT,
        };
        cliotree_calc::apply_and_rerun(&mut runner, &found, None);
        self.session.tick();
        let mut notice = format!(
            "Replaced {} by what they give now.",
            plural(
                found.replaced,
                "read of a data source",
                "reads of data sources"
            )
        );
        if !found.problems.is_empty() {
            notice.push_str(&format!(
                " {} still {} a data source.",
                plural(found.problems.len(), "use", "uses"),
                if found.problems.len() == 1 {
                    "needs"
                } else {
                    "need"
                }
            ));
        }
        self.notice = Some(notice);
    }

    pub(crate) fn disconnect_window(&mut self, ctx: &egui::Context) {
        let Some(found) = self.disconnect.as_ref() else {
            return;
        };
        let (mut yes, mut no) = (false, false);
        egui::Window::new("Disconnect from data sources")
            .collapsible(false)
            .resizable(true)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .default_width(480.0)
            .show(ctx, |ui| {
                ui.label(question(found));
                if !found.problems.is_empty() {
                    ui.add_space(4.0);
                    ui.label(if found.is_empty() {
                        "These uses of a data source cannot be replaced:"
                    } else {
                        "These uses of a data source stay as they are:"
                    });
                    egui::ScrollArea::vertical()
                        .max_height(180.0)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            for problem in &found.problems {
                                ui.label(format!("• {problem}"));
                            }
                        });
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if found.is_empty() {
                        no = ui.button("Close").clicked();
                    } else {
                        yes = ui.button("Replace").clicked();
                        no = ui.button("Cancel").clicked();
                    }
                });
            });
        if yes {
            self.disconnect_yes();
        } else if no {
            self.disconnect = None;
        }
    }
}

/// What the dialog says above the list of problems.
fn question(found: &Disconnect) -> String {
    if found.is_empty() {
        return if found.problems.is_empty() {
            "No formula, form or report reads a data source, so there is nothing to replace."
                .to_owned()
        } else {
            "Nothing can be replaced.".to_owned()
        };
    }
    let mut changed = Vec::new();
    if found.formulas() > 0 {
        changed.push(plural(found.formulas(), "formula", "formulas"));
    }
    if found.expressions() > 0 {
        changed.push(plural(
            found.expressions(),
            "expression of a form",
            "expressions of forms",
        ));
    }
    if found.report_expressions() > 0 {
        changed.push(plural(
            found.report_expressions(),
            "expression of a report",
            "expressions of reports",
        ));
    }
    let mut text = format!(
        "{} will be replaced by what they give now, as fixed values, so the workbook no longer \
         needs the sources. This changes {}.",
        plural(
            found.replaced,
            "read of a data source",
            "reads of data sources"
        ),
        and_list(&changed)
    );
    if !found.with_params.is_empty() {
        let reports: Vec<String> = found
            .with_params
            .iter()
            .map(|r| format!("the report {r}"))
            .collect();
        text.push_str(&format!(
            "\n\nReads that use the parameters of {} are replaced by what they give with the \
             values chosen now.",
            and_list(&reports)
        ));
    }
    if found.formulas() > 0 {
        text.push_str("\n\nEdit > Undo takes back the changes to the formulas.");
    }
    let forms = found.expressions() > 0;
    let reports = found.report_expressions() > 0;
    if forms || reports {
        text.push_str(&format!(
            "\n\nThe changes to the {} cannot be undone: save a copy of the workbook first if \
             you may want them back.",
            match (forms, reports) {
                (true, true) => "forms and reports",
                (true, false) => "forms",
                _ => "reports",
            }
        ));
    }
    text
}

/// `a`, `a and b`, `a, b and c`.
fn and_list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}
