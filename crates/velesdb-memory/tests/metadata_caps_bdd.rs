//! BDD integration tests for the metadata and working-context size caps
//! (`DoS` guards): `MemoryError::MetadataTooLarge` on `remember` and on a
//! context-compiler fragment's own `metadata`, and the `MAX_FACT_BYTES`
//! ceiling on `save_working_context`.
//!
//! Categories: Nominal (≥60%), Edge (~20%), Negative (≥20%).

#![cfg(all(feature = "context", feature = "persistence"))]

mod common;

use common::service;
use serde_json::{Map, Value};
use velesdb_memory::context::{
    CompilePolicy, CompileRequest, ContextCompiler, ContextFragment, WorkingContext,
};
use velesdb_memory::{limits, ErrorCategory, MemoryError};

/// A metadata map whose serialized JSON is exactly `target_bytes` long,
/// built from one string value padded to hit the byte target — simplest way
/// to land exactly on either side of [`limits::MAX_METADATA_BYTES`] without
/// guessing JSON overhead by hand.
fn metadata_of_size(target_bytes: usize) -> Map<String, Value> {
    // `{"v":""}` is 8 bytes; pad `v`'s value so the whole object hits
    // `target_bytes` exactly.
    let overhead = 8;
    let padding = "x".repeat(target_bytes.saturating_sub(overhead));
    let mut meta = Map::new();
    meta.insert("v".to_owned(), Value::String(padding));
    let built = limits::metadata_bytes(&meta);
    assert_eq!(
        built, target_bytes,
        "test helper must build metadata of the exact requested size"
    );
    meta
}

fn fragment(content: &str) -> ContextFragment {
    ContextFragment {
        id: None,
        content: content.to_owned(),
        kind: None,
        priority: None,
        metadata: None,
        media: None,
    }
}

fn compile_request(fragments: Vec<ContextFragment>) -> CompileRequest {
    CompileRequest {
        query: "q".to_owned(),
        fragments,
        project: None,
        target_model: None,
        token_budget: 10_000,
        memory_scope: None,
        policy: None,
    }
}

// --- Nominal -----------------------------------------------------------------

#[test]
fn test_remember_with_metadata_at_63kib_succeeds() {
    // Given metadata comfortably under the 64 KiB cap
    let (_dir, svc) = service();
    let meta = metadata_of_size(63 * 1024);

    // When remembering a fact tagged with it
    let result = svc.remember("a fact under the metadata cap", &[], Some(&meta));

    // Then it succeeds
    assert!(
        result.is_ok(),
        "63 KiB metadata must pass under the 64 KiB cap: {result:?}"
    );
}

#[test]
fn test_working_context_normal_round_trips() {
    // Given a small, ordinary working context
    let (_dir, svc) = service();
    let wc = WorkingContext {
        goal: Some("ship the metadata cap".to_owned()),
        pending_actions: vec!["open the PR".to_owned()],
        ..WorkingContext::default()
    };

    // When saved and loaded back
    svc.save_working_context("veles", "session-caps", &wc)
        .expect("save must succeed for an ordinary working context");
    let loaded = svc
        .load_working_context("veles", "session-caps")
        .expect("load")
        .expect("a genuinely saved working context must round-trip");

    // Then the working state is intact — non-regression on the marker check
    assert_eq!(loaded.goal.as_deref(), Some("ship the metadata cap"));
    assert_eq!(loaded.pending_actions, vec!["open the PR".to_owned()]);
}

// --- Edge ----------------------------------------------------------------------

#[test]
fn test_metadata_exactly_at_cap_succeeds() {
    // Given metadata exactly at the 64 KiB boundary
    let (_dir, svc) = service();
    let meta = metadata_of_size(limits::MAX_METADATA_BYTES);

    // When remembering a fact tagged with it
    let result = svc.remember("a fact exactly at the metadata cap", &[], Some(&meta));

    // Then the boundary itself is accepted (only strictly-over rejects)
    assert!(
        result.is_ok(),
        "metadata exactly at MAX_METADATA_BYTES must not be rejected: {result:?}"
    );
}

// --- Negative --------------------------------------------------------------------

#[test]
fn test_remember_with_oversized_metadata_errors() {
    // Given metadata just over the 64 KiB cap
    let (_dir, svc) = service();
    let meta = metadata_of_size(65 * 1024);

    // When remembering a fact tagged with it
    let err = svc
        .remember("a fact over the metadata cap", &[], Some(&meta))
        .expect_err("oversized metadata must be rejected");

    // Then the typed error fires, categorized as caller input
    assert!(
        matches!(err, MemoryError::MetadataTooLarge { .. }),
        "expected MetadataTooLarge, got {err:?}"
    );
    assert_eq!(err.category(), ErrorCategory::InvalidInput);
}

#[test]
fn test_remember_with_oversized_metadata_stores_nothing() {
    // Given metadata just over the cap
    let (_dir, svc) = service();
    let fact = "a fact that must never be persisted";
    let meta = metadata_of_size(65 * 1024);

    // When the oversized remember is rejected
    svc.remember(fact, &[], Some(&meta))
        .expect_err("oversized metadata must be rejected");

    // Then nothing was stored under it
    let hits = svc.recall(fact, 5, None).expect("recall");
    assert!(
        hits.is_empty(),
        "a rejected oversized-metadata remember must not leave a partial write"
    );
}

#[test]
fn test_compile_context_fragment_with_oversized_metadata_errors() {
    // Given a compile request whose sole fragment carries 65 KiB of metadata
    let mut meta = Map::new();
    meta.insert("v".to_owned(), Value::String("x".repeat(65 * 1024 - 8)));
    let oversized = ContextFragment {
        metadata: Some(meta),
        ..fragment("ordinary content")
    };
    let compiler = ContextCompiler::new(CompilePolicy::default());

    // When compiling
    let err = compiler
        .compile(&compile_request(vec![oversized]))
        .expect_err("an oversized fragment metadata must be rejected");

    // Then the same typed, InvalidInput-categorized error fires
    assert!(
        matches!(err, MemoryError::MetadataTooLarge { .. }),
        "expected MetadataTooLarge, got {err:?}"
    );
    assert_eq!(err.category(), ErrorCategory::InvalidInput);
}

#[test]
fn test_save_working_context_over_1mib_errors_and_stores_nothing() {
    // Given a working context whose serialized JSON exceeds MAX_FACT_BYTES (1 MiB)
    let (_dir, svc) = service();
    let huge = "x".repeat(limits::MAX_FACT_BYTES + 1);
    let wc = WorkingContext {
        goal: Some(huge),
        ..WorkingContext::default()
    };

    // When saving it
    let err = svc
        .save_working_context("veles", "huge-session", &wc)
        .expect_err("an over-cap working context must be rejected");
    assert!(
        matches!(err, MemoryError::ContextOverLimit(_)),
        "expected ContextOverLimit, got {err:?}"
    );

    // Then a later load sees nothing was ever stored
    let loaded = svc
        .load_working_context("veles", "huge-session")
        .expect("load must not error");
    assert!(
        loaded.is_none(),
        "a rejected oversized working context must leave nothing to load"
    );
}
