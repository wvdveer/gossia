//! The Immediate window of the code editor (DESIGN.md, "Python modules"), as in
//! the Visual Basic editor: type a Python expression, press Enter, and see its
//! result or its error. It runs on the evaluation thread against the workbook
//! as it is, as a formula on the node selected in the workbook's window would
//! (so `this` is that node), under the formula time limit, and only in a
//! trusted workbook. Nothing it reads becomes a dependency and nothing is
//! stored.
//!
//! Up and Down bring back what was typed before. The history belongs to the
//! user, not the workbook, and is kept in `immediate-history.json`; a result
//! longer than the preferences allow is cut off with a notice.

use egui::{Key, Modifiers};
use gossia_i18n::tr;

use crate::GossiaApp;
use crate::view::{error_color, named};
use gossia_core::Path;

/// The most expressions the window shows; older ones scroll away.
const MAX_SHOWN: usize = 200;

/// The id of the line the expression is typed in.
pub(crate) fn input_id() -> egui::Id {
    egui::Id::unique("immediate_input")
}

/// One expression and what it came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImmediateLine {
    pub text: String,
    /// Where it ran: the tab, and the node if not the root.
    pub place: String,
    /// The number of its answer, while it is waiting for it.
    pub waiting: Option<u64>,
    /// The result, or the error, as shown.
    pub output: Result<String, String>,
}

/// What the Immediate window keeps between frames.
#[derive(Debug, Clone)]
pub struct ImmediateUi {
    /// Whether the window is shown in the code editor.
    pub open: bool,
    /// What is being typed.
    pub input: String,
    pub lines: Vec<ImmediateLine>,
    /// While Up and Down go through the history: the entry shown, and what was
    /// typed before.
    recall: Option<(usize, String)>,
    /// Give the line the keyboard at the next frame.
    focus: bool,
}

impl Default for ImmediateUi {
    fn default() -> ImmediateUi {
        ImmediateUi {
            open: true,
            input: String::new(),
            lines: Vec::new(),
            recall: None,
            focus: false,
        }
    }
}

/// `text` cut to at most `limit` bytes, at a character boundary, with a
/// notice of how much was left out.
pub fn cut_to(text: &str, limit: u64) -> String {
    let limit = usize::try_from(limit).unwrap_or(usize::MAX);
    if text.len() <= limit {
        return text.to_owned();
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{}\n... {} more bytes left out (Tools > Options, Code editor: largest result shown)",
        &text[..end],
        text.len() - end
    )
}

impl GossiaApp {
    /// *View > Immediate window* and Ctrl+G: shows the window and gives its
    /// line the keyboard.
    pub fn show_immediate(&mut self) {
        self.code.immediate.open = true;
        self.code.immediate.focus = true;
    }

    /// Where an expression runs: the tab shown in the workbook's window, at
    /// the node selected there if it still exists, or at its root.
    pub fn immediate_place(&self) -> (String, Path) {
        let sheet = self.sheet.clone();
        let path = self
            .selection
            .clone()
            .filter(|p| {
                self.calc()
                    .data(&sheet)
                    .is_some_and(|data| data.get(p).is_some())
            })
            .unwrap_or_else(Path::root);
        (sheet, path)
    }

    /// Runs what is typed in the Immediate window: it goes in the history,
    /// and its answer comes with a later frame.
    pub fn run_immediate(&mut self, text: &str) {
        let text = text.trim().to_owned();
        if text.is_empty() {
            return;
        }
        let (sheet, path) = self.immediate_place();
        let place = if path.is_root() {
            sheet.clone()
        } else {
            format!("{sheet}.{path}")
        };
        let (waiting, output) = match self.session.run_immediate(&text, &sheet, &path) {
            Ok(id) => (Some(id), Ok(String::new())),
            Err(why) => (None, Err(why)),
        };
        let ui = &mut self.code.immediate;
        ui.lines.push(ImmediateLine {
            text: text.clone(),
            place,
            waiting,
            output,
        });
        let extra = ui.lines.len().saturating_sub(MAX_SHOWN);
        ui.lines.drain(..extra);
        ui.recall = None;
        if let Err(e) = self.user.add_immediate(&text) {
            self.code.notice = Some(tr!("The Immediate window's history: {e}", e = e));
        }
    }

