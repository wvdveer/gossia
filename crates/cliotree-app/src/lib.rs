//! The ClioTree desktop application, as a library so that its user interface
//! can be driven by tests.

pub mod app;
pub mod edit;
#[cfg(test)]
mod edit_tests;
pub mod rows;
pub mod sample;
pub mod status;
pub mod table;
pub mod table_view;
pub mod tables;
#[cfg(test)]
mod tables_tests;
pub mod view;

pub use app::ClioApp;

impl eframe::App for ClioApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.show(ui);
    }
}
