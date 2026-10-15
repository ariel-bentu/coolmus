#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod canvas;
mod settings;
mod sync;
mod ui_library;
mod ui_onenote;
mod ui_toolbar;

fn main() -> eframe::Result {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Sketcher")
            .with_app_id("sketcher")
            .with_inner_size([1280.0, 860.0])
            .with_min_inner_size([640.0, 420.0]),
        ..Default::default()
    };
    eframe::run_native("Sketcher", options, Box::new(|cc| Ok(Box::new(app::SketcherApp::new(cc)))))
}
