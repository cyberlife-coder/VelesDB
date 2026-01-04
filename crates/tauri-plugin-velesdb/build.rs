//! Build script for tauri-plugin-velesdb
//!
//! Generates Tauri plugin permissions for all commands.

const COMMANDS: &[&str] = &[
    "create_collection",
    "delete_collection",
    "list_collections",
    "get_collection",
    "upsert",
    "get_points",
    "delete_points",
    "search",
    "text_search",
    "hybrid_search",
    "query",
];

fn main() {
    tauri_plugin::Builder::new(COMMANDS).build();
}
