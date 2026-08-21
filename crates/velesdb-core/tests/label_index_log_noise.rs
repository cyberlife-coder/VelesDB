//! Behaviour: an unindexable node id is not a WARN-worthy event — the
//! AGGREGATE is (#1834).
//!
//! velesdb-memory ids are hashed u64s, so exceeding `u32::MAX` is the
//! NOMINAL state of every node of a memory store — yet each one used to emit
//! its own WARN at startup: 788 lines before the first request, in the very
//! log the incident preset (#1780/#1727) exists to keep readable. A warning
//! that describes the normal state of the world is not a warning; at that
//! volume it buries the `mcp http request` / `worker quit` lines an operator
//! greps for.
//!
//! The contract proven here, phase by phase under a counting subscriber:
//!
//! * an UNLABELED node with an oversized id emits NO warning at all — it had
//!   nothing to index, so nothing was lost (this is the memory-store nominal
//!   case, the 788);
//! * a rebuild that really did skip labeled nodes emits EXACTLY ONE warning,
//!   carrying the aggregated count — never one line per node. Per-node
//!   detail stays available at DEBUG for whoever turns it on.
//!
//! One `#[test]` on purpose: the counting subscriber must be the process
//! global (the rebuild may not run on the test thread), and a global can be
//! installed once — sequential phases inside one test keep every count
//! attributable.

#![cfg(feature = "persistence")]

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use serde_json::json;
use velesdb_core::collection::graph::LabelIndex;
use velesdb_core::{Database, DistanceMetric, Point};

/// Counts WARN events from the label-index code paths (`label_index` for the
/// per-node site, `lifecycle` for the rebuild aggregate) and records the
/// `skipped` field of the last such WARN — `u64::MAX` meaning "never seen".
struct WarnCounter {
    warns: Arc<AtomicUsize>,
    last_skipped: Arc<AtomicU64>,
}

/// Field visitor pulling the `skipped` count out of the aggregate WARN.
struct SkippedVisitor<'a>(&'a AtomicU64);

impl tracing::field::Visit for SkippedVisitor<'_> {
    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        if field.name() == "skipped" {
            self.0.store(value, Ordering::Relaxed);
        }
    }

    fn record_debug(&mut self, _: &tracing::field::Field, _: &dyn std::fmt::Debug) {}
}

impl tracing::Subscriber for WarnCounter {
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }

    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }

    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}

    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}

    fn event(&self, event: &tracing::Event<'_>) {
        let metadata = event.metadata();
        let target = metadata.target();
        if !(target.contains("label_index") || target.contains("lifecycle")) {
            return;
        }
        if *metadata.level() == tracing::Level::WARN {
            self.warns.fetch_add(1, Ordering::Relaxed);
            event.record(&mut SkippedVisitor(&self.last_skipped));
        }
    }

    fn enter(&self, _: &tracing::span::Id) {}

    fn exit(&self, _: &tracing::span::Id) {}
}

/// The first id past the bitmap's reach; every memory-store hash lands here.
const OVERSIZED: u64 = (u32::MAX as u64) + 1;

#[test]
fn an_unindexable_id_warns_in_aggregate_never_per_node() {
    let warns = Arc::new(AtomicUsize::new(0));
    let last_skipped = Arc::new(AtomicU64::new(u64::MAX));
    tracing::subscriber::set_global_default(WarnCounter {
        warns: Arc::clone(&warns),
        last_skipped: Arc::clone(&last_skipped),
    })
    .expect("install the counting subscriber");

    // --- Phase A: the memory-store nominal case. 50 UNLABELED nodes with
    // hashed-range ids must not emit a single warning — this is the exact
    // shape of the 788-line startup.
    let mut index = LabelIndex::new();
    for i in 0..50 {
        index.index_from_payload(OVERSIZED + i, &json!({ "content": "a fact" }));
    }
    assert_eq!(
        warns.load(Ordering::Relaxed),
        0,
        "an unlabeled node with an oversized id lost nothing and must not warn — \
         at memory-store scale this line IS the noise #1834 measured (788/startup)"
    );
    assert!(
        !index.has_large_ids(),
        "nothing indexable was skipped, so lookups need no full-scan fallback"
    );

    // --- Phase B: a store whose rebuild really does lose labeled nodes.
    let dir = tempfile::tempdir().expect("tempdir");
    {
        let db = Database::open(dir.path()).expect("open db");
        db.create_collection("g", 4, DistanceMetric::Cosine)
            .expect("create collection");
        let coll = db.get_vector_collection("g").expect("collection");
        let mut points: Vec<Point> = Vec::new();
        for i in 0..5 {
            points.push(Point {
                id: OVERSIZED + i,
                vector: vec![0.1, 0.2, 0.3, 0.4],
                payload: Some(json!({ "_labels": ["Person"] })),
                sparse_vectors: None,
            });
        }
        for i in 0..20 {
            points.push(Point {
                id: OVERSIZED + 100 + i,
                vector: vec![0.4, 0.3, 0.2, 0.1],
                payload: Some(json!({ "content": "unlabeled" })),
                sparse_vectors: None,
            });
        }
        coll.upsert(points).expect("upsert");
    }

    let before = warns.load(Ordering::Relaxed);
    let db = Database::open(dir.path()).expect("reopen db");
    let _coll = db
        .get_vector_collection("g")
        .expect("collection after reopen");
    let emitted = warns.load(Ordering::Relaxed) - before;

    assert_eq!(
        emitted, 1,
        "a rebuild that skipped 5 labeled and scanned 20 unlabeled oversized \
         nodes must warn EXACTLY once, aggregated — one line per node is the \
         788-line failure mode, zero lines would hide a real indexing loss"
    );
    assert_eq!(
        last_skipped.load(Ordering::Relaxed),
        5,
        "the aggregate warning must carry the exact count of labeled nodes lost"
    );
}
