#![cfg(feature = "persistence")]
#![allow(
    clippy::cast_precision_loss,
    clippy::uninlined_format_args,
    clippy::doc_markdown,
    clippy::doc_lazy_continuation,
    clippy::doc_link_with_quotes
)]

#[path = "bdd/admin_operations.rs"]
mod admin_operations;
#[path = "bdd/advanced.rs"]
mod advanced;
#[path = "bdd/agent_memory.rs"]
mod agent_memory;
#[path = "bdd/agent_memory_graph_traversal.rs"]
mod agent_memory_graph_traversal;
#[path = "bdd/agent_memory_recall_exact.rs"]
mod agent_memory_recall_exact;
#[path = "bdd/aggregation.rs"]
mod aggregation;
#[path = "bdd/array_contains.rs"]
mod array_contains;
#[path = "bdd/bm25_match_conformance.rs"]
mod bm25_match_conformance;
#[path = "bdd/bugfixes.rs"]
mod bugfixes;
#[path = "bdd/collection_type_migration.rs"]
mod collection_type_migration;
#[path = "bdd/contains_text_filter.rs"]
mod contains_text_filter;
#[path = "bdd/cross_collection.rs"]
mod cross_collection;
#[path = "bdd/cross_collection_join_optimization.rs"]
mod cross_collection_join_optimization;
#[path = "bdd/ddl_lifecycle.rs"]
mod ddl_lifecycle;
#[path = "bdd/default_limit.rs"]
mod default_limit;
#[path = "bdd/dml_enhanced.rs"]
mod dml_enhanced;
#[path = "bdd/explain_analyze.rs"]
mod explain_analyze;
#[path = "bdd/explain_configurable_threshold.rs"]
mod explain_configurable_threshold;
#[path = "bdd/explain_cost_calibrated.rs"]
mod explain_cost_calibrated;
#[path = "bdd/flush_operations.rs"]
mod flush_operations;
#[path = "bdd/fusion_rrf_conformance.rs"]
mod fusion_rrf_conformance;
#[path = "bdd/fusion_weighted_bug.rs"]
mod fusion_weighted_bug;
#[path = "bdd/geo_distance.rs"]
mod geo_distance;
#[path = "bdd/graph_anchor_prefilter.rs"]
mod graph_anchor_prefilter;
#[path = "bdd/graph_queries.rs"]
mod graph_queries;
#[path = "bdd/graph_vector_hybrid.rs"]
mod graph_vector_hybrid;
#[path = "bdd/helpers.rs"]
mod helpers;
#[path = "bdd/hybrid_compositions.rs"]
mod hybrid_compositions;
#[path = "bdd/hybrid_vector_first_exact.rs"]
mod hybrid_vector_first_exact;
#[path = "bdd/index_management.rs"]
mod index_management;
#[path = "bdd/introspection.rs"]
mod introspection;
#[path = "bdd/join_exact_conformance.rs"]
mod join_exact_conformance;
#[path = "bdd/match_graph_first.rs"]
mod match_graph_first;
#[path = "bdd/match_order_by_exact.rs"]
mod match_order_by_exact;
#[path = "bdd/match_relationship_semantics.rs"]
mod match_relationship_semantics;
#[path = "bdd/match_traversal_exact.rs"]
mod match_traversal_exact;
#[path = "bdd/match_vector_first.rs"]
mod match_vector_first;
#[path = "bdd/metrics_ranking_conformance.rs"]
mod metrics_ranking_conformance;
#[path = "bdd/near_exact_ranking.rs"]
mod near_exact_ranking;
#[path = "bdd/near_fused_parse_only.rs"]
mod near_fused_parse_only;
#[path = "bdd/not_filters_exact.rs"]
mod not_filters_exact;
#[path = "bdd/operators.rs"]
mod operators;
#[path = "bdd/recall_contract.rs"]
mod recall_contract;
#[path = "bdd/recall_contract_multimetric.rs"]
mod recall_contract_multimetric;
#[path = "bdd/regression.rs"]
mod regression;
#[path = "bdd/scalar_filter_exact.rs"]
mod scalar_filter_exact;
#[path = "bdd/secondary_index_bitmap_in.rs"]
mod secondary_index_bitmap_in;
#[path = "bdd/set_operations.rs"]
mod set_operations;
#[path = "bdd/sparse_near_conformance.rs"]
mod sparse_near_conformance;
#[path = "bdd/temporal_computed_orderby.rs"]
mod temporal_computed_orderby;
#[path = "bdd/vector_group_by.rs"]
mod vector_group_by;
#[path = "bdd/vector_search.rs"]
mod vector_search;
#[path = "bdd/velesql_reject_conformance.rs"]
mod velesql_reject_conformance;
#[path = "bdd/where_filters.rs"]
mod where_filters;
#[path = "bdd/where_params.rs"]
mod where_params;
#[path = "bdd/window_functions.rs"]
mod window_functions;
