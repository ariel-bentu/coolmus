#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod backend;
mod commands;
mod settings;

use tauri::Manager;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    tauri::Builder::default()
        .setup(|app| {
            app.manage(backend::AppState::init());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_state,
            commands::get_tree,
            commands::search,
            commands::load_page,
            commands::save_page,
            commands::create_page,
            commands::create_folder,
            commands::rename,
            commands::move_item,
            commands::delete_item,
            commands::set_folder_label,
            commands::copy_items,
            commands::paste_items,
            commands::add_root,
            commands::remove_root,
            commands::update_settings,
            commands::reveal,
            commands::open_external,
            commands::print,
            commands::print_folder,
            commands::save_png,
            commands::import_onenote,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Sketcher");
}
