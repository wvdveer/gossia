//! The node inspector (DESIGN.md, "Look and feel", "Node inspector"): a
//! panel on the right that says what the selected node is: its path, its type
//! and what the schema allows, where its value comes from, its formula, its
//! state, what is wrong with it, and its comment.
//!
//! What it shows is worked out by [`GossiaApp::inspect`] without a window, as a
//! list of facts, so that it can be tested apart from drawing.

use egui::RichText;
use gossia_core::validate::validate_at;
use gossia_core::{Kind, Path, Range, Segment, StringConstraints, StringFormat, TypeSpec, Value};
use gossia_i18n::tr;

use crate::app::GossiaApp;
use crate::style::Palette;

/// How a fact is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Plain,
    /// A path or a formula, in the monospace font.
    Code,
    /// Something wrong.
    Error,
    /// Something to note: waiting, untrusted, out of date.
    Note,
}

/// One line of the inspector: a label and what it says.
#[derive(Debug, Clone, PartialEq)]
pub struct Fact {
    pub label: &'static str,
    pub text: String,
    pub tone: Tone,
}

fn fact(label: &'static str, text: impl Into<String>, tone: Tone) -> Fact {
    Fact {
        label,
        text: text.into(),
        tone,
    }
}

/// The bounds of a number, in words.
fn range_text(range: &Range) -> Option<String> {
    match (range.min, range.max) {
        (Some(min), Some(max)) => Some(tr!("from {min} to {max}", min = min, max = max)),
        (Some(min), None) => Some(tr!("at least {min}", min = min)),
        (None, Some(max)) => Some(tr!("at most {max}", max = max)),
        (None, None) => None,
    }
}

/// How many of something, in words: `1 to 20 characters`.
fn count_text(min: Option<usize>, max: Option<usize>, what: &str) -> Option<String> {
    match (min, max) {
        (Some(a), Some(b)) if a == b => Some(tr!("exactly {a} {what}", a = a, what = what)),
        (Some(a), Some(b)) => Some(format!("{a} to {b} {what}")),
        (Some(a), None) => Some(tr!("at least {a} {what}", a = a, what = what)),
        (None, Some(b)) => Some(tr!("at most {b} {what}", b = b, what = what)),
        (None, None) => None,
    }
}

fn string_rules(strings: &StringConstraints, out: &mut Vec<String>) {
    out.extend(count_text(
        strings.min_length,
        strings.max_length,
        "characters",
    ));
    if let Some(pattern) = &strings.pattern {
        out.push(format!("matches {}", pattern.source()));
    }
    match strings.format {
        Some(StringFormat::Date) => out.push("a date (2026-10-30)".to_owned()),
        Some(StringFormat::DateTime) => {
            out.push(tr!("a date and time (2026-10-30T14:30:00Z)").to_owned());
        }
        None => {}
    }
}

/// What the schema allows for a type, in words, or `None` if it says nothing
/// more than the type.
pub fn allowed(spec: &TypeSpec, kind: &Kind) -> Option<String> {
    let mut rules = Vec::new();
    match kind {
        Kind::Integer(range) | Kind::Number(range) => rules.extend(range_text(range)),
        Kind::String(strings) => string_rules(strings, &mut rules),
        Kind::Enum { values, strings } => {
            rules.push(tr!("one of {values}", values = values.join(", ")));
            string_rules(strings, &mut rules);
        }
        Kind::Array {
            min_items,
            max_items,
            items,
        } => {
            rules.extend(count_text(*min_items, *max_items, "items"));
            if let Some(more) = allowed(spec, &spec.get(*items).kind) {
                rules.push(tr!("each {more}", more = more));
            }
        }
        _ => {}
    }
    (!rules.is_empty()).then(|| rules.join("; "))
}

/// A container's value in a few words: `{4 fields}`, `[3 items]`.
fn summary(value: &Value) -> String {
    match value {
        Value::Array(items) => match items.len() {
            1 => "[1 item]".to_owned(),
            n => format!("[{n} items]"),
        },
        Value::Object(fields) => match fields.len() {
            1 => "{1 field}".to_owned(),
            n => format!("{{{n} fields}}"),
        },
        other => crate::rows::display(other),
    }
}

