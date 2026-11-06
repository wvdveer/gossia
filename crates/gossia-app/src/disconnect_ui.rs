//! *Tools > Disconnect from Data Sources* (DESIGN.md, "Data sources",
//! "Disconnect from Data Sources"): every read of a data source in the
//! formulas, the forms and the reports is replaced by what it gives now,
//! after a dialog that says what changes and what cannot.

use gossia_i18n::tr;
use std::time::Duration;

use web_time::Instant;

use gossia_calc::{Disconnect, DisconnectError, SessionRunner};

use crate::app::GossiaApp;

/// How long the command waits for the sources to answer.
const WAIT: Duration = Duration::from_secs(120);

/// "1 read of a data source", "3 reads of data sources".
fn reads(count: usize) -> String {
    counted(
        count,
        tr!("1 read of a data source"),
        tr!("{n} reads of data sources"),
    )
}

impl GossiaApp {
    /// Reads every source the formulas, forms and reports read, waiting for
    /// them, and asks before replacing the reads.
    pub fn begin_disconnect(&mut self) {
        if !self.calc().is_trusted() {
            self.notice = Some(
                tr!(
                    "This workbook is not trusted, so what its data sources give cannot be read. \
                 Trust it first to disconnect it from them."
                )
                .to_owned(),
            );
            return;
        }
        let mut runner = SessionRunner {
            session: &mut self.session,
            deadline: Instant::now() + WAIT,
        };
        match gossia_calc::find_disconnect(&mut runner) {
            Ok(found) => {
                self.notice = None;
                self.disconnect = Some(found);
            }
            Err(DisconnectError::Refused(e)) => {
                self.notice = Some(tr!("Cannot disconnect from the data sources: {e}.", e = e));
            }
            Err(DisconnectError::Timeout) => {
                self.notice = Some(
                    tr!("The data sources did not answer in time, so nothing was changed.")
                        .to_owned(),
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
        gossia_calc::apply_and_rerun(&mut runner, &found, None);
        self.session.tick();
        let mut notice = tr!(
            "Replaced {reads} by what they give now.",
            reads = reads(found.replaced)
        );
        if !found.problems.is_empty() {
            notice.push(' ');
            notice.push_str(&counted(
                found.problems.len(),
                tr!("1 use still needs a data source."),
                tr!("{n} uses still need a data source."),
            ));
        }
        self.notice = Some(notice);
    }

    pub(crate) fn disconnect_window(&mut self, ctx: &egui::Context) {
        let Some(found) = self.disconnect.as_ref() else {
            return;
        };
        let (mut yes, mut no) = (false, false);
        egui::Window::new(tr!("Disconnect from data sources"))
            .collapsible(false)
            .resizable(true)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .default_width(480.0)
            .show(ctx, |ui| {
                ui.label(question(found));
                if !found.problems.is_empty() {
                    ui.add_space(4.0);
                    ui.label(if found.is_empty() {
                        tr!("These uses of a data source cannot be replaced:")
                    } else {
                        tr!("These uses of a data source stay as they are:")
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
                        no = ui.button(tr!("Close")).clicked();
                    } else {
                        yes = ui.button(tr!("Replace")).clicked();
                        no = ui.button(tr!("Cancel")).clicked();
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
            tr!("No formula, form or report reads a data source, so there is nothing to replace.")
                .to_owned()
        } else {
            tr!("Nothing can be replaced.").to_owned()
        };
    }
    let mut changed = Vec::new();
    if found.formulas() > 0 {
        changed.push(counted(
            found.formulas(),
            tr!("1 formula"),
            tr!("{n} formulas"),
        ));
    }
    if found.expressions() > 0 {
        changed.push(counted(
            found.expressions(),
            tr!("1 expression of a form"),
            tr!("{n} expressions of forms"),
        ));
    }
    if found.report_expressions() > 0 {
        changed.push(counted(
            found.report_expressions(),
            tr!("1 expression of a report"),
            tr!("{n} expressions of reports"),
        ));
    }
    let mut text = tr!(
        "{reads} will be replaced by what they give now, as fixed values, so the workbook no longer \
         needs the sources. This changes {changes}.",
        reads = reads(found.replaced),
        changes = and_list(&changed)
    );
    if !found.with_params.is_empty() {
        let reports: Vec<String> = found
            .with_params
            .iter()
            .map(|r| tr!("the report {r}", r = r))
            .collect();
        text.push_str("\n\n");
        text.push_str(&tr!(
            "Reads that use the parameters of {reports} are replaced by what they give with the \
             values chosen now.",
            reports = and_list(&reports)
        ));
    }
    if found.formulas() > 0 {
        text.push_str("\n\n");
        text.push_str(tr!("Edit > Undo takes back the changes to the formulas."));
    }
    let forms = found.expressions() > 0;
    let reports = found.report_expressions() > 0;
    if forms || reports {
        text.push_str("\n\n");
        text.push_str(match (forms, reports) {
            (true, true) => tr!(
                "The changes to the forms and reports cannot be undone: save a copy of the \
                 workbook first if you may want them back."
            ),
            (true, false) => tr!(
                "The changes to the forms cannot be undone: save a copy of the workbook first if \
                 you may want them back."
            ),
            _ => tr!(
                "The changes to the reports cannot be undone: save a copy of the workbook first \
                 if you may want them back."
            ),
        });
    }
    text
}

/// `a`, `a and b`, `a, b and c`.
use gossia_i18n::{and_list, counted};
