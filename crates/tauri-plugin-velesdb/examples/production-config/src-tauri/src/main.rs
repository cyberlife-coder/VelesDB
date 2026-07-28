// src-tauri/src/main.rs — the shape you would actually ship.
//
// Two differences from the quickstart, both of which matter in a release:
//
//   1. the database lives in the platform's per-user app-data directory
//      instead of ./velesdb_data next to whatever the working directory
//      happened to be;
//   2. the engine is configured from a TOML file that FAILS FAST — a missing,
//      unparsable or out-of-range file stops the app instead of silently
//      falling back to defaults.
//
// Add the dependency first, from src-tauri/:
//     cargo add tauri-plugin-velesdb

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;

/// Name used to build the app-data path.
///
/// Windows: %APPDATA%\VelesDBExample\velesdb\
/// macOS:   ~/Library/Application Support/VelesDBExample/velesdb/
/// Linux:   ~/.local/share/VelesDBExample/velesdb/
const APP_NAME: &str = "VelesDBExample";

fn main() {
    // `get_app_data_dir` resolves the platform directory and returns an error
    // rather than guessing when it cannot. Failing here is correct: an app that
    // cannot find its own data directory has nowhere to put user data.
    let data_dir: PathBuf = tauri_plugin_velesdb::get_app_data_dir(APP_NAME)
        .expect("could not resolve the platform app-data directory");
    println!("VelesDB data directory: {}", data_dir.display());

    let mut builder = tauri_plugin_velesdb::Builder::new(&data_dir);

    // Optional engine tuning. `with_config_path` returns Err when the file is
    // missing, is not valid TOML, or fails engine validation — the typed core
    // error is preserved in `Error::ConfigLoad`.
    //
    // Only the engine sections are read: [search], [hnsw], [storage],
    // [limits], [quantization], [wal_batch]. A [server] table belonging to
    // another VelesDB component in a shared file is ignored, not rejected.
    //
    // A relative path here has exactly the working-directory problem this file
    // exists to avoid, so it is treated as a development convenience: absent ->
    // keep the engine defaults, present but broken -> stop. For a shipped
    // build, bundle the file as a Tauri resource and hand `with_config_path` an
    // absolute path resolved from `BaseDirectory::Resource` — see ../README.md.
    let dev_config = PathBuf::from("velesdb.toml");
    if dev_config.exists() {
        builder = builder
            .with_config_path(&dev_config)
            .expect("velesdb.toml is present but invalid");
    }

    tauri::Builder::default()
        .plugin(builder.build())
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

// ---------------------------------------------------------------------------
// The simpler variants, for reference
// ---------------------------------------------------------------------------
//
// App-data directory, no engine tuning, one line:
//
//     .plugin(
//         tauri_plugin_velesdb::init_with_app_data("VelesDBExample")
//             .expect("could not resolve the platform app-data directory"),
//     )
//
// A directory you own outright:
//
//     .plugin(tauri_plugin_velesdb::init_with_path("./my_data"))
//
// Prototyping only — ./velesdb_data relative to the working directory:
//
//     .plugin(tauri_plugin_velesdb::init())
