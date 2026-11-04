//! The ClioTree desktop application.

// A release build on Windows is a GUI program: no console window opens behind it. Debug
// builds keep the console for logs; the test control feature is for the tests.
#![cfg_attr(
    all(windows, not(debug_assertions), not(feature = "test-control")),
    windows_subsystem = "windows"
)]

use cliotree_app::ClioApp;
use cliotree_app::prefs::{Config, User};

/// A blank workbook, as *File > New* makes it, or the workbook named on the
/// command line (as *Save and restart* starts it), with the user's
/// preferences and trust record from disk. What could not be read is said
/// in the notice line. The window driven by the end-to-end tests starts
/// blank too: they drop a file on it to have data.
fn start() -> ClioApp {
    let (user, mut warnings) = User::load(Config::standard());
    let mut app = ClioApp::new(cliotree_app::workbook_file::blank_workbook()).with_user(user);
    if let Some(path) = std::env::args_os().nth(1)
        && let Err(e) = app.open_path(std::path::Path::new(&path))
    {
        warnings.push(e);
    }
    if !warnings.is_empty() {
        app.notice = Some(warnings.join(" "));
    }
    app
}

fn main() -> eframe::Result {
    let size = 64;
    let icon = egui::IconData {
        rgba: cliotree_app::app_icon::pixels(size),
        width: size,
        height: size,
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("ClioTree")
            // On Linux the desktop matches the window to cliotree.desktop
            // (packaging/linux) by this name, for its icon and launcher.
            .with_app_id("cliotree")
            .with_icon(std::sync::Arc::new(icon))
            .with_inner_size([1100.0, 720.0])
            .with_min_inner_size([640.0, 400.0]),
        ..Default::default()
    };
    eframe::run_native("ClioTree", options, creator())
}

#[cfg(not(feature = "test-control"))]
fn creator() -> eframe::AppCreator<'static> {
    Box::new(|_cc| Ok(Box::new(start())))
}

/// With the `test-control` feature the window also listens for the tests.
#[cfg(feature = "test-control")]
fn creator() -> eframe::AppCreator<'static> {
    use cliotree_app::control_server::Controlled;
    Box::new(|cc| {
        let app = Controlled::start(start(), cc.egui_ctx.clone())?;
        Ok(Box::new(app))
    })
}
