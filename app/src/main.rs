#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod backend;
mod commands;
mod settings;
mod updates;

use tauri::Manager;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    tauri::Builder::default()
        // The window only ever shows the app: a file dropped on it or a stray
        // link would otherwise replace Coolmus with that document.
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(
            tauri::plugin::Builder::<tauri::Wry>::new("stay-in-app")
                .on_navigation(|_, url| is_app_url(url))
                .build(),
        )
        .setup(|app| {
            app.manage(backend::AppState::init());
            if let Some(window) = app.get_webview_window("main") {
                fit_to_screen(&window);
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_state,
            commands::get_tree,
            commands::search,
            commands::load_page,
            commands::page_background,
            commands::save_page,
            commands::create_page,
            commands::create_folder,
            commands::rename,
            commands::move_item,
            commands::delete_item,
            commands::set_folder_label,
            commands::copy_items,
            commands::paste_items,
            commands::drop_file,
            commands::add_root,
            commands::remove_root,
            commands::move_root,
            commands::reorder,
            commands::update_settings,
            commands::reveal,
            commands::open_external,
            commands::print_view,
            commands::save_png,
            commands::pick_onenote_files,
            commands::import_onenote,
            updates::check_update,
            updates::install_update,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Coolmus");
}

/// Opens the window maximized, with a restored size that fits the screen: on a
/// small laptop screen the default size would put the bottom of the window,
/// and of its menus, off screen.
fn fit_to_screen(window: &tauri::WebviewWindow) {
    if let Ok(Some(monitor)) = window.current_monitor() {
        let area = monitor.work_area().size;
        if let Ok(size) = window.outer_size() {
            if size.width > area.width || size.height > area.height {
                let fit = tauri::PhysicalSize::new(size.width.min(area.width * 9 / 10), size.height.min(area.height * 9 / 10));
                let _ = window.set_size(fit);
                let _ = window.center();
            }
        }
    }
    let _ = window.maximize();
}

/// The app's own pages: tauri://localhost on macOS and Linux,
/// http(s)://tauri.localhost on Windows.
fn is_app_url(url: &tauri::Url) -> bool {
    match url.scheme() {
        "tauri" => true,
        "http" | "https" => url.host_str() == Some("tauri.localhost"),
        _ => false,
    }
}
