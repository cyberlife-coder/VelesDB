//! Behavior PINS for context-compiler surfaces already published in 0.9.0
//! (PR V0-D, a CI/test-hardening pass — no product-behavior change).
//!
//! These tests do not enforce a spec: they pin CURRENT observable behavior
//! discovered by reading `budget.rs`/`classify.rs`/the memory bridge, so a
//! future change to any of it is a conscious, reviewed diff (a failing
//! pin), never a silent regression. Where the current behavior turned out
//! to be a real limitation, that is documented inline (and in the crate
//! README) rather than "fixed" — fixing it is out of scope here.
//!
//! Kept in its own file, separate from `context_memory_bdd.rs`, which other
//! PRs are concurrently editing.

#![cfg(all(feature = "context", feature = "persistence"))]

mod common;

use common::service;
use serde_json::Value;
use velesdb_memory::context::{
    CompilePolicy, CompileRequest, CompiledContext, ContextAction, ContextCompiler,
    ContextFragment, HeuristicEstimator, MediaRef, SectionKind, TokenEstimator,
};

fn fragment(content: &str) -> ContextFragment {
    ContextFragment {
        path: None,
        id: None,
        content: content.to_owned(),
        kind: None,
        priority: None,
        metadata: None,
        media: None,
    }
}

fn fragment_with_meta(content: &str, pairs: &[(&str, Value)]) -> ContextFragment {
    let mut meta = serde_json::Map::new();
    for (key, value) in pairs {
        meta.insert((*key).to_owned(), value.clone());
    }
    ContextFragment {
        metadata: Some(meta),
        ..fragment(content)
    }
}

fn request(query: &str, fragments: Vec<ContextFragment>, budget: u64) -> CompileRequest {
    CompileRequest {
        query: query.to_owned(),
        fragments,
        project: None,
        target_model: None,
        token_budget: budget,
        memory_scope: None,
        policy: None,
    }
}

/// Compile with the default, memoryless compiler (mirrors
/// `context_compiler_bdd.rs`'s helper).
fn compile(req: &CompileRequest) -> CompiledContext {
    ContextCompiler::new(CompilePolicy::default())
        .compile(req)
        .expect("compile")
}

fn cache_section_content(out: &CompiledContext) -> Option<String> {
    out.sections
        .iter()
        .find(|section| section.kind == SectionKind::Cache)
        .map(|section| section.content.clone())
}

// === Pin (a): cache-prefix byte stability under a CHANGING query ===========
//
// `selection_order` (crates/velesdb-memory/src/context/budget.rs:67-78)
// ranks same-critical, same-priority fragments by lexical relevance to the
// request's query — and `cache.stable_prefix` marks BOTH of two
// caller-marked-cache fragments critical with the default priority. Under a
// budget tight enough for only one of them, the query alone decides which
// one wins the Cache section. The crate's committed measurement harness
// (`examples/context_savings/real_measures/cache_prefix.mjs`) only ever
// re-runs the SAME query across turns, so it never exercises this path.

#[test]
fn test_cache_prefix_pin_under_a_changing_query_and_a_tight_budget() {
    let redis_fact = "The redis cluster caches session state for the login flow.";
    let mongo_fact = "The mongo cluster caches session state for the login flow.";
    let estimator = HeuristicEstimator;
    let cost_redis = estimator.estimate(redis_fact);
    let cost_mongo = estimator.estimate(mongo_fact);
    assert_eq!(
        cost_redis, cost_mongo,
        "fixture requires equal packed cost, so only relevance (query-dependent) breaks the tie"
    );

    // Several large, non-cache, non-critical filler fragments — plausible
    // "body volume" competing for the same tight budget; they are never
    // critical so they never affect which cache fragment wins, only
    // confirming the tight budget realistically starves the body too.
    let filler = |n: usize| -> ContextFragment {
        fragment(&format!(
            "Volatile turn {n}: the deploy queue processed {n} items, all green, \
             no retries needed, moving on to the next batch of work items now."
        ))
    };

    let fragments = || {
        vec![
            fragment_with_meta(redis_fact, &[("cache", Value::Bool(true))]),
            fragment_with_meta(mongo_fact, &[("cache", Value::Bool(true))]),
            filler(1),
            filler(2),
            filler(3),
        ]
    };

    // Tight budget: room for exactly one cache fragment plus its ~1-token
    // joiner, never both, and never any filler.
    let budget = cost_redis + 2;

    let out_redis_query = compile(&request("redis cluster status", fragments(), budget));
    let out_mongo_query = compile(&request("mongo cluster status", fragments(), budget));

    let cache_redis_query =
        cache_section_content(&out_redis_query).expect("a Cache section must exist");
    let cache_mongo_query =
        cache_section_content(&out_mongo_query).expect("a Cache section must exist");

    // PIN of the REAL behavior observed: the two cache-marked fragments tie
    // on criticality and priority, so relevance (query-dependent) picks the
    // winner — the Cache prefix is NOT byte-stable across a query change
    // under a budget too tight for both. This is a real limitation of the
    // "stable prefix for provider prompt caching" claim: it holds only when
    // the query stays fixed (exactly what the committed harness measures),
    // not across an arbitrary query change. See the crate README's
    // prompt-cache section and the tracking issue linked there.
    assert_ne!(
        cache_redis_query, cache_mongo_query,
        "documented limitation regressed: the cache prefix used to churn under a \
         changing query and a tight budget (query-dependent relevance breaks the \
         tie between two equally-critical cache fragments) — if this now passes \
         because the prefix is stable, the README/skill limitation note this test \
         guards should be revisited, not just this assertion"
    );
    assert!(
        cache_redis_query.contains("redis"),
        "the higher-relevance-for-this-query fragment must win: {cache_redis_query}"
    );
    assert!(
        cache_mongo_query.contains("mongo"),
        "the higher-relevance-for-this-query fragment must win: {cache_mongo_query}"
    );
}

