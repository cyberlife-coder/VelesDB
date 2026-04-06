#![cfg(feature = "persistence")]
#![allow(clippy::cast_precision_loss, clippy::uninlined_format_args)]

#[path = "bdd/admin_operations.rs"]
mod admin_operations;
#[path = "bdd/advanced.rs"]
mod advanced;
#[path = "bdd/agent_memory.rs"]
mod agent_memory;
#[path = "bdd/aggregation.rs"]
mod aggregation;
#[path = "bdd/array_contains.rs"]
mod array_contains;
#[path = "bdd/bugfixes.rs"]
mod bugfixes;
#[path = "bdd/collection_type_migration.rs"]
mod collection_type_migration;
#[path = "bdd/cross_collection.rs"]
mod cross_collection;
#[path = "bdd/cross_collection_join_optimization.rs"]
mod cross_collection_join_optimization;
#[path = "bdd/ddl_lifecycle.rs"]
mod ddl_lifecycle;
#[path = "bdd/dml_enhanced.rs"]
mod dml_enhanced;
#[path = "bdd/flush_operations.rs"]
mod flush_operations;
#[path = "bdd/geo_distance.rs"]
mod geo_distance;
#[path = "bdd/graph_queries.rs"]
mod graph_queries;
#[path = "bdd/helpers.rs"]
mod helpers;
#[path = "bdd/hybrid_compositions.rs"]
mod hybrid_compositions;
#[path = "bdd/index_management.rs"]
mod index_management;
#[path = "bdd/introspection.rs"]
mod introspection;
#[path = "bdd/operators.rs"]
mod operators;
#[path = "bdd/recall_contract.rs"]
mod recall_contract;
#[path = "bdd/regression.rs"]
mod regression;
#[path = "bdd/secondary_index_bitmap_in.rs"]
mod secondary_index_bitmap_in;
#[path = "bdd/set_operations.rs"]
mod set_operations;
#[path = "bdd/vector_group_by.rs"]
mod vector_group_by;
#[path = "bdd/vector_search.rs"]
mod vector_search;
#[path = "bdd/where_filters.rs"]
mod where_filters;
