//! The parameters of a report (DESIGN.md, Reports, "Report parameters"):
//! the parameter bar above the report, where the user chooses their values,
//! and the list of parameters on the property page, where they are declared.
//!
//! A value chosen redraws the report at once. It is not an edit: Undo does
//! not take it back. In a workbook that is not trusted the values cannot be
//! changed.

use chrono::NaiveDate;
use cliotree_calc::{ExprKind, ParamState, today};
use cliotree_core::report::{
    ParamKind, PeriodPreset, ReportDesign, ReportParam, period, period_dates,
};
use cliotree_core::{Node, Value};

use crate::app::ClioApp;
use crate::form_layout::{draft_id, expr_text, option_text};
use crate::rows::display;
use crate::view::error_color;

/// A list of choices longer than this gets a search box.
const SEARCH_FROM: usize = 10;

/// What a chosen value is called in the bar.
fn shown(value: &Value, label: Option<&Value>) -> String {
    display(label.unwrap_or(value))
}

/// The label of a choice's value, from its list.
fn label_of<'a>(state: &'a ParamState, value: &Value) -> Option<&'a Value> {
    match &state.choices {
        Some(Ok(list)) => list
            .iter()
            .find(|(v, _)| v == value)
            .and_then(|(_, l)| l.as_ref()),
        _ => None,
    }
}

fn to_jiff(d: NaiveDate) -> jiff::civil::Date {
    use chrono::Datelike;
    jiff::civil::date(d.year() as i16, d.month() as i8, d.day() as i8)
}

fn from_jiff(d: jiff::civil::Date) -> NaiveDate {
    NaiveDate::from_ymd_opt(i32::from(d.year()), d.month() as u32, d.day() as u32)
        .expect("a picked day is a date")
}

/// The key of a draft of the box `part` of the parameter `name`.
fn draft_key(name: &str, part: &str) -> String {
    format!("{name}/{part}")
}

impl ClioApp {
    /// The parameter bar: a control for each parameter of the report being
    /// shown, what was dropped from a choice, and why a default or a list
    /// could not be used.
    pub(crate) fn parameter_bar(&mut self, ui: &mut egui::Ui) {
        let report = self.sheet.clone();
        let Some(design) = self.calc().report(&report).cloned() else {
            return;
        };
        if design.params.is_empty() {
            return;
        }
        let states = self.calc().report_params(&report);
        let trusted = self.calc().is_trusted();
        let mut chosen: Vec<(String, Option<Value>)> = Vec::new();
        ui.horizontal_wrapped(|ui| {
            ui.add_space(crate::report_view::MARGIN);
            ui.add_enabled_ui(trusted, |ui| {
                for (p, state) in design.params.iter().zip(&states) {
                    ui.label(if p.required {
                        format!("{} *", p.shown_name())
                    } else {
                        p.shown_name().to_owned()
                    })
                    .on_hover_text(format!("params.{}", p.name));
                    if let Some(value) = self.param_control(ui, p, state) {
                        chosen.push((p.name.clone(), value));
                    }
                    if state.pending {
                        crate::style::pending_marker(ui).on_hover_text("Waiting to be calculated");
                    }
                    ui.add_space(12.0);
                }
                if states.iter().any(|s| s.chosen)
                    && ui
                        .button("Defaults")
                        .on_hover_text("Every parameter back to its default")
                        .clicked()
                {
                    for p in &design.params {
                        chosen.push((p.name.clone(), None));
                    }
                }
            })
            .response
            .on_disabled_hover_text(
                "This workbook is not trusted, so its parameters cannot be changed.",
            );
        });
        for (p, state) in design.params.iter().zip(&states) {
            let mut notes = Vec::new();
            for gone in &state.dropped {
                notes.push(format!(
                    "{} is no longer in the list of {}, so it was dropped.",
                    display(gone),
                    p.shown_name()
                ));
            }
            if let Some(Err(e)) = &state.choices
                && trusted
            {
                notes.push(format!("{}: the list of choices: {e}", p.shown_name()));
            }
            if let Some(e) = &state.default_error {
                notes.push(format!("{}: the default: {e}", p.shown_name()));
            }
            if let Some((start, end)) = period_dates(&state.value)
                && end < start
            {
                notes.push(format!(
                    "{}: the period ends before it starts.",
                    p.shown_name()
                ));
            }
            for note in notes {
                ui.horizontal(|ui| {
                    ui.add_space(crate::report_view::MARGIN);
                    ui.colored_label(error_color(ui), note);
                });
            }
        }
        for (name, value) in chosen {
            self.session
                .calc_mut()
                .set_report_param(&report, &name, value);
        }
    }

