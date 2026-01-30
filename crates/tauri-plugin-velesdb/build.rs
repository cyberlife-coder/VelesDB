//! Build script for tauri-plugin-velesdb
//!
//! Generates Tauri plugin permissions for all commands.
//!
//! IMPORTANT: This list MUST be kept in sync with:
//! - `src/lib.rs` invoke_handler registration
//! - `src/commands_tests.rs` REGISTERED_COMMANDS array
//! - `permissions/default.toml` [default] permissions
//!
//! When adding a new command:
//! 1. Add the command function to commands.rs or commands_graph.rs
//! 2. Register it in lib.rs invoke_handler
//! 3. Add it to this COMMANDS array (triggers permission file generation)
//! 4. Add "allow-{command-name}" to default.toml [default] section
//! 5. Add it to commands_tests.rs REGISTERED_COMMANDS array

const COMMANDS: &[&str] = &[
    // Collection management
    "create_collection",
    "create_metadata_collection",
    "delete_collection",
    "list_collections",
    "get_collection",
    "is_empty",
    "flush",
    // Point operations
    "upsert",
    "upsert_metadata",
    "get_points",
    "delete_points",
    // Search operations
    "search",
    "batch_search",
    "text_search",
    "hybrid_search",
    "multi_query_search",
    "query",
    // AgentMemory (semantic)
    "semantic_store",
    "semantic_query",
    // Knowledge Graph
    "add_edge",
    "get_edges",
    "traverse_graph",
    "get_node_degree",
];

fn main() {
    tauri_plugin::Builder::new(COMMANDS).build();
}
