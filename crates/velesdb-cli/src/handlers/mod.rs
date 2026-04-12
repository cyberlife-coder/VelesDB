//! CLI subcommand handlers extracted from `main()`.
//!
//! Each module contains one or more handler functions that correspond to
//! a `Commands` variant. The `main()` dispatcher delegates to these.

mod collections;
mod data;
mod index;
mod info;
mod search;
mod tools;

pub use collections::{
    handle_create_graph_collection, handle_create_metadata_collection,
    handle_create_vector_collection, handle_delete_collection,
};
pub use data::{
    handle_delete_points, handle_export, handle_get, handle_import, handle_scroll,
    handle_stream_insert, handle_upsert,
};
pub use index::handle_index;
pub use info::{handle_analyze, handle_info, handle_list, handle_show};
pub use search::{handle_batch_search, handle_multi_search};
pub use tools::{handle_completions, handle_explain, handle_license, handle_simd};
