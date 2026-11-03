//! The property page of a report (DESIGN.md, Reports, "The report
//! designer"): with nothing selected, the report itself (its page setup and,
//! for a multipage report, its sections); with a band of a multipage report
//! selected, the band (its height, and for a section header or the detail
//! band, its items).
//!
//! The page setup and the options of the sections are not changes to the
//! controls, so Undo layout does not take them back, as with the height of a
//! single page canvas. The bands and their properties are controls, and it
//! does.

use cliotree_calc::ExprKind;
use cliotree_core::form::{CONTROL_HEIGHTS, Control};
use cliotree_core::report::{self, BandKind, Orientation, Paper, ReportDesign, SectionOptions};

use crate::app::ClioApp;
use crate::form_layout::{draft_id, expr_text, expression_box, is_name, option_text};
use crate::view::error_color;

/// The height a band is given when it is shown again.
const SHOWN_BAND: u32 = 24;

impl ClioApp {
    /// The design of the report being shown.
    fn shown_report_design(&self) -> Option<ReportDesign> {
        self.calc().report(&self.sheet).cloned()
    }

    /// Gives the report being shown another design, outside Undo layout.
    fn put_report(&mut self, design: ReportDesign) {
        let name = self.sheet.clone();
        let _ = self.session.calc_mut().set_report(&name, design);
    }

    /// Whether the control at `at` is a band of the multipage report being
    /// shown.
    pub(crate) fn is_band(&self, at: &[usize]) -> bool {
        self.in_report()
            && at.len() == 1
            && self
                .calc()
                .report(&self.sheet)
                .is_some_and(|d| d.bands().is_some())
    }

