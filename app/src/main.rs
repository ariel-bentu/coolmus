#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod backend;
mod commands;
mod settings;
mod sync;

use tauri::Manager;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    tauri::Builder::default()
        .setup(|app| {
            let state = backend::AppState::init(app.handle().clone());
            app.manage(state);
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
            commands::update_settings,
            commands::choose_root,
            commands::save_png,
            commands::open_url,
            commands::reveal_root,
            commands::onenote_sign_in,
            commands::onenote_cancel,
            commands::onenote_sign_out,
            commands::onenote_list_notebooks,
            commands::onenote_list_pages,
            commands::onenote_import_page,
            commands::onenote_import_section,
            commands::onenote_import_notebook,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Sketcher");
}
