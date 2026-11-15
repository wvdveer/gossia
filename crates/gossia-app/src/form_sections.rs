//! The property pages of a canvas form in layout mode (DESIGN.md, Later
//! versions, "Form canvases, 0.7.0", "The designer"): the form itself with
//! nothing selected, a section, and the properties of a context field.

use gossia_core::form::canvas::{
    ContextType, FORM_WIDTHS, MIN_ROW_HEIGHT, Section, default_row_name, is_context_name,
};
use gossia_core::form::{CONTROL_HEIGHTS, Control, ControlKind, FormDesign};
use gossia_i18n::tr;

use crate::app::GossiaApp;
use crate::form::canvas::{add_repeater_menu, section_title};
use crate::form_layout::{ControlAction, option_text};
use crate::view::error_color;

/// The height a hidden section is given back by *Show section*.
const SHOWN_SECTION: u32 = 48;

impl GossiaApp {
    /// The property page of a canvas form with nothing selected: its width,
    /// and *Add repeater*.
    pub(crate) fn canvas_properties(&mut self, ui: &mut egui::Ui, design: &FormDesign) {
        ui.strong(tr!("Form"));
        let before = design.form_width();
        let mut width = before;
        ui.horizontal(|ui| {
            ui.label(tr!("Width"));
            let response = ui.add(
                egui::DragValue::new(&mut width)
                    .range(FORM_WIDTHS)
                    .suffix(" pt"),
            );
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::SpinButton, true, tr!("Width of the form"))
            });
        });
        if width != before {
            self.edit_design_merging(Some("form width".to_owned()), |d| {
                d.width = Some(width);
                Some(None)
            });
        }
        if let Some(spec) = self.calc().spec(&self.sheet).cloned()
            && let Some(path) = add_repeater_menu(ui, &spec)
        {
            self.add_repeater(&path);
        }
        ui.weak(tr!(
            "The form is a stack of sections: its header, a repeater section for each list it \
             shows, with a section between each two, and its footer. Click the name of a \
             section on the left, or a control, to change it."
        ));
    }

    /// The property page of a section of a canvas form: its height (0
    /// hides a fixed one), and for a repeater section its path, its row
    /// name and *Remove repeater*.
    pub(crate) fn section_properties(
        &mut self,
        ui: &mut egui::Ui,
        design: &FormDesign,
        index: usize,
    ) {
        let (Some(section), Some(control)) = (design.section(index), design.controls.get(index))
        else {
            return;
        };
        let mut edited: Control = control.clone();
        let mut merge = None;
        let key = |property: &str| format!("section {index}:{property}");
        ui.strong(section_title(design, index));
        ui.weak(match section {
            Section::Header => tr!("Drawn once, at the top of the form. Context fields go here."),
            Section::Repeater(_) => tr!(
                "Drawn once for each item of its array, the controls on it being one row. Its \
                 row name is the number of the row, for paths and expressions."
            ),
            Section::Between(_) => tr!("Drawn once, between the repeater sections around it."),
            Section::Footer => tr!("Drawn once, at the bottom of the form."),
        });

        let repeater = section.is_repeater();
        let least = if repeater { MIN_ROW_HEIGHT } else { 0 };
        let mut height = edited.height.unwrap_or(0);
        ui.horizontal(|ui| {
            ui.label(tr!("Height"));
            let response = ui.add(
                egui::DragValue::new(&mut height)
                    .range(least..=*CONTROL_HEIGHTS.end())
                    .suffix(" pt"),
            );
            response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::Role::SpinButton,
                    true,
                    tr!("Height of the section"),
                )
            });
            if !repeater {
                let (label, to) = if height == 0 {
                    (tr!("Show section"), SHOWN_SECTION)
                } else {
                    (tr!("Hide section"), 0)
                };
                if ui
                    .button(label)
                    .on_hover_text(tr!("A hidden section, of height 0, is not drawn."))
                    .clicked()
                {
                    height = to;
                }
            }
        });
        if Some(height) != edited.height {
            edited.height = Some(height);
            merge = Some(key("height"));
        }

        let mut remove = false;
        if let Section::Repeater(k) = section {
            if option_text(ui, tr!("Path"), &mut edited.path, "items", false) {
                merge = Some(key("path"));
            }
            if let Some(spec) = self.calc().spec(&self.sheet).cloned() {
                let arrays = gossia_core::form::bindable(&spec, ControlKind::Repeater);
                ui.menu_button(tr!("Choose an array"), |ui| {
                    for path in arrays {
                        let text = path.to_string();
                        if ui.button(&text).clicked() {
                            edited.path = Some(text);
                            ui.close();
                        }
                    }
                });
            }
            let hint = default_row_name(k);
            if option_text(ui, tr!("Row name"), &mut edited.row_name, &hint, false) {
                merge = Some(key("row_name"));
            }
            if let Some(name) = &edited.row_name
                && !is_context_name(name)
            {
                ui.label(
                    egui::RichText::new(tr!(
                        "{name} is not a name: use letters, digits and _, not starting with a \
                         digit.",
                        name = format!("{name:?}")
                    ))
                    .small()
                    .color(error_color(ui)),
                );
            }
            ui.weak(tr!(
                "Paths on its row are read from its item. Elsewhere its row name is the number \
                 of the current row, as in items[row].name."
            ));
            remove = ui.button(tr!("Remove repeater")).clicked();
            ui.weak(tr!(
                "Removing a repeater takes away the section under it (above it, for the last) \
                 and what is on both."
            ));
        }

        if remove {
            self.apply_control(ControlAction::RemoveRepeater(index));
        } else if &edited != control {
            self.edit_design_merging(merge, |d| {
                *d.get_mut(&[index])? = edited;
                Some(Some(vec![index]))
            });
        }
    }

    /// The properties of a context field: its name, type, default and
    /// whether it is hidden. Returns whether one was changed.
    pub(crate) fn context_properties(&mut self, ui: &mut egui::Ui, edited: &mut Control) -> bool {
        let before = edited.clone();
        if option_text(ui, tr!("Name"), &mut edited.context, "sel", false)
            && edited.context.is_none()
        {
            // Still a context field while its name is being typed.
            edited.context = Some(String::new());
        }
        let mut kind = edited.context_kind();
        ui.horizontal(|ui| {
            ui.label(tr!("Type"));
            let response = egui::ComboBox::from_id_salt("property_context_type")
                .selected_text(kind.label())
                .show_ui(ui, |ui| {
                    for k in ContextType::ALL {
                        ui.selectable_value(&mut kind, k, k.label());
                    }
                })
                .response;
            response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::Role::ComboBox,
                    true,
                    tr!("Type of the context field"),
                )
            });
        });
        if kind != edited.context_kind() {
            edited.context_type = (kind != ContextType::default()).then_some(kind);
        }
        let hint = match kind {
            ContextType::Integer => "0",
            ContextType::Number => "0.0",
            ContextType::Text => "",
            ContextType::Date => "2026-01-31",
            ContextType::Boolean => "false",
        };
        option_text(ui, tr!("Default"), &mut edited.context_default, hint, false);
        if let Err(e) = edited.context_start() {
            ui.label(egui::RichText::new(e).small().color(error_color(ui)));
        }
        ui.checkbox(&mut edited.hidden, tr!("Hidden"))
            .on_hover_text(tr!(
                "Not drawn when the form is used; its value still chooses."
            ));
        ui.weak(tr!(
            "Its value belongs to the form, not to the data. Paths use it by its name, as \
             items[sel].name, and expressions as ctx.sel."
        ));
        *edited != before
    }
}
