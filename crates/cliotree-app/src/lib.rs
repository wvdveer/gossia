//! The ClioTree desktop application, as a library so that its user interface
//! can be driven by tests.

pub mod app;
pub mod app_icon;
pub mod chart_view;
pub mod clipboard;
#[cfg(test)]
mod clipboard_tests;
pub mod code;
pub mod code_check;
pub mod code_text;
#[cfg(test)]
mod code_text_tests;
pub mod comments;
pub mod control;
#[cfg(feature = "test-control")]
pub mod control_server;
pub mod disconnect_ui;
pub mod edit;
#[cfg(test)]
mod edit_tests;
pub mod export;
pub mod file;
pub mod form;
pub mod form_exprs;
pub mod form_free;
pub mod form_layout;
#[cfg(test)]
mod form_layout_tests;
pub mod form_nav;
pub mod icons;
pub mod immediate;
pub mod import;
pub mod inspector;
pub mod map_keys;
pub mod options;
pub mod pick;
pub mod prefs;
pub mod raw;
pub mod replace_inside;
pub mod report_design;
pub mod report_params;
pub mod report_props;
pub mod report_tab;
#[cfg(test)]
mod report_tab_tests;
pub mod report_view;
pub mod rows;
pub mod sample;
pub mod schema;
#[cfg(test)]
mod schema_tests;
pub mod schema_view;
pub mod sources_ui;
pub mod status;
pub mod style;
pub mod tab_commands;
pub mod tab_rename;
pub mod table;
pub mod table_view;
pub mod tables;
#[cfg(test)]
mod tables_tests;
pub mod toolbar;
pub mod tree_hidden;
pub mod trust_ui;
pub mod view;
pub mod view_state;
pub mod workbook_file;

pub use app::ClioApp;

impl eframe::App for ClioApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        log_window(ui.ctx());
        keep_size_across_monitors(ui.ctx());
        self.show(ui);
    }
}

/// Windows grows a window each time it is dragged between monitors of
/// different scale (observed with 125% and 100%: about 1.25 times larger after
/// each round trip). So the size, in points, is remembered from before a change
/// of scale, and for a second after the change the window is put back to it if
/// it has moved away. Not applied to a window that is maximised or full screen.
fn keep_size_across_monitors(ctx: &egui::Context) {
    use std::cell::RefCell;

    #[derive(Default)]
    struct Guard {
        scale: Option<f32>,
        size: Option<egui::Vec2>,
        /// The size to keep, and the time until which to keep it.
        pinned: Option<(egui::Vec2, f64)>,
    }
    thread_local! {
        static GUARD: RefCell<Guard> = RefCell::new(Guard::default());
    }
    const HOLD: f64 = 1.0;
    const TOLERANCE: f32 = 1.5;
    let (scale, size, now, fixed) = ctx.input(|i| {
        let viewport = i.viewport();
        (
            viewport
                .native_pixels_per_point
                .unwrap_or(i.pixels_per_point()),
            viewport.inner_rect.map(|r| r.size()),
            i.time,
            viewport.maximized == Some(true) || viewport.fullscreen == Some(true),
        )
    });
    let Some(size) = size else {
        return;
    };
    GUARD.with(|guard| {
        let mut guard = guard.borrow_mut();
        if fixed {
            guard.pinned = None;
            guard.size = Some(size);
            guard.scale = Some(scale);
            return;
        }
        if guard
            .scale
            .is_some_and(|before| (before - scale).abs() > 0.001)
            && let Some(before) = guard.size
        {
            guard.pinned = Some((before, now + HOLD));
        }
        guard.scale = Some(scale);
        match guard.pinned {
            Some((keep, until)) if now < until => {
                let off =
                    (size.x - keep.x).abs() > TOLERANCE || (size.y - keep.y).abs() > TOLERANCE;
                if off {
                    if std::env::var_os("CLIOTREE_LOG_WINDOW").is_some() {
                        eprintln!("window: put back to {:.0}x{:.0} points", keep.x, keep.y);
                    }
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(keep));
                }
                ctx.request_repaint();
            }
            _ => {
                guard.pinned = None;
                guard.size = Some(size);
            }
        }
    });
}

/// With `CLIOTREE_LOG_WINDOW` set, prints the size and scale of the window to
/// standard error each time they change, for finding out what a window does
/// when it moves between monitors.
fn log_window(ctx: &egui::Context) {
    use std::cell::RefCell;
    use std::sync::OnceLock;
    static ON: OnceLock<bool> = OnceLock::new();
    if !*ON.get_or_init(|| std::env::var_os("CLIOTREE_LOG_WINDOW").is_some()) {
        return;
    }
    thread_local! {
        static LAST: RefCell<String> = const { RefCell::new(String::new()) };
    }
    let line = ctx.input(|i| {
        let viewport = i.viewport();
        let size = |r: Option<egui::Rect>| {
            r.map_or("-".to_owned(), |r| {
                format!("{:.0}x{:.0}", r.width(), r.height())
            })
        };
        format!(
            "scale {:.3}, inner {} points, outer {} points, monitor {:?} points, native scale {:?}",
            i.pixels_per_point(),
            size(viewport.inner_rect),
            size(viewport.outer_rect),
            viewport.monitor_size.map(|m| (m.x.round(), m.y.round())),
            viewport.native_pixels_per_point,
        )
    });
    LAST.with(|last| {
        if *last.borrow() != line {
            eprintln!("window: {line}");
            *last.borrow_mut() = line;
        }
    });
}

#[cfg(test)]
mod workbook_file_tests;
#[cfg(test)]
mod workbook_state_tests;

#[cfg(test)]
mod copybook_tests;
#[cfg(test)]
mod export_tests;
#[cfg(test)]
mod map_keys_tests;
#[cfg(test)]
mod options_tests;
#[cfg(test)]
mod tab_commands_tests;
#[cfg(test)]
mod tab_rename_tests;
