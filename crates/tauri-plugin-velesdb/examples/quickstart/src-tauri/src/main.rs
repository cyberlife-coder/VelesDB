// src-tauri/src/main.rs — the smallest host app that exposes VelesDB.
//
// Copy this into your Tauri 2 project at src-tauri/src/main.rs, or add the
// single `.plugin(...)` line to the builder you already have.
//
// Add the dependency first, from src-tauri/:
//     cargo add tauri-plugin-velesdb
//
// `init()` opens the database at ./velesdb_data, relative to the process's
// working directory. That is fine while prototyping and wrong for a shipped
// app — see ../../../production-config for the app-data variant.

// Hide the console window on Windows release builds. Standard Tauri boilerplate,
// unrelated to VelesDB.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    tauri::Builder::default()
        // Registering the plugin opens the database inside the app process and
        // exposes the IPC commands to the webview. The commands stay denied
        // until a capability allows them — see
        // src-tauri/capabilities/velesdb.json.
        .plugin(tauri_plugin_velesdb::init())
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
