#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod embeddings;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_velesdb::init())
        .invoke_handler(tauri::generate_handler![
            commands::ingest_text,
            commands::search,
            commands::get_stats,
            commands::clear_index,
            commands::get_model_status,
            commands::preload_model,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
