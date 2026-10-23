//! The ClioTree desktop application.

use cliotree_app::ClioApp;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("ClioTree")
            .with_inner_size([1100.0, 720.0])
            .with_min_inner_size([640.0, 400.0]),
        ..Default::default()
    };
    eframe::run_native("ClioTree", options, creator())
}

#[cfg(not(feature = "test-control"))]
fn creator() -> eframe::AppCreator<'static> {
    Box::new(|_cc| Ok(Box::new(ClioApp::sample())))
}

/// With the `test-control` feature the window also listens for the tests.
#[cfg(feature = "test-control")]
fn creator() -> eframe::AppCreator<'static> {
    use cliotree_app::control_server::Controlled;
    Box::new(|cc| {
        let app = Controlled::start(ClioApp::sample(), cc.egui_ctx.clone())?;
        Ok(Box::new(app))
    })
}