    /// The control of one parameter in the bar. Returns what was chosen:
    /// a value, or None for its default.
    fn param_control(
        &mut self,
        ui: &mut egui::Ui,
        p: &ReportParam,
        state: &ParamState,
    ) -> Option<Option<Value>> {
        let name = p.shown_name().to_owned();
        match p.kind {
            ParamKind::Date => self
                .date_box(ui, &p.name, "date", &name, &state.value)
                .map(Some),
            ParamKind::Period => self.period_control(ui, p, state).map(Some),
            ParamKind::Choice => {
                let mut picked = None;
                let text = match &state.value {
                    Value::Null => "(none)".to_owned(),
                    v => shown(v, label_of(state, v)),
                };
                egui::ComboBox::from_id_salt(("param", p.name.as_str()))
                    .selected_text(text)
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_label(state.value == Value::Null, "(none)")
                            .clicked()
                        {
                            picked = Some(Some(Value::Null));
                        }
                        if let Some(Ok(list)) = &state.choices {
                            for (value, label) in list {
                                let on = &state.value == value;
                                if ui
                                    .selectable_label(on, shown(value, label.as_ref()))
                                    .clicked()
                                {
                                    picked = Some(Some(value.clone()));
                                }
                            }
                        }
                    })
                    .response
                    .widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, &name));
                picked
            }
            ParamKind::Choices => self.choices_menu(ui, p, state),
            ParamKind::Number => {
                let current = match &state.value {
                    Value::Null => String::new(),
                    v => display(v),
                };
                let typed = self.param_text(ui, &p.name, "number", &name, &current, 80.0)?;
                let t = typed.trim();
                let value = if t.is_empty() {
                    Value::Null
                } else if let Ok(n) = t.parse::<i64>() {
                    Value::Int(n)
                } else if let Ok(x) = t.parse::<f64>() {
                    Value::Float(x)
                } else {
                    return None;
                };
                Some(Some(value))
            }
            ParamKind::Text => {
                let current = match &state.value {
                    Value::Str(s) => s.clone(),
                    _ => String::new(),
                };
                let typed = self.param_text(ui, &p.name, "text", &name, &current, 140.0)?;
                Some(Some(Value::Str(typed)))
            }
            ParamKind::YesNo => {
                let mut on = state.value == Value::Bool(true);
                let r = ui.checkbox(&mut on, "");
                r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::CheckBox, true, &name));
                r.changed().then_some(Some(Value::Bool(on)))
            }
        }
    }

    /// A box of text in the bar. What is typed is chosen as it is typed;
    /// the box keeps it while it has the focus. Returns the text when it
    /// changed.
    fn param_text(
        &mut self,
        ui: &mut egui::Ui,
        param: &str,
        part: &str,
        name: &str,
        current: &str,
        width: f32,
    ) -> Option<String> {
        let key = draft_key(param, part);
        let mut text = self
            .report
            .param_drafts
            .get(&key)
            .cloned()
            .unwrap_or_else(|| current.to_owned());
        let r = ui.add(egui::TextEdit::singleline(&mut text).desired_width(width));
        let label = name.to_owned();
        r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, &label));
        if r.has_focus() {
            self.report.param_drafts.insert(key, text.clone());
        } else {
            self.report.param_drafts.remove(&key);
        }
        r.changed().then_some(text)
    }

    /// A date in the bar: a box to type it in, and a calendar. Returns the
    /// date chosen, or None (`Value::Null`) for a box emptied.
    fn date_box(
        &mut self,
        ui: &mut egui::Ui,
        param: &str,
        part: &str,
        name: &str,
        value: &Value,
    ) -> Option<Value> {
        let day = match value {
            Value::Date(d) => Some(*d),
            _ => None,
        };
        let current = day
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_default();
        let mut out = None;
        if let Some(typed) = self.param_text(ui, param, part, name, &current, 86.0) {
            let t = typed.trim();
            if t.is_empty() {
                out = Some(Value::Null);
            } else if let Ok(d) = t.parse::<NaiveDate>() {
                out = Some(Value::Date(d));
            }
        }
        let mut picked = to_jiff(day.unwrap_or_else(today));
        let before = picked;
        let r = ui.add(
            egui_extras::DatePickerButton::new(&mut picked)
                .id_salt(&format!("param_date {param} {part}"))
                .format(crate::form::CALENDAR)
                .show_icon(false)
                .combo_boxes(false)
                .calendar_week(false)
                .highlight_weekends(false),
        );
        let label = format!("Pick a date for {name}");
        r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
        if picked != before || (day.is_none() && r.changed()) {
            self.report.param_drafts.remove(&draft_key(param, part));
            out = Some(Value::Date(from_jiff(picked)));
        }
        out
    }

    /// A period in the bar: its presets, and for any two dates, a start and
    /// an end.
    fn period_control(
        &mut self,
        ui: &mut egui::Ui,
        p: &ReportParam,
        state: &ParamState,
    ) -> Option<Value> {
        let today = today();
        let dates = period_dates(&state.value);
        let preset = dates.and_then(|(s, e)| PeriodPreset::of(s, e, today));
        let custom_id = ui.make_persistent_id(("param_custom", p.name.as_str()));
        let mut custom = ui.data(|d| d.get_temp::<bool>(custom_id)).unwrap_or(false)
            || (dates.is_some() && preset.is_none());
        let mut out = None;
        let name = p.shown_name().to_owned();
        let text = match (preset, dates) {
            _ if custom => "Dates".to_owned(),
            (Some(p), _) => p.label().to_owned(),
            _ => "(none)".to_owned(),
        };
        egui::ComboBox::from_id_salt(("param", p.name.as_str()))
            .selected_text(text)
            .show_ui(ui, |ui| {
                for each in PeriodPreset::ALL {
                    if ui
                        .selectable_label(preset == Some(each) && !custom, each.label())
                        .clicked()
                    {
                        let (s, e) = each.dates(today);
                        out = Some(period(s, e));
                        custom = false;
                    }
                }
                if ui.selectable_label(custom, "Dates").clicked() {
                    custom = true;
                }
                if !p.required
                    && ui
                        .selectable_label(dates.is_none() && !custom, "(none)")
                        .clicked()
                {
                    out = Some(Value::Null);
                    custom = false;
                }
            })
            .response
            .widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, &name));
        ui.data_mut(|d| d.insert_temp(custom_id, custom));
        if custom {
            let (start, end) = dates.unwrap_or_else(|| PeriodPreset::ThisMonth.dates(today));
            let from = self.date_box(
                ui,
                &p.name,
                "start",
                &format!("{name} from"),
                &Value::Date(start),
            );
            ui.label("to");
            let to = self.date_box(ui, &p.name, "end", &format!("{name} to"), &Value::Date(end));
            if let Some(Value::Date(s)) = from {
                out = Some(period(s, end));
            }
            if let Some(Value::Date(e)) = to {
                out = Some(period(start, e));
            }
            if dates.is_none() && out.is_none() {
                out = Some(period(start, end));
            }
        }
        out
    }

    /// A subset of a list in the bar: a menu of check boxes, with *Select
    /// all*, *Clear* and, for a long list, a search box.
    fn choices_menu(
        &mut self,
        ui: &mut egui::Ui,
        p: &ReportParam,
        state: &ParamState,
    ) -> Option<Option<Value>> {
        let list: Vec<(Value, Option<Value>)> = match &state.choices {
            Some(Ok(list)) => list.clone(),
            _ => Vec::new(),
        };
        let picked: Vec<Value> = match &state.value {
            Value::Array(items) => items.iter().map(|n| n.value().clone()).collect(),
            _ => Vec::new(),
        };
        let text = match picked.len() {
            0 => "(none)".to_owned(),
            n if n == list.len() && n > 1 => "All".to_owned(),
            n if n <= 2 => picked
                .iter()
                .map(|v| shown(v, label_of(state, v)))
                .collect::<Vec<_>>()
                .join(", "),
            n => format!("{n} of {}", list.len()),
        };
        let mut out = None;
        let search_key = draft_key(&p.name, "search");
        let name = p.shown_name().to_owned();
        let r = ui.menu_button(text, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Select all").clicked() {
                    out = Some(list.iter().map(|(v, _)| v.clone()).collect::<Vec<_>>());
                }
                if ui.button("Clear").clicked() {
                    out = Some(Vec::new());
                }
            });
            let mut search = self
                .report
                .param_drafts
                .get(&search_key)
                .cloned()
                .unwrap_or_default();
            if list.len() > SEARCH_FROM {
                let r = ui.add(egui::TextEdit::singleline(&mut search).hint_text("Search"));
                r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, "Search"));
                self.report
                    .param_drafts
                    .insert(search_key.clone(), search.clone());
            }
            let needle = search.trim().to_lowercase();
            egui::ScrollArea::vertical()
                .max_height(320.0)
                .show(ui, |ui| {
                    for (value, label) in &list {
                        let text = shown(value, label.as_ref());
                        if !needle.is_empty() && !text.to_lowercase().contains(&needle) {
                            continue;
                        }
                        let mut on = picked.contains(value);
                        if ui.checkbox(&mut on, text).changed() {
                            let mut now: Vec<Value> = list
                                .iter()
                                .map(|(v, _)| v)
                                .filter(|v| if *v == value { on } else { picked.contains(v) })
                                .cloned()
                                .collect();
                            now.dedup();
                            out = Some(now);
                        }
                    }
                });
        });
        r.response
            .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &name));
        out.map(|values| {
            Some(if values.is_empty() {
                Value::Null
            } else {
                Value::Array(values.into_iter().map(Node::new).collect())
            })
        })
    }

    /// The parameters on the property page, with nothing selected: each
    /// one's name, label, kind, default, list and whether it is required,
    /// and *Add parameter*. They are not controls, so Undo layout does not
    /// take a change back.
    pub(crate) fn params_properties(&mut self, ui: &mut egui::Ui, design: &ReportDesign) {
        let report = self.sheet.clone();
        let mut edited = design.clone();
        let states = self.calc().report_params(&report);
        let problems = design.problems();
        ui.strong("Parameters");
        if design.params.is_empty() {
            ui.weak("None: the report shows the same for everyone.");
        }
        let mut remove = None;
        let mut swap = None;
        let count = edited.params.len();
        for (i, p) in edited.params.iter_mut().enumerate() {
            let id = ui.make_persistent_id(("report_param", i));
            egui::CollapsingHeader::new(format!("{} ({})", p.shown_name(), p.kind.label()))
                .id_salt(id)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Name");
                        let r =
                            ui.add(egui::TextEdit::singleline(&mut p.name).desired_width(140.0));
                        r.widget_info(|| {
                            egui::WidgetInfo::labeled(egui::Role::TextInput, true, "Parameter name")
                        });
                    });
                    ui.weak(format!("Expressions use it as params.{}", p.name.trim()));
                    option_text(ui, "Label", &mut p.label, p.name.trim(), false);
                    ui.horizontal(|ui| {
                        ui.label("Kind");
                        egui::ComboBox::from_id_salt(("param_kind", i))
                            .selected_text(p.kind.label())
                            .show_ui(ui, |ui| {
                                for kind in ParamKind::ALL {
                                    ui.selectable_value(&mut p.kind, kind, kind.label());
                                }
                            })
                            .response
                            .widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::Role::ComboBox,
                                    true,
                                    "Parameter kind",
                                )
                            });
                    });
                    if p.kind == ParamKind::Period {
                        ui.horizontal(|ui| {
                            ui.label("Default");
                            egui::ComboBox::from_id_salt(("param_preset", i))
                                .selected_text(p.preset.map_or("(none)", PeriodPreset::label))
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut p.preset, None, "(none)");
                                    for preset in PeriodPreset::ALL {
                                        ui.selectable_value(
                                            &mut p.preset,
                                            Some(preset),
                                            preset.label(),
                                        );
                                    }
                                })
                                .response
                                .widget_info(|| {
                                    egui::WidgetInfo::labeled(
                                        egui::Role::ComboBox,
                                        true,
                                        "Default period",
                                    )
                                });
                        });
                    } else {
                        let hint = match p.kind {
                            ParamKind::Date => "datetime.date.today()",
                            ParamKind::Choices => "a list, as ['North']",
                            ParamKind::YesNo => "True",
                            _ => "",
                        };
                        expr_text(
                            ui,
                            "Default",
                            &mut p.default,
                            hint,
                            draft_id(&[], Some(&format!("param {i}")), ExprKind::Value),
                        );
                    }
                    if p.kind.has_choices() {
                        expr_text(
                            ui,
                            "Choices",
                            &mut p.choices,
                            "a list, as [u.name for u in OrgUnits.items]",
                            draft_id(&[], Some(&format!("param {i}")), ExprKind::Choices),
                        );
                        ui.weak(
                            "Each item is a value, or a (value, label) pair to show something \
                             else.",
                        );
                    }
                    ui.checkbox(&mut p.required, "Required")
                        .on_hover_text("While it is empty the report is not drawn");
                    let state = states.iter().find(|s| s.name == p.name);
                    let mut errors: Vec<String> = Vec::new();
                    if let Some(e) = state.and_then(|s| s.default_error.as_ref()) {
                        errors.push(format!("Default: {e}"));
                    }
                    if let Some(Some(Err(e))) = state.map(|s| &s.choices) {
                        errors.push(format!("Choices: {e}"));
                    }
                    for (_, why) in &problems {
                        if why.contains(&format!(" {} ", p.name.trim()))
                            || why.contains(&format!("{:?}", p.name.trim()))
                        {
                            errors.push(why.clone());
                        }
                    }
                    for e in errors {
                        ui.label(egui::RichText::new(e).small().color(error_color(ui)));
                    }
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(i > 0, egui::Button::new("Move up"))
                            .clicked()
                        {
                            swap = Some((i - 1, i));
                        }
                        if ui
                            .add_enabled(i + 1 < count, egui::Button::new("Move down"))
                            .clicked()
                        {
                            swap = Some((i, i + 1));
                        }
                        if ui.button("Remove parameter").clicked() {
                            remove = Some(i);
                        }
                    });
                });
        }
        if let Some((a, b)) = swap {
            edited.params.swap(a, b);
        }
        if let Some(i) = remove {
            edited.params.remove(i);
        }
        if ui.button("Add parameter").clicked() {
            let name = (1..)
                .map(|n| format!("param{n}"))
                .find(|n| !edited.params.iter().any(|p| &p.name == n))
                .expect("some number is free");
            edited.params.push(ReportParam::new(&name, ParamKind::Date));
        }
        ui.weak("Expressions see the values as params.<name>. They are chosen in the bar above the report.");
        if edited != *design {
            let _ = self.session.calc_mut().set_report(&report, edited);
        }
    }
}

/// Names joined as a sentence has them: "A", "A and B", "A, B and C".
pub(crate) fn and_list(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}