impl GossiaApp {
    /// What the inspector says about the node at `path` of the current tab.
    pub fn inspect(&self, path: &Path) -> Vec<Fact> {
        let calc = self.calc();
        let sheet = &self.sheet;
        let mut out = vec![fact(tr!("Path"), self.name_of(path), Tone::Code)];
        let Some(spec) = calc.spec(sheet) else {
            return out;
        };
        let resolved = spec.resolve(path);
        let cell = calc.cell(sheet, path);
        let node = cell.as_ref().and_then(|c| c.node);

        // The type, and what the schema says about it.
        match resolved {
            Some(here) => {
                let def = spec.get(here.ty);
                out.push(fact(tr!("Type"), spec.describe(here.ty), Tone::Plain));
                if let Some(title) = &def.title {
                    out.push(fact(tr!("Title"), title.clone(), Tone::Plain));
                }
                if let Some(description) = &def.description {
                    out.push(fact(tr!("Description"), description.clone(), Tone::Plain));
                }
                if matches!(path.segments().last(), Some(Segment::Key(_)))
                    && let Some(parent) = path.parent()
                    && spec
                        .resolve(&parent)
                        .is_some_and(|p| matches!(spec.get(p.ty).kind, Kind::Object { .. }))
                {
                    let required = if here.required { tr!("Yes") } else { tr!("No") };
                    out.push(fact(tr!("Required"), required, Tone::Plain));
                }
                if let Some(rules) = allowed(spec, &def.kind) {
                    out.push(fact(tr!("Allowed"), rules, Tone::Plain));
                }
            }
            None => out.push(fact(tr!("Type"), tr!("not in the schema"), Tone::Note)),
        }

        // Where the value comes from.
        let formula = cell.as_ref().and_then(|c| c.formula);
        let computed_by = cell.as_ref().and_then(|c| c.computed_by.clone());
        let column = match path.segments() {
            [.., Segment::Index(_), Segment::Key(field)] => {
                let array = Path::from(path.segments()[..path.len() - 2].to_vec());
                calc.column_formula(sheet, &array, field)
            }
            _ => None,
        };
        let (source, shown) = if let Some(text) = column {
            (tr!("Column formula").to_owned(), Some(text))
        } else if let Some(text) = formula {
            (tr!("Formula").to_owned(), Some(text))
        } else if let Some(by) = &computed_by {
            (tr!("Computed by {name}", name = self.name_of(by)), None)
        } else if resolved.is_none() {
            (tr!("Data the schema does not define").to_owned(), None)
        } else if node.is_none() {
            (tr!("Absent").to_owned(), None)
        } else {
            (tr!("Value").to_owned(), None)
        };
        out.push(fact(tr!("Source"), source, Tone::Plain));
        if let Some(text) = shown {
            out.push(fact(tr!("Formula"), format!("={text}"), Tone::Code));
        }

        // The value.
        match node.map(|n| n.value()) {
            Some(Value::Error(error)) => {
                let text = match &error.message {
                    m if m.is_empty() => error.kind.code().to_owned(),
                    m => format!("{} {m}", error.kind.code()),
                };
                out.push(fact(tr!("Error"), text, Tone::Error));
            }
            Some(value) => out.push(fact(tr!("Value"), summary(value), Tone::Plain)),
            None => {}
        }
        if let Some(traceback) = cell.as_ref().and_then(|c| c.traceback) {
            out.push(fact(tr!("Traceback"), traceback, Tone::Code));
        }

        // Its state.
        if let Some(c) = &cell {
            if c.pending {
                out.push(fact(
                    tr!("State"),
                    tr!("Waiting to be calculated"),
                    Tone::Note,
                ));
            }
            if c.stale {
                out.push(fact(
                    tr!("State"),
                    tr!("Out of date: the workbook is not trusted"),
                    Tone::Note,
                ));
            } else if c.untrusted {
                out.push(fact(
                    tr!("State"),
                    tr!("Not run: the workbook is not trusted, so this is the stored value"),
                    Tone::Note,
                ));
            }
        }

        // What is wrong with it: a scalar's own problems, or a required node
        // that is missing.
        match (node, resolved) {
            (Some(node), Some(here)) if !matches!(node.value(), Value::Error(_)) => {
                let scalar = !matches!(
                    spec.get(here.ty).kind,
                    Kind::Object { .. } | Kind::Array { .. } | Kind::Map { .. }
                );
                let issues = validate_at(spec, here.ty, node, path);
                for issue in issues.iter().filter(|i| scalar || &i.path == path) {
                    out.push(fact(tr!("Problem"), issue.message.clone(), Tone::Error));
                }
            }
            (None, Some(here)) if here.required => {
                out.push(fact(
                    tr!("Problem"),
                    tr!("Required, but absent"),
                    Tone::Error,
                ));
            }
            _ => {}
        }

        if let Some(comment) = calc.comment(sheet, path) {
            out.push(fact(tr!("Comment"), comment, Tone::Plain));
        }
        let hidden = self.hidden_children(path).len();
        if hidden > 0 {
            out.push(fact(
                tr!("Hidden"),
                format!("{hidden} hidden inside"),
                Tone::Note,
            ));
        }
        out
    }

    /// The inspector's panel.
    pub(crate) fn inspector_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading(tr!("Inspector"));
        let Some(path) = self.selection.clone() else {
            ui.weak(tr!("Select a node to see what it is."));
            return;
        };
        let facts = self.inspect(&path);
        let palette = Palette::of_ui(ui);
        egui::ScrollArea::vertical()
            .id_salt("inspector")
            .auto_shrink([false, true])
            .show(ui, |ui| {
                egui::Grid::new("inspector_facts")
                    .num_columns(2)
                    .spacing([10.0, 6.0])
                    .show(ui, |ui| {
                        for f in &facts {
                            ui.label(RichText::new(f.label).color(palette.text_weak));
                            let text = match f.tone {
                                Tone::Plain => RichText::new(&f.text),
                                Tone::Code => RichText::new(&f.text).monospace(),
                                Tone::Error => RichText::new(&f.text).color(palette.error),
                                Tone::Note => RichText::new(&f.text).color(palette.warning),
                            };
                            ui.horizontal_wrapped(|ui| {
                                ui.add(egui::Label::new(text).wrap());
                                if f.label == tr!("Path")
                                    && crate::icons::button(
                                        ui,
                                        true,
                                        crate::toolbar::COPY,
                                        "c",
                                        tr!("Copy path"),
                                    )
                                    .clicked()
                                {
                                    ui.ctx().copy_text(f.text.clone());
                                }
                            });
                            ui.end_row();
                        }
                    });
            });
    }
}