    /// Takes the answers that have come, cut to the length the preferences
    /// allow.
    pub fn collect_immediate(&mut self) {
        let limit = self.user.prefs.code.max_output;
        for answer in self.session.take_immediate() {
            if let Some(line) = self
                .code
                .immediate
                .lines
                .iter_mut()
                .find(|l| l.waiting == Some(answer.id))
            {
                line.waiting = None;
                line.output = match answer.result {
                    Ok(text) => Ok(cut_to(&text, limit)),
                    Err(text) => Err(cut_to(&text, limit)),
                };
            }
        }
    }

    /// Up (`back`) or Down through the history, from what is being typed.
    fn recall(&mut self, back: bool) {
        let history = &self.user.immediate;
        let ui = &mut self.code.immediate;
        if history.is_empty() {
            return;
        }
        let next = match (&ui.recall, back) {
            (None, true) => Some(history.len() - 1),
            (None, false) => None,
            (Some((i, _)), true) => Some(i.saturating_sub(1)),
            (Some((i, _)), false) => (i + 1 < history.len()).then_some(i + 1),
        };
        match next {
            Some(i) => {
                let typed = match ui.recall.take() {
                    Some((_, typed)) => typed,
                    None => ui.input.clone(),
                };
                ui.input = history[i].clone();
                ui.recall = Some((i, typed));
            }
            None => {
                if let Some((_, typed)) = ui.recall.take() {
                    ui.input = typed;
                }
            }
        }
    }

    /// The Immediate window: what was typed and what it came to, and the line
    /// to type in.
    pub(crate) fn immediate_pane(&mut self, ui: &mut egui::Ui) {
        // The panel takes the height of what it draws. The output's height
        // below rests on an estimate of the input line's, and when what was
        // drawn came out shorter the panel shrank a little at every redraw
        // (moving the pointer over it was enough). Claiming the whole height
        // first keeps it at the size the user gave it.
        ui.set_min_height(ui.available_height());
        let trusted = self.calc().is_trusted();
        let (sheet, path) = self.immediate_place();
        let here = if path.is_root() {
            sheet
        } else {
            format!("{sheet}.{path}")
        };
        ui.horizontal(|ui| {
            ui.strong(tr!("Immediate"));
            ui.weak(tr!("this is {here}", here = here));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(
                        !self.code.immediate.lines.is_empty(),
                        egui::Button::new(tr!("Clear")).small(),
                    )
                    .on_hover_text(tr!(
                        "Clears what is shown; the history of what was typed stays."
                    ))
                    .clicked()
                {
                    self.code.immediate.lines.clear();
                }
            });
        });
        let font = egui::FontId::monospace(self.user.prefs.code.font_size as f32);
        let input_height = ui.fonts_mut(|f| f.row_height(&font)) + 12.0;
        let running = self.session.running_immediate().is_some();
        egui::ScrollArea::vertical()
            .id_salt("immediate_output")
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .max_height((ui.available_height() - input_height).max(20.0))
            .show(ui, |ui| {
                for line in &self.code.immediate.lines {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(format!("> {}", line.text))
                                .font(font.clone())
                                .weak(),
                        )
                        .selectable(true),
                    )
                    .on_hover_text(tr!("Run with this as {place}", place = line.place));
                    let (text, color) = match (&line.waiting, &line.output) {
                        (Some(_), _) => ("running...".to_owned(), ui.visuals().weak_text_color()),
                        (None, Ok(text)) => (text.clone(), ui.visuals().text_color()),
                        (None, Err(text)) => (text.clone(), error_color(ui)),
                    };
                    ui.add(
                        egui::Label::new(egui::RichText::new(text).font(font.clone()).color(color))
                            .selectable(true)
                            .wrap(),
                    );
                }
            });
        if !trusted {
            ui.weak(tr!(
                "The Immediate window runs Python, so it works once the workbook is trusted."
            ));
            return;
        }
        let response = ui.add_enabled(
            !running,
            egui::TextEdit::singleline(&mut self.code.immediate.input)
                .id(input_id())
                .font(font)
                .hint_text(tr!("A Python expression, then Enter"))
                .desired_width(f32::INFINITY),
        );
        named(&response, tr!("Immediate expression"));
        if std::mem::take(&mut self.code.immediate.focus) {
            response.request_focus();
        }
        if response.has_focus() {
            let moved = if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowUp)) {
                self.recall(true);
                true
            } else if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowDown)) {
                self.recall(false);
                true
            } else {
                false
            };
            if moved {
                let end = self.code.immediate.input.chars().count();
                crate::code::set_selection(ui.ctx(), input_id(), end, end);
            }
        }
        if response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
            let text = std::mem::take(&mut self.code.immediate.input);
            self.run_immediate(&text);
            self.code.immediate.focus = true;
        }
    }
}
