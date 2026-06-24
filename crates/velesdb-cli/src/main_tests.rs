//! Unit tests for the CLI entry point (`main.rs`).
//!
//! Extracted from `main.rs` into a sibling `*_tests.rs` file per the project
//! test convention (tests live beside their module). `super` resolves to the
//! crate root, so the original imports are unchanged.

use super::*;
use cli_types::{MetricArg, StorageModeArg};
use velesdb_core::{DistanceMetric, StorageMode};

#[test]
fn test_commands_enum_size_below_threshold() {
    let size = std::mem::size_of::<Commands>();
    eprintln!("Commands enum size: {} bytes", size);
    // Sub-enum grouping should keep the enum well under 1 KB.
    assert!(
        size < 1024,
        "Commands enum is {} bytes, expected < 1024",
        size
    );
}

// =========================================================================
// Tests for MetricArg conversions
// =========================================================================

#[test]
fn test_metric_arg_cosine() {
    let metric: DistanceMetric = MetricArg::Cosine.into();
    assert_eq!(metric, DistanceMetric::Cosine);
}

#[test]
fn test_metric_arg_euclidean() {
    let metric: DistanceMetric = MetricArg::Euclidean.into();
    assert_eq!(metric, DistanceMetric::Euclidean);
}

#[test]
fn test_metric_arg_dot() {
    let metric: DistanceMetric = MetricArg::Dot.into();
    assert_eq!(metric, DistanceMetric::DotProduct);
}

#[test]
fn test_metric_arg_hamming() {
    let metric: DistanceMetric = MetricArg::Hamming.into();
    assert_eq!(metric, DistanceMetric::Hamming);
}

#[test]
fn test_metric_arg_jaccard() {
    let metric: DistanceMetric = MetricArg::Jaccard.into();
    assert_eq!(metric, DistanceMetric::Jaccard);
}

// =========================================================================
// Tests for StorageModeArg conversions (Phase 1.2)
// =========================================================================

#[test]
fn test_storage_mode_arg_full() {
    let mode: StorageMode = StorageModeArg::Full.into();
    assert_eq!(mode, StorageMode::Full);
}

#[test]
fn test_storage_mode_arg_sq8() {
    let mode: StorageMode = StorageModeArg::Sq8.into();
    assert_eq!(mode, StorageMode::SQ8);
}

#[test]
fn test_storage_mode_arg_binary() {
    let mode: StorageMode = StorageModeArg::Binary.into();
    assert_eq!(mode, StorageMode::Binary);
}

#[test]
fn test_storage_mode_arg_pq() {
    let mode: StorageMode = StorageModeArg::Pq.into();
    assert_eq!(mode, StorageMode::ProductQuantization);
}

#[test]
fn test_storage_mode_arg_rabitq() {
    let mode: StorageMode = StorageModeArg::Rabitq.into();
    assert_eq!(mode, StorageMode::RaBitQ);
}

#[test]
fn test_storage_mode_arg_default_is_full() {
    let mode = StorageModeArg::default();
    assert!(matches!(mode, StorageModeArg::Full));
}

// =========================================================================
// Optional-value boolean flags: --include-vectors / --progress must default
// to true, accept an explicit value, AND still work bare (regression guard
// against the SetTrue definition that made them un-disableable no-ops).
// =========================================================================

fn parse_export_include_vectors(args: &[&str]) -> bool {
    match Cli::try_parse_from(args)
        .expect("export args should parse")
        .command
    {
        Commands::Data {
            action: DataCommands::Export {
                include_vectors, ..
            },
        } => include_vectors,
        _ => panic!("expected `data export`"),
    }
}

fn parse_import_progress(args: &[&str]) -> bool {
    match Cli::try_parse_from(args)
        .expect("import args should parse")
        .command
    {
        Commands::Data {
            action: DataCommands::Import { progress, .. },
        } => progress,
        _ => panic!("expected `data import`"),
    }
}

#[test]
fn test_include_vectors_defaults_true() {
    assert!(parse_export_include_vectors(&[
        "velesdb", "data", "export", "db", "coll"
    ]));
}

#[test]
fn test_include_vectors_explicit_false_disables() {
    assert!(!parse_export_include_vectors(&[
        "velesdb",
        "data",
        "export",
        "db",
        "coll",
        "--include-vectors",
        "false",
    ]));
}

#[test]
fn test_include_vectors_bare_flag_stays_true() {
    assert!(parse_export_include_vectors(&[
        "velesdb",
        "data",
        "export",
        "db",
        "coll",
        "--include-vectors",
    ]));
}

#[test]
fn test_progress_defaults_true() {
    assert!(parse_import_progress(&[
        "velesdb", "data", "import", "f.jsonl", "-c", "coll"
    ]));
}

#[test]
fn test_progress_explicit_false_disables() {
    assert!(!parse_import_progress(&[
        "velesdb",
        "data",
        "import",
        "f.jsonl",
        "-c",
        "coll",
        "--progress",
        "false",
    ]));
}

// =========================================================================
// query execute --collection (parity backlog #18b): a bare MATCH in
// `query execute` must be able to pick a target collection, mirroring the
// REST /query collection body field.
// =========================================================================

fn parse_query_execute_collection(args: &[&str]) -> Option<String> {
    match Cli::try_parse_from(args)
        .expect("query execute args should parse")
        .command
    {
        Commands::QueryCmd {
            action: QueryCommands::Execute { collection, .. },
        } => collection,
        _ => panic!("expected `query execute`"),
    }
}

#[test]
fn test_query_execute_collection_defaults_none() {
    assert_eq!(
        parse_query_execute_collection(&["velesdb", "query", "execute", "db", "SELECT * FROM t"]),
        None
    );
}

#[test]
fn test_query_execute_collection_flag() {
    assert_eq!(
        parse_query_execute_collection(&[
            "velesdb",
            "query",
            "execute",
            "db",
            "MATCH (a)-[:KNOWS]->(b) RETURN a, b LIMIT 10",
            "--collection",
            "g",
        ]),
        Some("g".to_string())
    );
}
