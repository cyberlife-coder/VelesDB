//! Build script for tauri-plugin-velesdb
//!
//! Generates Tauri plugin permissions for all commands.
//!
//! IMPORTANT: This list MUST be kept in sync with:
//! - `src/lib.rs` `invoke_handler` registration
//! - `permissions/default.toml` [default] permissions
//!
//! The sync is enforced by the permission tests in `src/commands_tests.rs`,
//! which parse the `lib.rs` registration, this array, and the TOML.
//!
//! When adding a new command:
//! 1. Add the command function to `commands.rs` or `commands_graph.rs`
//! 2. Register it in `lib.rs` `invoke_handler`
//! 3. Add it to this COMMANDS array (triggers permission file generation)
//! 4. Add "allow-{command-name}" to default.toml [default] section

const COMMANDS: &[&str] = &[
    // Collection management
    "create_collection",
    "create_metadata_collection",
    "delete_collection",
    "list_collections",
    "get_collection",
    "is_empty",
    "flush",
    "scroll_collection",
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
    // AgentMemory (semantic, episodic, procedural)
    "semantic_store",
    "semantic_store_with_ttl",
    "semantic_query",
    "semantic_delete",
    "semantic_dimension",
    "semantic_serialize",
    "semantic_deserialize",
    "episodic_record",
    "episodic_recent",
    "episodic_recall_similar",
    "episodic_older_than",
    "episodic_delete",
    "episodic_serialize",
    "episodic_deserialize",
    "procedural_learn",
    "procedural_recall",
    "procedural_reinforce",
    "procedural_list_all",
    "procedural_delete",
    "procedural_serialize",
    "procedural_deserialize",
    // AgentMemory TTL / eviction / snapshot-versioning / VelesQL parity
    "memory_set_ttl",
    "memory_auto_expire",
    "memory_evict_low_confidence",
    "memory_snapshot",
    "memory_load_latest_snapshot",
    "memory_load_snapshot_version",
    "memory_list_snapshot_versions",
    "memory_query_semantic",
    "memory_query_episodic",
    "memory_query_procedural",
    // Knowledge Graph
    "create_graph_collection",
    "add_edge",
    "get_edges",
    "traverse_graph",
    "get_node_degree",
    "traverse_graph_parallel",
    // Sparse vector operations
    "sparse_search",
    "hybrid_sparse_search",
    "sparse_upsert",
    // PQ training
    "train_pq",
    // Streaming insert (persistence only)
    "stream_insert",
    // Secondary Indexes
    "create_index",
    "drop_index",
    "list_indexes",
];

fn main() {
    tauri_plugin::Builder::new(COMMANDS).build();
}
