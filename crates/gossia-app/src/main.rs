//! The Gossia desktop application.

// A release build on Windows is a GUI program: no console window opens behind it. Debug
// builds keep the console for logs; the test control feature is for the tests.
#![cfg_attr(
    all(windows, not(debug_assertions), not(feature = "test-control")),
    windows_subsystem = "windows"
)]

use gossia_app::GossiaApp;
use gossia_app::prefs::{Config, User};

/// A blank workbook, as *File > New* makes it, or the workbook named on the
/// command line (as *Save and restart* starts it), or a blank one with the
/// import dialog of another file named there, with the user's
/// preferences and trust record from disk. What could not be read is said
/// in the notice line. The window driven by the end-to-end tests starts
/// blank too: they drop a file on it to have data.
fn start() -> GossiaApp {
    let (user, mut warnings) = User::load(Config::standard());
    let mut app = GossiaApp::new(gossia_app::workbook_file::blank_workbook()).with_user(user);
    // The interface's language (DESIGN.md, step 13f), before anything is
    // shown: the one chosen in Options, or the system's.
    gossia_i18n::set_language(app.language_for(app.user.prefs.appearance.language));
    // A workbook opens; any other file (a calendar the system hands over,
    // say) is imported into the blank workbook, as dropping it would.
    if let Some(path) = std::env::args_os().nth(1).map(std::path::PathBuf::from) {
        let workbook = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case(gossia_app::workbook_file::EXTENSION));
        let opened = if workbook {
            app.open_path(&path)
        } else {
            app.begin_import_path(&path)
        };
        warnings.extend(opened.err());
    }
    if !warnings.is_empty() {
        app.notice = Some(warnings.join(" "));
    }
    app
}

fn main() -> eframe::Result {
    let size = 64;
    let icon = egui::IconData {
        rgba: gossia_app::app_icon::pixels(size),
        width: size,
        height: size,
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Gossia")
            // On Linux the desktop matches the window to gossia.desktop
            // (packaging/linux) by this name, for its icon and launcher.
            .with_app_id("gossia")
            .with_icon(std::sync::Arc::new(icon))
            .with_inner_size([1100.0, 720.0])
            .with_min_inner_size([640.0, 400.0]),
        ..Default::default()
    };
    eframe::run_native("Gossia", options, creator())
}

#[cfg(not(feature = "test-control"))]
fn creator() -> eframe::AppCreator<'static> {
    Box::new(|_cc| Ok(Box::new(start())))
}

/// With the `test-control` feature the window also listens for the tests.
#[cfg(feature = "test-control")]
fn creator() -> eframe::AppCreator<'static> {
    use gossia_app::control_server::Controlled;
    Box::new(|cc| {
        let app = Controlled::start(start(), cc.egui_ctx.clone())?;
        Ok(Box::new(app))
    })
}