// === Pin (b): a cache:true MEDIA fragment still gets media.atomic ==========
//
// classify.rs's RULES table is first-match-wins and lists `media.atomic`
// BEFORE `cache.stable_prefix` — so `metadata: {"cache": true}` on a media
// fragment is silently ignored: it still classifies `media.atomic`
// (Preserve, critical) and packs into the Body section, never Cache.

#[test]
fn test_media_fragment_marked_cache_true_still_classifies_media_atomic() {
    let png_b64 = "iVBORw0KGgoAAAANSUhEUgAAAEAAAAAwCAYAAAAAAAAA";
    let media_fragment = ContextFragment {
        media: Some(MediaRef {
            mime: "image/png".to_owned(),
            bytes_b64: png_b64.to_owned(),
        }),
        ..fragment_with_meta("a screenshot", &[("cache", Value::Bool(true))])
    };

    let out = compile(&request("q", vec![media_fragment], 10_000));

    assert_eq!(out.decisions.len(), 1);
    assert_eq!(
        out.decisions[0].rule_id, "media.atomic",
        "media.atomic must win the first-match rule table even when metadata.cache=true"
    );
    assert_eq!(out.decisions[0].action, ContextAction::Preserve);
    assert!(
        out.sections
            .iter()
            .all(|section| section.kind != SectionKind::Cache),
        "a cache:true media fragment must never land in the Cache section, \
         got sections: {:?}",
        out.sections.iter().map(|s| s.kind).collect::<Vec<_>>()
    );
}

// === Pin (c): working context on an untouched project/session ==============
//
// `load_working_context` on a project/session that was never saved returns
// `Ok(None)` cleanly. A stale/pre-0.9.0-shape JSON payload can NOT be forged
// through the public API to probe deserialization: `save_working_context`
// only accepts a strongly-typed `WorkingContext` (every field
// `#[serde(default)]`, see `context/model.rs`), so it always serializes the
// CURRENT shape — there is no reachable external path to write an
// old/malformed shape and read it back. Testing that would need an internal
// (non-`tests/`) unit test with access to the private deserialization path,
// which this CI-hardening PR does not add — documented here and skipped
// rather than forced.

#[test]
fn test_load_working_context_on_untouched_project_session_is_a_clean_none() {
    let (_dir, svc) = service();
    let loaded = svc
        .load_working_context("never-touched-project", "never-touched-session")
        .expect("load must not error on an untouched project/session");
    assert!(
        loaded.is_none(),
        "an untouched project/session must load as None, never an error"
    );
}

// === Pin (d): mime-divergent, byte-identical media fragments ===============
//
// Media identity (`Analysis::handle_hash` / `fragment_handle_hash`) is keyed
// on the raw DECODED bytes only, never the declared `mime`. Two fragments
// with byte-identical `bytes_b64` but different declared `mime` therefore
// dedupe onto the SAME `ctx://source` handle, and
// `store_context_sources`'s `by_hash.entry(hash).or_insert(fragment)` keeps
// the FIRST occurrence (the anchor) — so resolving the shared handle serves
// the ANCHOR's declared mime, even though a later duplicate declared a
// different one.

#[test]
fn test_mime_divergent_byte_identical_media_dedupes_and_resolves_the_anchors_mime() {
    let (_dir, svc) = service();
    let shared_bytes = "iVBORw0KGgoAAAANSUhEUgAAAEAAAAAwCAYAAAAAAAAA";
    let anchor = ContextFragment {
        media: Some(MediaRef {
            mime: "image/png".to_owned(),
            bytes_b64: shared_bytes.to_owned(),
        }),
        ..fragment("first occurrence")
    };
    let duplicate = ContextFragment {
        media: Some(MediaRef {
            mime: "image/jpeg".to_owned(),
            bytes_b64: shared_bytes.to_owned(),
        }),
        ..fragment("second occurrence, same bytes, different declared mime")
    };

    let compiler = ContextCompiler::new(CompilePolicy::default());
    let out = svc
        .compile_context(&compiler, &request("q", vec![anchor, duplicate], 10_000))
        .expect("compile");

    assert_eq!(
        out.sources.len(),
        1,
        "byte-identical media must dedupe onto one source regardless of declared mime"
    );
    let handle = out.sources[0].handle.clone();

    let resolved = svc
        .retrieve_context_source(&handle)
        .expect("retrieve the shared handle");
    let media = resolved.media.expect("media must round-trip");
    assert_eq!(
        media.mime, "image/png",
        "resolution must serve the ANCHOR fragment's declared mime, never a later duplicate's"
    );
    assert_eq!(media.bytes_b64, shared_bytes);
}