    /// The property page with nothing selected: the page setup, and the
    /// sections of a multipage report.
    pub(crate) fn report_properties(&mut self, ui: &mut egui::Ui) {
        let Some(design) = self.shown_report_design() else {
            return;
        };
        ui.strong("Report");
        let mut edited = design.clone();
        ui.horizontal(|ui| {
            ui.label("Paper");
            egui::ComboBox::from_id_salt("report_paper")
                .selected_text(edited.page.paper.label())
                .show_ui(ui, |ui| {
                    for paper in Paper::ALL {
                        ui.selectable_value(&mut edited.page.paper, paper, paper.label());
                    }
                })
                .response
                .widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, "Paper"));
        });
        ui.horizontal(|ui| {
            ui.label("Orientation");
            ui.radio_value(
                &mut edited.page.orientation,
                Orientation::Portrait,
                "Portrait",
            );
            ui.radio_value(
                &mut edited.page.orientation,
                Orientation::Landscape,
                "Landscape",
            );
        });
        ui.label("Margins, in points");
        egui::Grid::new("report_margins")
            .num_columns(4)
            .show(ui, |ui| {
                let m = &mut edited.page.margins;
                let rows = [
                    [("Top", &mut m.top), ("Right", &mut m.right)],
                    [("Bottom", &mut m.bottom), ("Left", &mut m.left)],
                ];
                for pair in rows {
                    for (label, value) in pair {
                        ui.label(label);
                        let response = ui.add(egui::DragValue::new(value).range(0..=400));
                        let name = format!("{label} margin");
                        response.widget_info(|| {
                            egui::WidgetInfo::labeled(egui::Role::SpinButton, true, &name)
                        });
                    }
                    ui.end_row();
                }
            });
        ui.weak(match design.kind {
            report::ReportKind::SinglePage => {
                "The canvas is as wide as the paper less its margins; the PDF is one page that wide."
            }
            report::ReportKind::Multipage => {
                "Each page is the paper; the bands are as wide as it less its margins."
            }
        });
        if edited != design {
            self.put_report(edited);
        }
        if let Some(n) = design.bands() {
            ui.separator();
            ui.label(match n {
                0 => "No sections: the detail band repeats for each of its items.".to_owned(),
                1 => "One section around the detail band.".to_owned(),
                n => format!("{n} sections, one inside the other, around the detail band."),
            });
            if ui.button("Add section").clicked() {
                self.add_report_section();
            }
            ui.weak(
                "A new section goes inside the others, around the detail band. Its items are on \
                 its header band.",
            );
        }
        ui.separator();
        ui.weak("Select a control or a band to change it.");
    }

    /// The property page of a band of a multipage report.
    pub(crate) fn band_properties(&mut self, ui: &mut egui::Ui, band: usize) {
        let Some(design) = self.shown_report_design() else {
            return;
        };
        let (Some(kind), Some(control)) =
            (design.band_kind(band), design.layout.controls.get(band))
        else {
            return;
        };
        let at = vec![band];
        let mut edited: Control = control.clone();
        let mut merge = None;
        let key = |property: &str| format!("band {band}:{property}");
        ui.strong(design.band_name(band));
        ui.weak(match kind {
            BandKind::ReportHeader => "Drawn once, at the start of the report.",
            BandKind::ReportFooter => "Drawn once, at the end of the report.",
            BandKind::PageHeader => {
                "Drawn at the top of every page. Its expressions can use page.number and \
                 page.count."
            }
            BandKind::PageFooter => {
                "Drawn at the bottom of every page. Its expressions can use page.number and \
                 page.count."
            }
            BandKind::SectionHeader(_) => {
                "Drawn before each item of the section. The section's items are set here."
            }
            BandKind::SectionFooter(_) => {
                "Drawn after each item of the section. Its items are set on its header band."
            }
            BandKind::Detail => "Drawn once for each of its items.",
        });

        // The height, and hiding the band.
        let mut height = edited.height.unwrap_or(0);
        ui.horizontal(|ui| {
            ui.label("Height");
            let response = ui.add(
                egui::DragValue::new(&mut height)
                    .range(0..=*CONTROL_HEIGHTS.end())
                    .suffix(" pt"),
            );
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::SpinButton, true, "Height of the band")
            });
            let (label, to) = if height == 0 {
                ("Show band", SHOWN_BAND)
            } else {
                ("Hide band", 0)
            };
            if ui
                .button(label)
                .on_hover_text("A hidden band, of height 0, is not drawn.")
                .clicked()
            {
                height = to;
            }
        });
        if Some(height) != edited.height {
            edited.height = Some(height);
            merge = Some(key("height"));
        }

        // The items of a section, on its header band, and of the detail band.
        if matches!(kind, BandKind::SectionHeader(_) | BandKind::Detail) {
            let expr = ExprKind::Items;
            if expr_text(
                ui,
                "Items",
                &mut edited.items,
                "a list, as Orders.items",
                draft_id(&at, None, expr),
            ) {
                merge = Some(key("items"));
            }
            self.expression_error(ui, &at, None, expr);
            if option_text(ui, "Item name", &mut edited.item_name, "item", false) {
                merge = Some(key("item_name"));
            }
            if option_text(ui, "List name", &mut edited.list_name, "", false) {
                merge = Some(key("list_name"));
            }
            for name in [&edited.item_name, &edited.list_name].into_iter().flatten() {
                if !is_name(name.trim()) && !name.trim().is_empty() {
                    ui.label(
                        egui::RichText::new(format!(
                            "{name:?} is not a name: use letters, digits and _, not starting \
                             with a digit."
                        ))
                        .small()
                        .color(error_color(ui)),
                    );
                }
            }
            ui.weak(
                "The bands inside see each item by the item name, and the whole list by the list \
                 name, which the header and footer around can total.",
            );
        }

        // Where a band is not visible it takes no room.
        if !kind.on_every_page() {
            let expr = ExprKind::Visible;
            if expression_box(
                ui,
                expr,
                &mut edited.visible_when,
                draft_id(&at, None, expr),
            ) {
                merge = Some(key("visible"));
            }
            self.expression_error(ui, &at, None, expr);
        }

        if let BandKind::SectionHeader(k) | BandKind::SectionFooter(k) = kind {
            ui.separator();
            let before = design.section_options(k);
            let mut options = before;
            ui.checkbox(&mut options.new_page, "Start each on a new page");
            ui.checkbox(
                &mut options.keep_with_first,
                "Keep the header with the first band",
            );
            ui.checkbox(
                &mut options.repeat_header,
                "Repeat the header on the pages it goes on to",
            );
            if options != before {
                self.set_section_options(k, options);
            }
            if ui.button("Remove section").clicked() {
                self.remove_report_section(k);
                return;
            }
        }
        if kind == BandKind::Detail {
            ui.separator();
            if ui.button("Add section").clicked() {
                self.add_report_section();
                return;
            }
        }

        if &edited != control {
            self.edit_design_merging(merge, |d| {
                *d.get_mut(&at)? = edited;
                Some(Some(at))
            });
        }
    }

    /// Adds a section inside the others, around the detail band, and
    /// selects its header band: one step of Undo layout.
    pub(crate) fn add_report_section(&mut self) {
        self.report.restructure = true;
        self.edit_design(|d| {
            let k = report::add_section(d)?;
            Some(Some(vec![BandKind::SectionHeader(k).index(k + 1)]))
        });
        self.report.restructure = false;
    }

    /// Takes away the section at depth `k`, its header and footer with what
    /// is on them: one step of Undo layout. Its options go too.
    pub(crate) fn remove_report_section(&mut self, k: usize) {
        self.report.restructure = true;
        let removed = self.edit_design(|d| report::remove_section(d, k).then_some(None));
        self.report.restructure = false;
        if removed
            && let Some(mut design) = self.shown_report_design()
            && k < design.sections.len()
        {
            design.sections.remove(k);
            self.put_report(design);
        }
    }

    fn set_section_options(&mut self, k: usize, options: SectionOptions) {
        let Some(mut design) = self.shown_report_design() else {
            return;
        };
        if design.sections.len() <= k {
            design.sections.resize(k + 1, SectionOptions::default());
        }
        design.sections[k] = options;
        while design.sections.last() == Some(&SectionOptions::default()) {
            design.sections.pop();
        }
        self.put_report(design);
    }
}
