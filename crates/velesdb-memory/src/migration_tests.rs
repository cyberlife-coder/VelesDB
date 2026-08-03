//! The feasibility gate for #1762 (PR A).
//!
//! Whether a rebuild is even possible turns on one question the architecture
//! does not answer on paper: can every fact be read back out with its `u64` id,
//! its content, its ordinary metadata, its RESERVED metadata and its absolute
//! expiry — completely, once each, across page boundaries?
//!
//! `MemoryStore` cannot: every read is by id or a top-`k` vector search. Two
//! paths below it can — a `VelesQL` scan walked by `LIMIT`/`OFFSET`, and the
//! collection's own `scroll_batch` cursor — and this file exists because
//! neither is trustworthy on the strength of compiling. Each is run against a
//! seeded store, compared field by field, and compared against the other.
//!
//! The measurements are the point, and they did not agree with the plan: the
//! `OFFSET` walk is not merely quadratic but BOUNDED, going silently empty past
//! offset `100_000`, while the cursor — which an earlier reading of this same
//! architecture concluded did not exist — reads a 100 001-fact store whole in
//! about a second. See `past_the_ceiling_the_offset_walk_truncates_and_the_cursor_does_not`.
//!
//! A `Missing` verdict here stops the whole project before PR B rather than
//! producing an identifier mapping nobody asked for.

use super::{enumerate_collection, RawFact, AGENT_COLLECTIONS};
use crate::storage::NativeStore;
use crate::{MemoryStore, Metadata};
use serde_json::Value;
use std::collections::BTreeSet;

const DIM: usize = 4;
const EMBEDDING: [f32; 4] = [1.0, 0.0, 0.0, 0.0];

/// Facts enough to cross a page boundary that is not a divisor of the count,
/// so an off-by-one in the walk shows up as a gap or a repeat.
const SEEDED: u64 = 7;
const PAGE: usize = 3;

/// Ids that are NOT contiguous and are NOT inserted in ascending order.
///
/// A fixture of `1..=n` written in order proves far less than it appears to:
/// insertion order, id order and physical order all coincide, so a walk that
/// paged by position would look correct. These do not coincide — `2000` is
/// written first and `7` last — which is what makes a gap or a repeat visible.
const SCRAMBLED: &[u64] = &[2000, 41, 999, 3, 17, 1_000_000, 58, 7];

fn meta(pairs: &[(&str, Value)]) -> Metadata {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), v.clone()))
        .collect()
}

/// A store holding facts that carry every shape the rebuild must preserve:
/// an ordinary field, a RESERVED field, and one fact under a TTL.
///
/// The store handle is DROPPED before returning, and that is not tidiness: the
/// engine holds an exclusive `velesdb.lock` on the directory, so a second
/// handle cannot open it while the first lives. A diagnosis therefore only ever
/// runs against a store nothing else has open — which is the offline protocol
/// this migration is built on, observed here rather than assumed.
fn seeded() -> (tempfile::TempDir, Metadata) {
    let dir = tempfile::tempdir().expect("tempdir");
    let ttl_metadata;
    {
        let store = NativeStore::open(dir.path(), DIM).expect("open store");
        for id in 1..=SEEDED {
            store
                .store_with_metadata(
                    id,
                    &format!("fact number {id}"),
                    &EMBEDDING,
                    &meta(&[
                        ("project", Value::from("veles")),
                        ("_veles_hub", Value::Bool(id == 1)),
                    ]),
                )
                .expect("seed fact");
        }
        store
            .store_with_metadata_and_ttl(
                100,
                "a fact under a ttl",
                &EMBEDDING,
                &meta(&[("project", Value::from("veles"))]),
                3600,
            )
            .expect("seed ttl fact");
        ttl_metadata = store
            .get_metadata(100)
            .expect("read back")
            .expect("the ttl fact exists");
    }
    (dir, ttl_metadata)
}

/// Reopen the seeded directory as a bare `Database` — the read path a
/// diagnosis uses, which takes no dimension and so does not refuse.
fn database(dir: &tempfile::TempDir) -> velesdb_core::Database {
    velesdb_core::Database::open(dir.path()).expect("open database")
}

// ---------------------------------------------------------------------------
// THE GATE
// ---------------------------------------------------------------------------

#[test]
fn raw_enumeration_has_no_gaps_or_duplicates_across_pages() {
    let (dir, _ttl_meta) = seeded();
    let db = database(&dir);

    let facts = enumerate_collection(&db, "_semantic_memory", PAGE).expect("enumerate");

    let ids: Vec<u64> = facts.iter().map(|f| f.id).collect();
    let unique: BTreeSet<u64> = ids.iter().copied().collect();
    assert_eq!(
        ids.len(),
        unique.len(),
        "a page walk must not return a fact twice, got {ids:?}"
    );
    let expected: BTreeSet<u64> = (1..=SEEDED).chain(std::iter::once(100)).collect();
    assert_eq!(
        unique, expected,
        "every seeded fact must come back exactly once; a missing id is a fact \
         the rebuild would silently drop"
    );
}

#[test]
fn raw_enumeration_carries_content_ordinary_and_reserved_metadata() {
    let (dir, _ttl_meta) = seeded();
    let db = database(&dir);

    let facts = enumerate_collection(&db, "_semantic_memory", PAGE).expect("enumerate");
    let first = facts
        .iter()
        .find(|f| f.id == 1)
        .expect("fact 1 must be enumerated");
    let payload: Value = serde_json::from_str(&first.payload).expect("payload is json");

    assert_eq!(
        payload.get("content").and_then(Value::as_str),
        Some("fact number 1"),
        "content must survive the scan verbatim: {payload}"
    );
    assert_eq!(
        payload.get("project").and_then(Value::as_str),
        Some("veles"),
        "ordinary metadata must survive the scan: {payload}"
    );
    assert_eq!(
        payload.get("_veles_hub").and_then(Value::as_bool),
        Some(true),
        "RESERVED metadata must survive the scan — the rebuild has to put it \
         back, and a scan that strips it makes that impossible: {payload}"
    );
}

#[test]
fn raw_enumeration_carries_the_absolute_expiry() {
    let (dir, ttl_meta) = seeded();
    let db = database(&dir);

    let expected = ttl_meta
        .get("_veles_expires_at")
        .expect("the storage layer records an absolute expiry")
        .clone();

    let facts = enumerate_collection(&db, "_semantic_memory", PAGE).expect("enumerate");
    let ttl_fact = facts
        .iter()
        .find(|f| f.id == 100)
        .expect("the ttl fact must be enumerated");
    let payload: Value = serde_json::from_str(&ttl_fact.payload).expect("payload is json");

    assert_eq!(
        payload.get("_veles_expires_at"),
        Some(&expected),
        "the expiry must come back as the SAME absolute instant — recomputing a \
         duration from migration time would silently extend every fact's life"
    );
}

#[test]
fn an_empty_collection_enumerates_as_empty_rather_than_failing() {
    // Empty is not the same as absent: the other two collections are opened at
    // the same dimension and so take part in the refusal, which means the
    // rebuild has to recreate them even with nothing in them.
    let (dir, _ttl_meta) = seeded();
    let db = database(&dir);

    for collection in AGENT_COLLECTIONS
        .iter()
        .filter(|c| **c != "_semantic_memory")
    {
        let facts = enumerate_collection(&db, collection, PAGE)
            .unwrap_or_else(|e| panic!("{collection} must be inventoriable, got {e}"));
        assert_eq!(
            facts,
            Vec::<RawFact>::new(),
            "{collection} holds nothing yet must still answer"
        );
    }
}

/// The positive control. Without it, an `enumerate_collection` that returned an
/// empty vector for everything would satisfy the gap-and-duplicate test above.
#[test]
fn the_gate_would_notice_an_enumeration_that_returns_nothing() {
    let (dir, _ttl_meta) = seeded();
    let db = database(&dir);

    let facts = enumerate_collection(&db, "_semantic_memory", PAGE).expect("enumerate");
    assert!(
        !facts.is_empty(),
        "a seeded collection that enumerates empty means the scan does not work, \
         not that the store is empty"
    );
}

// ---------------------------------------------------------------------------
// GATE 2 — complete, deterministic paging
//
// Correctness and cost are proven separately: a walk can be exhaustive and
// still be unusable at scale. The tests below settle correctness; the cost
// profile is measured on its own, further down.
// ---------------------------------------------------------------------------

/// A store seeded with [`SCRAMBLED`], plus the count the engine itself reports.
///
/// The count is captured while the store is alive because the exclusive lock
/// forbids holding both handles, and it is the independent witness the page
/// walk is checked against — comparing a walk to a list this same module built
/// would only prove the module agrees with itself.
fn scrambled_store() -> (tempfile::TempDir, BTreeSet<u64>, usize) {
    let dir = tempfile::tempdir().expect("tempdir");
    let counted;
    {
        let store = NativeStore::open(dir.path(), DIM).expect("open store");
        for id in SCRAMBLED {
            store
                .store_with_metadata(
                    *id,
                    &format!("fact {id}"),
                    &EMBEDDING,
                    &meta(&[("project", Value::from("veles"))]),
                )
                .expect("seed");
        }
        counted = store.count();
    }
    (dir, SCRAMBLED.iter().copied().collect(), counted)
}

#[test]
fn every_page_size_yields_the_same_complete_set() {
    let (dir, expected, _) = scrambled_store();
    let db = database(&dir);

    // 1 and 2 divide nothing; 3 leaves a remainder; 7 is one short of the
    // total; 99 exceeds it, so the walk must stop on a short page.
    for page in [1usize, 2, 3, 7, 99] {
        let facts = enumerate_collection(&db, "_semantic_memory", page)
            .unwrap_or_else(|e| panic!("page {page}: {e}"));
        let ids: Vec<u64> = facts.iter().map(|f| f.id).collect();
        let unique: BTreeSet<u64> = ids.iter().copied().collect();
        assert_eq!(
            ids.len(),
            unique.len(),
            "page {page}: duplicate ids {ids:?}"
        );
        assert_eq!(
            unique, expected,
            "page {page}: the union of pages must be exactly the seeded set"
        );
    }
}

#[test]
fn the_walk_agrees_with_the_engines_own_count() {
    let (dir, expected, counted) = scrambled_store();
    let db = database(&dir);

    let facts = enumerate_collection(&db, "_semantic_memory", 3).expect("enumerate");
    assert_eq!(
        facts.len(),
        counted,
        "the walk must return as many facts as the collection reports holding"
    );
    assert_eq!(
        counted,
        expected.len(),
        "the fixture and the engine must agree"
    );
}

#[test]
fn repeated_walks_return_the_same_order() {
    let (dir, _, _) = scrambled_store();
    let db = database(&dir);

    let first: Vec<u64> = enumerate_collection(&db, "_semantic_memory", 3)
        .expect("enumerate")
        .iter()
        .map(|f| f.id)
        .collect();
    for run in 2..=4 {
        let again: Vec<u64> = enumerate_collection(&db, "_semantic_memory", 3)
            .expect("enumerate")
            .iter()
            .map(|f| f.id)
            .collect();
        assert_eq!(
            first, again,
            "run {run} returned a different order; a resumable walk cannot be \
             built on an order that varies between executions"
        );
    }
    let ascending: Vec<u64> = {
        let mut sorted = first.clone();
        sorted.sort_unstable();
        sorted
    };
    assert_eq!(
        first, ascending,
        "the walk must come back in ascending id order — that is the property \
         `ORDER BY id` is there to provide, and the natural scan order does NOT \
         have it (measured: ids came back out of order without it)"
    );
}

#[test]
fn resuming_at_a_recorded_offset_skips_nothing_and_repeats_nothing() {
    let (dir, expected, _) = scrambled_store();
    let db = database(&dir);
    let page = 3usize;

    // Walk the first page, record where a checkpoint would resume, then finish
    // from there — the shape a rebuild interrupted after one batch would take.
    let head = enumerate_page(&db, "_semantic_memory", page, 0);
    let mut seen: Vec<u64> = head.iter().map(|f| f.id).collect();
    let mut offset = head.len();
    loop {
        let next = enumerate_page(&db, "_semantic_memory", page, offset);
        if next.is_empty() {
            break;
        }
        offset += next.len();
        seen.extend(next.iter().map(|f| f.id));
    }

    let unique: BTreeSet<u64> = seen.iter().copied().collect();
    assert_eq!(seen.len(), unique.len(), "resume duplicated ids: {seen:?}");
    assert_eq!(
        unique, expected,
        "a walk resumed from a recorded offset must cover exactly the same set"
    );
}

/// One page, at an explicit offset — the unit a checkpoint resumes from.
fn enumerate_page(
    db: &velesdb_core::Database,
    collection: &str,
    page: usize,
    offset: usize,
) -> Vec<super::RawFact> {
    super::enumerate_page(db, collection, page, offset).expect("page")
}

// ---------------------------------------------------------------------------
// THE GUARD — a rebuild must not come back to LIMIT/OFFSET
// ---------------------------------------------------------------------------

/// Files allowed to name the `OFFSET` walk, because they DEFINE it or exercise
/// it as the independent verification path.
const OFFSET_WALK_OWNERS: &[&str] = &["migration.rs", "migration_tests.rs"];

/// Whether a source text reaches for the bounded `OFFSET` walk.
///
/// A type-level guard would not hold here: in Rust a child module sees its
/// ancestor's private items, so a future `migration::rebuild` could construct
/// whatever "cursor-only" newtype this module defined. The enforceable line is
/// therefore contractual — and a contract nobody checks is decoration, so it is
/// checked below, with a positive control proving the check can fail.
fn uses_the_offset_walk(source: &str) -> bool {
    source.contains("enumerate_collection")
        || source.contains("enumerate_page")
        || source.contains("OFFSET")
}

/// No migration module beyond the verification path may page by `OFFSET`.
///
/// The walk is correct only below 100 000 facts and goes silently empty above
/// it, so a rebuild that used it would drop the tail of any large store and
/// report success. `scroll_page` is the supported route.
///
/// Today this scans zero files — `migration.rs` and `migration_tests.rs` own
/// the walk — and that is stated rather than hidden: the guard is armed for the
/// module PR B will add, and its detection logic is proven now, not then.
#[test]
fn no_migration_module_beyond_verification_reaches_for_the_offset_walk() {
    // Positive control: the check must be able to fail.
    assert!(
        uses_the_offset_walk("let facts = enumerate_page(&db, c, 10, 0);"),
        "the guard must catch a call into the OFFSET walk"
    );
    assert!(
        uses_the_offset_walk("SELECT * FROM c ORDER BY id LIMIT 10 OFFSET 20"),
        "the guard must catch a raw OFFSET query"
    );
    // Negative control: the supported route must not trip it.
    assert!(
        !uses_the_offset_walk("let batch = scroll_page(&db, c, cursor, 1024)?;"),
        "the guard must not fire on the cursor route"
    );

    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut scanned: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(&src).expect("read src/") {
        let path = entry.expect("dir entry").path();
        let name = path
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or_default()
            .to_owned();
        let is_rust = path.extension().and_then(std::ffi::OsStr::to_str) == Some("rs");
        if !name.starts_with("migration") || !is_rust {
            continue;
        }
        if OFFSET_WALK_OWNERS.contains(&name.as_str()) {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("read migration source");
        assert!(
            !uses_the_offset_walk(&text),
            "{name} reaches for the bounded OFFSET walk; a rebuild must page by \
             cursor (`scroll_page`), which carries no 100 000-fact ceiling"
        );
        scanned.push(name);
    }
    println!("  guard armed; migration modules scanned beyond the walk's owners: {scanned:?}");
}

// ---------------------------------------------------------------------------
// GATE 4 — the WRITE path, which is what actually bounds a rebuild
// ---------------------------------------------------------------------------
//
// The enumeration gates above measure reading. A rebuild also writes every fact
// back, and the write path in use today — `store_with_metadata`, one call per
// fact — resolves to `collection.upsert(vec![point])`: the SAME call the batch
// path makes, with a vector of one. So the question is not which API to use but
// what `upsert` costs per CALL rather than per point, and whether the dedicated
// `upsert_bulk` beats it.
//
// Until that is measured, `scalable_reconstruction` has no verdict, and PR B
// stays blocked on it rather than on the enumeration.

/// Micro-seconds per fact, in the style the cost tests above already use.
fn per_fact(elapsed: std::time::Duration, n: u64) -> f64 {
    let micros = u32::try_from(elapsed.as_micros()).map_or(f64::from(u32::MAX), f64::from);
    micros / f64::from(u32::try_from(n).expect("fits"))
}

/// Points shaped like the ones `store_with_metadata` writes.
fn bench_points(n: u64) -> Vec<velesdb_core::Point> {
    (1..=n)
        .map(|id| {
            let payload = serde_json::json!({
                "content": format!("fact {id}"),
                "project": "veles",
            });
            velesdb_core::Point::new(id, EMBEDDING.to_vec(), Some(payload))
        })
        .collect()
}

/// A store whose three collections exist but hold nothing.
fn empty_store() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    drop(NativeStore::open(dir.path(), DIM).expect("open store"));
    dir
}

/// Writes `points` through the collection in chunks of `batch`.
///
/// Returns the elapsed time, the number of write CALLS, and the number of facts
/// a cursor reads back afterwards. The read-back is outside the timer and is
/// not decoration: an `upsert` that silently wrote nothing would otherwise post
/// the best number in the table.
fn time_batched(
    points: &[velesdb_core::Point],
    batch: usize,
    bulk: bool,
) -> (std::time::Duration, usize, usize) {
    let dir = empty_store();
    let db = database(&dir);
    let coll = db
        .get_vector_collection("_semantic_memory")
        .expect("collection exists");
    let start = std::time::Instant::now();
    let mut calls = 0usize;
    for chunk in points.chunks(batch) {
        if bulk {
            coll.upsert_bulk(chunk).expect("bulk upsert");
        } else {
            coll.upsert(chunk.to_vec()).expect("upsert");
        }
        calls += 1;
    }
    let elapsed = start.elapsed();
    let read_back = super::enumerate_by_cursor(&db, "_semantic_memory", 4_096)
        .expect("read back")
        .len();
    (elapsed, calls, read_back)
}

/// Writes `n` facts one at a time through the path in use today.
fn time_unit(n: u64) -> std::time::Duration {
    let dir = tempfile::tempdir().expect("tempdir");
    let start = std::time::Instant::now();
    {
        let store = NativeStore::open(dir.path(), DIM).expect("open");
        for id in 1..=n {
            store
                .store_with_metadata(
                    id,
                    &format!("fact {id}"),
                    &EMBEDDING,
                    &meta(&[("project", Value::from("veles"))]),
                )
                .expect("seed");
        }
    }
    start.elapsed()
}

/// A deterministic vector whose values actually differ between facts.
///
/// [`bench_points`] gives every fact the SAME vector, which is not a neutral
/// simplification: in an HNSW graph every candidate then sits at distance zero
/// from every other and neighbour selection degenerates, so a per-point cost
/// measured that way may belong to the fixture rather than to the engine.
///
/// Built from bit patterns rather than float casts so it stays exact and
/// lint-clean: `(bits >> 9) | 0x3f80_0000` is an `f32` in `[1, 2)`.
fn distinct_vector(id: u64, dim: usize) -> Vec<f32> {
    let mut state = id.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    let mut out = Vec::with_capacity(dim);
    for _ in 0..dim {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let bits = u32::try_from(state >> 32).expect("shifted into 32 bits");
        out.push(f32::from_bits((bits >> 9) | 0x3f80_0000) - 1.5);
    }
    out
}

/// Builds the payload a benchmark row writes, so the shape can be varied
/// without spelling out the closure type at every use site.
type PayloadShape = fn(u64) -> Option<Value>;

/// The same points, but with vectors that differ.
fn varied_points(n: u64, dim: usize) -> Vec<velesdb_core::Point> {
    (1..=n)
        .map(|id| {
            let payload = serde_json::json!({
                "content": format!("fact {id}"),
                "project": "veles",
            });
            velesdb_core::Point::new(id, distinct_vector(id, dim), Some(payload))
        })
        .collect()
}

/// Is the per-point write cost the engine's, or the fixture's?
///
/// The grid below plateaus around 3.3 ms per point even when the whole volume
/// goes in ONE call, so the cost is per point and not per call. That is only
/// meaningful if the points are representative — hence this control, which
/// writes the same volume through the same call with varied vectors.
///
/// `#[ignore]`d: writes 2 000 facts twice.
#[test]
#[ignore = "writes 2 000 facts twice; run deliberately, on a machine at rest"]
fn the_per_point_write_cost_is_measured_against_varied_vectors() {
    const N: u64 = 2_000;
    const BATCH: usize = 1_024;
    let expected = usize::try_from(N).expect("fits");

    let (flat, _, flat_read) = time_batched(&bench_points(N), BATCH, true);
    assert_eq!(
        flat_read, expected,
        "identical-vector write must be readable"
    );

    let (varied, _, varied_read) = time_batched(&varied_points(N, DIM), BATCH, true);
    assert_eq!(
        varied_read, expected,
        "varied-vector write must be readable"
    );

    println!(
        "  identical vectors  {flat:>10.2?}  {:>9.1} us/fact",
        per_fact(flat, N)
    );
    println!(
        "  distinct  vectors  {varied:>10.2?}  {:>9.1} us/fact",
        per_fact(varied, N)
    );
    println!(
        "  ratio identical/distinct = {:.2}",
        per_fact(flat, N) / per_fact(varied, N)
    );
}

/// Which half of a point costs the 3.4 ms — its payload, or its vector?
///
/// Three hypotheses were refuted by measurement before this one was written: it
/// is not the `AgentMemory` layer (batch=1 costs the same as
/// `store_with_metadata`), not degenerate HNSW neighbours (identical and
/// distinct vectors cost the same, ratio 1.00), and not the debug profile
/// (`--release` gains ~6%). A cost insensitive to optimisation is not compute,
/// so this attributes it to a component instead of guessing at one.
///
/// Measured 2026-08-03 (macOS, `--release`, n = 2 000, batch = 1 024):
///
/// | payload shape               | us/fact | what runs                  |
/// |-----------------------------|---------|----------------------------|
/// | full (content+project)      |  3457.8 | `store_batch` + BM25 WAL   |
/// | no content key (still text) |  3418.7 | same — see below           |
/// | numeric only (no text)      |    15.1 | `store_batch` only         |
/// | no payload at all           |    10.2 | neither                    |
///
/// So the payload write itself costs ~5 us per point and the BM25 WAL costs
/// ~3 403 — **99.6% of the total**, a factor of ~229.
///
/// The cause is [`wal_append_add_document`], which opens the BM25 WAL file,
/// writes one entry and `sync_all()`s it — PER DOCUMENT — from a per-point loop
/// in `bulk_store_payloads_inner`. The bulk path is careful to take a single
/// WAL sync for payloads and a single flush for vectors, then falls back to an
/// open + fsync per point for the text index. ~3.4 ms is exactly one fsync.
///
/// Dropping the `content` key does not help because `extract_text` collects
/// EVERY string in the payload recursively, so `"veles"` is still indexable.
///
/// This bounds any rebuild, but it is not a migration defect: it caps every
/// insert of text-bearing points in the engine at ~290 points/s where ~66 000
/// is reachable. Tracked separately from #1762.
///
/// `#[ignore]`d: writes 2 000 facts four times.
#[test]
#[ignore = "writes 2 000 facts three times; run deliberately, on a machine at rest"]
fn the_per_point_write_cost_is_attributed_to_payload_or_vector() {
    const N: u64 = 2_000;
    const BATCH: usize = 1_024;
    let expected = usize::try_from(N).expect("fits");

    // `extract_text` collects EVERY string in the payload, recursively, so
    // dropping the `content` key is not enough to skip text indexing — the
    // remaining `"veles"` is still indexable. A purely numeric payload is what
    // separates the BM25 WAL from the rest of the payload write.
    let shapes: [(&str, PayloadShape); 4] = [
        ("full (content+project)", |id| {
            Some(serde_json::json!({ "content": format!("fact {id}"), "project": "veles" }))
        }),
        ("no content key (still text)", |_| {
            Some(serde_json::json!({ "project": "veles" }))
        }),
        ("numeric only (no text)", |id| {
            Some(serde_json::json!({ "n": id }))
        }),
        ("no payload at all", |_| None),
    ];

    for (label, shape) in shapes {
        let points: Vec<velesdb_core::Point> = (1..=N)
            .map(|id| velesdb_core::Point::new(id, distinct_vector(id, DIM), shape(id)))
            .collect();
        let (elapsed, _, read_back) = time_batched(&points, BATCH, true);
        assert_eq!(
            read_back, expected,
            "the write must stay readable for shape `{label}`"
        );
        println!(
            "  {label:<24}  {elapsed:>10.2?}  {:>9.1} us/fact",
            per_fact(elapsed, N)
        );
    }
}

/// Does the batched write stay linear as the volume grows?
///
/// This is the question that decides feasibility, and the one the batch-size
/// grid cannot answer: if the per-fact cost RISES with the volume, the rebuild
/// does not scale and `scalable_reconstruction` is `Missing` however good the
/// chunk size looks at 2 000 facts.
///
/// Measured twice, `--release`, batch = 1 024, `upsert_bulk`, distinct vectors,
/// dimension 4, on an idle machine. The two columns are the SAME code either
/// side of the BM25 fix (#1797), which removed a per-document fsync from the
/// bulk text-index path:
///
/// | facts  | before #1797 | after #1797 | facts/s after |
/// |--------|--------------|-------------|---------------|
/// | 1 000  | 3 555.8 us   | **24.1 us** | 41 475        |
/// | 2 000  | 3 408.8      | 20.5        | 48 794        |
/// | 4 000  | 3 341.6      | 17.7        | 56 518        |
/// | 8 000  | 3 374.0      | 16.7        | 59 720        |
/// | 16 000 | 3 345.8      | **16.3**    | **61 524**    |
///
/// The earlier figures were never the rebuild's cost: they were one fsync per
/// fact, paid inside `upsert_bulk` by the BM25 WAL. With that gone the same
/// walk runs ~205x faster, and the per-fact cost now FALLS as the volume grows
/// (ratios 0.85, 0.86, 0.95, 0.97) because the fixed costs amortise — better
/// than the linear behaviour this test was written to demand.
///
/// What that changes for the migration: a million-fact rebuild moves from
/// roughly 56 minutes to roughly 16 seconds, so throughput is no longer what
/// decides whether an offline rebuild is acceptable.
///
/// `#[ignore]`d: writes 31 000 facts in total.
#[test]
#[ignore = "writes 31 000 facts; run deliberately, on a machine at rest"]
fn batched_write_cost_stays_flat_as_the_volume_doubles() {
    const BATCH: usize = 1_024;
    let mut measured: Vec<(u64, f64)> = Vec::new();

    for n in [1_000u64, 2_000, 4_000, 8_000, 16_000] {
        let (elapsed, calls, read_back) = time_batched(&varied_points(n, DIM), BATCH, true);
        assert_eq!(
            read_back,
            usize::try_from(n).expect("fits"),
            "the batched write must be readable in full at n={n}"
        );
        let cost = per_fact(elapsed, n);
        let ratio = measured.last().map_or(String::from("—"), |(_, prev)| {
            format!("x{:.2}", cost / prev)
        });
        println!(
            "  n={n:6}  {elapsed:>10.2?}  {cost:>9.1} us/fact  {:>8.0} facts/s  calls={calls:<4} ratio={ratio}",
            1_000_000.0 / cost
        );
        measured.push((n, cost));
    }

    let first = measured.first().expect("measured").1;
    let last = measured.last().expect("measured").1;
    println!("  per-fact cost {first:.1} -> {last:.1} us/fact across a 16x volume increase");
    assert!(
        last < first * 2.0,
        "a 16x volume must not double the PER-FACT cost, or the rebuild does not \
         scale: {first:.1} -> {last:.1} us/fact. Quadratic growth here means \
         `scalable_reconstruction` is Missing and PR B does not start."
    );
}

/// Batch size against throughput, at one fixed volume — the table that says
/// whether a batched rebuild is worth writing at all, and at what chunk size.
///
/// `#[ignore]`d: it writes 2 000 facts once per configuration.
#[test]
#[ignore = "writes 2 000 facts per configuration; run deliberately, on a machine at rest"]
fn write_path_unit_versus_batch_at_a_fixed_volume() {
    const N: u64 = 2_000;

    let unit = time_unit(N);
    println!(
        "  store_with_metadata (unit)      {unit:>12.2?}  {:>9.1} us/fact  calls={N}",
        per_fact(unit, N)
    );

    let points = bench_points(N);
    let mut best = (usize::MAX, f64::MAX, false);
    for batch in [1usize, 16, 64, 256, 1_024, 4_096] {
        for bulk in [false, true] {
            let (elapsed, calls, read_back) = time_batched(&points, batch, bulk);
            assert_eq!(
                read_back,
                usize::try_from(N).expect("fits"),
                "the write must be READABLE afterwards: batch={batch} bulk={bulk} \
                 wrote {read_back} of {N}"
            );
            let cost = per_fact(elapsed, N);
            let label = if bulk { "upsert_bulk" } else { "upsert     " };
            println!(
                "  {label} batch={batch:<5}      {elapsed:>12.2?}  {cost:>9.1} us/fact  calls={calls}"
            );
            if cost < best.1 {
                best = (batch, cost, bulk);
            }
        }
    }

    let unit_cost = per_fact(unit, N);
    println!(
        "  BEST: {} at batch={} -> {:.1} us/fact ({:.1}x the unit path)",
        if best.2 { "upsert_bulk" } else { "upsert" },
        best.0,
        best.1,
        unit_cost / best.1
    );
    assert!(
        best.1 < unit_cost,
        "batching must beat the per-fact path or a rebuild has no batched route: \
         best {:.1} us/fact vs unit {unit_cost:.1} us/fact",
        best.1
    );
}

// ---------------------------------------------------------------------------
// GATE 2b — cost, measured separately from correctness
// ---------------------------------------------------------------------------

/// The cost profile of the `ORDER BY id` + `OFFSET` walk, on this machine at
/// rest. `#[ignore]`d because it seeds thousands of facts and takes tens of
/// seconds; run it deliberately, never as part of the ordinary suite.
///
/// Measured 2026-08-03 (macOS, page size 100):
///
/// | facts | elapsed  | µs/fact | ratio on doubling |
/// |-------|----------|---------|-------------------|
/// | 250   | 10.3 ms  | 41.1    | —                 |
/// | 500   | 24.4 ms  | 48.8    | ×2.37             |
/// | 1 000 | 69.7 ms  | 69.7    | ×2.86             |
/// | 2 000 | 252.9 ms | 126.4   | ×3.63             |
///
/// The per-fact cost RISES with the volume and the doubling ratio approaches
/// four: the walk is quadratic, which is what re-sorting the whole collection
/// for every page costs. Correct, and unusable at scale — extrapolating to a
/// million facts puts the walk in the hours.
///
/// This is why `scalable_enumeration` is [`Capability::Missing`] while
/// `deterministic_enumeration` is proven: a store of eight facts hides it
/// completely, and the rebuild is meant for stores that are not small.
#[test]
#[ignore = "seeds thousands of facts; run deliberately, on a machine at rest"]
fn paging_cost_grows_faster_than_the_store() {
    let mut costs: Vec<f64> = Vec::new();
    for n in [250u64, 500, 1000, 2000] {
        let dir = tempfile::tempdir().expect("tempdir");
        {
            let store = NativeStore::open(dir.path(), DIM).expect("open");
            for id in 1..=n {
                store
                    .store_with_metadata(
                        id * 7,
                        &format!("fact {id}"),
                        &EMBEDDING,
                        &meta(&[("project", Value::from("veles"))]),
                    )
                    .expect("seed");
            }
        }
        let db = database(&dir);
        let start = std::time::Instant::now();
        let facts = enumerate_collection(&db, "_semantic_memory", 100).expect("enumerate");
        let elapsed = start.elapsed();
        assert_eq!(facts.len(), usize::try_from(n).expect("fits"));
        let cost = per_fact(elapsed, n);
        println!("  n={n:5}  elapsed={elapsed:>9.2?}  us/fact={cost:>7.1}");
        costs.push(cost);
    }
    let first = costs.first().copied().expect("measured");
    let last = costs.last().copied().expect("measured");
    assert!(
        last > first * 1.5,
        "the per-fact cost was expected to RISE with the volume (it is what          makes this walk unusable at scale); if it no longer does, the engine          gained a cheaper scan and this capability should be re-classified —          first={first:.1} us/fact, last={last:.1} us/fact"
    );
}

// ---------------------------------------------------------------------------
// GATE 2c — the ceiling, which is a CORRECTNESS bound and not a cost one
// ---------------------------------------------------------------------------

// The executor caps how far an `OFFSET` walk can reach, and the cap is silent.
// `compute_fetch_limit` asks the collection for `limit + offset` rows and clamps
// that to `MAX_LIMIT` (100_000), then `apply_select_postprocessing` skips
// `offset` of what came back. Past the cap the subtraction is the whole page, so
// the query succeeds and returns nothing.
//
// That matters far more than the quadratic cost measured above. A slow walk is
// merely slow; a walk that ends early while reporting success drops facts. The
// completeness proof in this file ran on stores of eight facts and 2 000 facts —
// both well under the cap — so it establishes nothing above it.

// ---------------------------------------------------------------------------
// GATE 3 — the cursor, which is the enumeration the rebuild should actually use
// ---------------------------------------------------------------------------

/// The cursor walk returns the same facts as the page walk, payloads included.
///
/// This is the parity check that lets the `OFFSET` walk stand as the reference
/// while the cursor takes over: they are independent paths through the engine —
/// one goes through the `VelesQL` pipeline, the other calls `scroll_batch` on
/// the collection — so agreement between them is evidence, where a cursor
/// compared against a list this module built would only prove self-consistency.
///
/// It doubles as its own positive control: a cursor that returned nothing would
/// fail against a page walk that returns eight facts.
#[test]
fn the_cursor_walk_matches_the_page_walk_fact_for_fact() {
    let (dir, expected, _count) = scrambled_store();
    let db = database(&dir);

    let by_page = enumerate_collection(&db, "_semantic_memory", PAGE).expect("page walk");
    let by_cursor = super::enumerate_by_cursor(&db, "_semantic_memory", PAGE).expect("cursor walk");

    let cursor_ids: BTreeSet<u64> = by_cursor.iter().map(|f| f.id).collect();
    assert_eq!(
        by_cursor.len(),
        cursor_ids.len(),
        "the cursor walk returned a fact twice: {:?}",
        by_cursor.iter().map(|f| f.id).collect::<Vec<_>>()
    );
    assert_eq!(
        cursor_ids, expected,
        "the cursor walk must cover exactly the seeded ids"
    );

    // Payloads too, not just ids: the rebuild re-inserts what is in them, so a
    // cursor that carried ids but dropped reserved keys would pass an id-only
    // check and still lose data.
    let mut page_sorted = by_page;
    page_sorted.sort_by_key(|f| f.id);
    let mut cursor_sorted = by_cursor;
    cursor_sorted.sort_by_key(|f| f.id);
    for (page, cursor) in page_sorted.iter().zip(cursor_sorted.iter()) {
        let page_json: Value = serde_json::from_str(&page.payload).expect("page payload is json");
        let cursor_json: Value =
            serde_json::from_str(&cursor.payload).expect("cursor payload is json");
        assert_eq!(
            page_json, cursor_json,
            "the two paths disagree on the payload of fact {}",
            page.id
        );
    }
}

/// A cursor resumed mid-walk continues where it stopped — the property that
/// makes a checkpointed rebuild possible, and the one `WHERE id > n` could not
/// give because filters never see the id.
#[test]
fn a_cursor_resumed_mid_walk_skips_nothing_and_repeats_nothing() {
    let (dir, expected, _count) = scrambled_store();
    let db = database(&dir);

    let (head, cursor) = super::scroll_page(&db, "_semantic_memory", None, 3).expect("first batch");
    let mut seen: Vec<u64> = head.iter().map(|f| f.id).collect();
    let mut next = cursor;
    while let Some(from) = next {
        let (batch, after) =
            super::scroll_page(&db, "_semantic_memory", Some(from), 3).expect("resumed batch");
        if batch.is_empty() {
            break;
        }
        seen.extend(batch.iter().map(|f| f.id));
        next = after;
    }

    let unique: BTreeSet<u64> = seen.iter().copied().collect();
    assert_eq!(seen.len(), unique.len(), "resume duplicated ids: {seen:?}");
    assert_eq!(
        unique, expected,
        "a walk resumed from a recorded cursor must cover exactly the same set"
    );
    let ascending: Vec<u64> = expected.iter().copied().collect();
    assert_eq!(
        seen, ascending,
        "scroll_batch documents ascending id order; the rebuild's checkpointing \
         depends on it"
    );
}

/// The same store, above the cap, read BOTH ways — the comparison is the point.
///
/// Measured 2026-08-03 (macOS, 100 001 facts seeded in 1242 s):
///
/// | read path              | result                                    |
/// |------------------------|-------------------------------------------|
/// | OFFSET page at 99 995  | 5 rows returned, 10 asked for and existing |
/// | OFFSET page at 100 000 | 0 rows, though fact 100 001 exists         |
/// | cursor walk            | 100 001 rows in 1.31 s (13.1 us/fact)      |
///
/// So the `OFFSET` walk does not merely slow down past the cap — it goes blind,
/// and `enumerate_collection`, which stops on an empty page, would have called
/// that a complete enumeration of a store missing its tail. The cursor reads the
/// same store whole, and does it in about a second.
///
/// One number here is not about reading at all: seeding took 1242 s for 100 001
/// facts — 12.4 ms per fact, against 13.1 US per fact to read one back. Writing
/// is ~950x the cost of reading, so the rebuild in PR B is bounded by its
/// re-insertion, not by this enumeration. `AnyCollection::upsert` takes a
/// `Vec<Point>`; whether batching it beats the per-fact path is a measurement
/// PR B owes, not an assumption it may make.
///
/// `#[ignore]`d: it seeds just over 100 000 facts, which is the only way to
/// observe a bound that sits at 100 000, and it takes ~21 minutes.
#[test]
#[ignore = "seeds 100_001 facts; run deliberately, on a machine at rest"]
fn past_the_ceiling_the_offset_walk_truncates_and_the_cursor_does_not() {
    const CEILING: u64 = 100_000;
    const SEEDED_ABOVE: u64 = CEILING + 1;
    const PAGE: usize = 10;

    let dir = tempfile::tempdir().expect("tempdir");
    let seed_start = std::time::Instant::now();
    {
        let store = NativeStore::open(dir.path(), DIM).expect("open");
        for id in 1..=SEEDED_ABOVE {
            store
                .store_with_metadata(
                    id,
                    &format!("fact {id}"),
                    &EMBEDDING,
                    &meta(&[("project", Value::from("veles"))]),
                )
                .expect("seed");
        }
    }
    println!(
        "  seeded {SEEDED_ABOVE} facts in {:?}",
        seed_start.elapsed()
    );
    let db = database(&dir);

    // --- the OFFSET walk, at and around the cap -----------------------------

    // Straddling it: `limit + offset` exceeds the cap, so the page comes back
    // SHORT — fewer rows than asked for, though that many facts exist.
    let straddling = enumerate_page(
        &db,
        "_semantic_memory",
        PAGE,
        usize::try_from(CEILING - 5).expect("fits"),
    );
    // At it: nothing at all, though one more fact exists beyond.
    let past = enumerate_page(
        &db,
        "_semantic_memory",
        PAGE,
        usize::try_from(CEILING).expect("fits"),
    );
    println!(
        "  OFFSET page at {} -> {} rows (asked {PAGE})",
        CEILING - 5,
        straddling.len()
    );
    println!(
        "  OFFSET page at {CEILING} -> {} rows (fact {SEEDED_ABOVE} exists)",
        past.len()
    );
    assert!(
        straddling.len() < PAGE,
        "expected the cap to SHORTEN a straddling page; it returned a full page          of {}, so the executor no longer clamps `limit + offset`",
        straddling.len()
    );
    assert!(
        past.is_empty(),
        "expected the cap to EMPTY a page at the ceiling; it returned {} rows",
        past.len()
    );

    // --- the cursor walk, over the same store -------------------------------

    let cursor_start = std::time::Instant::now();
    let by_cursor =
        super::enumerate_by_cursor(&db, "_semantic_memory", 10_000).expect("cursor enumeration");
    let cursor_elapsed = cursor_start.elapsed();
    let ids: BTreeSet<u64> = by_cursor.iter().map(|f| f.id).collect();
    println!(
        "  CURSOR walk -> {} rows in {cursor_elapsed:?} ({:.1} us/fact)",
        by_cursor.len(),
        per_fact(cursor_elapsed, SEEDED_ABOVE)
    );

    assert_eq!(
        by_cursor.len(),
        ids.len(),
        "the cursor walk returned a fact twice"
    );
    assert_eq!(
        ids.len(),
        usize::try_from(SEEDED_ABOVE).expect("fits"),
        "the cursor walk must reach EVERY fact past the cap that bounds the          OFFSET walk; it stopped at {} of {SEEDED_ABOVE}",
        ids.len()
    );
    assert!(
        ids.contains(&SEEDED_ABOVE),
        "the fact beyond the cap is precisely the one the OFFSET walk drops; the          cursor must carry it"
    );
}

// ---------------------------------------------------------------------------
// GATE 3 — the inventory
//
// A rebuild has to be told what it is about to move before it moves it, and by
// something that CANNOT move it. Every test below runs the diagnosis and then
// checks the store is exactly as it was — because a read-only claim that is
// never checked is a claim, not a property.
// ---------------------------------------------------------------------------

use super::{diagnose, Capability, SourceProvenance};

const TARGET_MODEL: &str = "bge-m3";
const TARGET_DIM: usize = 1024;

/// Every file under `dir`, by relative path, with its length and its bytes.
///
/// Timestamps are deliberately absent: `atime` moves when a file is READ, so a
/// comparison including it would report every diagnosis as a modification and
/// prove nothing. Content and length are what a rebuild would actually lose.
fn tree(dir: &std::path::Path) -> std::collections::BTreeMap<String, (u64, Vec<u8>)> {
    fn walk(
        root: &std::path::Path,
        dir: &std::path::Path,
        out: &mut std::collections::BTreeMap<String, (u64, Vec<u8>)>,
    ) {
        for entry in std::fs::read_dir(dir).expect("read_dir") {
            let entry = entry.expect("entry");
            let path = entry.path();
            if entry.metadata().expect("metadata").is_dir() {
                walk(root, &path, out);
            } else {
                let rel = path
                    .strip_prefix(root)
                    .expect("under root")
                    .to_string_lossy()
                    .into_owned();
                let bytes = std::fs::read(&path).expect("read file");
                out.insert(rel, (bytes.len() as u64, bytes));
            }
        }
    }
    let mut out = std::collections::BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

/// Find one collection's entry in a report, by name.
fn inventory_of<'a>(
    report: &'a super::DiagnosisReport,
    name: &str,
) -> &'a super::CollectionInventory {
    report
        .collections
        .iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("{name} must appear in the report"))
}

#[test]
fn all_three_collections_are_inventoried() {
    let (dir, _ttl_meta) = seeded();

    let report = diagnose(dir.path(), TARGET_MODEL, TARGET_DIM, None).expect("diagnose");

    let named: Vec<&str> = report.collections.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        named, AGENT_COLLECTIONS,
        "the report must carry one entry per agent collection, in a fixed order, \
         so a store missing one is visible as ABSENT rather than as a shorter list"
    );

    let semantic = inventory_of(&report, "_semantic_memory");
    assert!(semantic.present, "the seeded collection exists");
    assert_eq!(
        semantic.dimension,
        Some(DIM),
        "the source width is what makes the store unopenable at the target width; \
         a report that omitted it would omit the reason for the migration"
    );
    assert_eq!(
        semantic.facts,
        SEEDED + 1,
        "the walk must count every seeded fact, the TTL one included"
    );
    assert_eq!(
        report.facts,
        SEEDED + 1,
        "the store-wide total must equal the sum over collections"
    );
    assert!(
        semantic.reserved_metadata.contains("_veles_hub"),
        "a reserved key present in a payload must be reported; an unlisted one is \
         a key the rebuild would not know to carry, got {:?}",
        semantic.reserved_metadata
    );
    assert_eq!(
        semantic.ttl.with_expiry, 1,
        "exactly one seeded fact carries an expiry"
    );
    assert!(
        semantic.ttl.earliest.is_some_and(|e| e > 0),
        "an expiry is an ABSOLUTE unix second; a zero or absent bound would mean \
         the rebuild has nothing to re-attach"
    );
    assert_eq!(
        report.format_version,
        super::DIAGNOSIS_FORMAT_VERSION,
        "every report is stamped, so a later binary can refuse a shape it does not know"
    );
}

/// The positive control for the test above. Without it, an inventory that
/// reported zeroes everywhere would satisfy every equality that follows from
/// an empty store.
#[test]
fn the_inventory_would_notice_a_store_it_failed_to_read() {
    let empty = tempfile::tempdir().expect("tempdir");
    {
        let _store = NativeStore::open(empty.path(), DIM).expect("open store");
    }
    let on_empty = diagnose(empty.path(), TARGET_MODEL, TARGET_DIM, None).expect("diagnose");

    let (seeded_dir, _ttl) = seeded();
    let on_seeded = diagnose(seeded_dir.path(), TARGET_MODEL, TARGET_DIM, None).expect("diagnose");

    assert_eq!(
        on_empty.facts, 0,
        "a store with nothing in it must report nothing"
    );
    assert!(
        on_seeded.facts > on_empty.facts,
        "the inventory must tell a seeded store from an empty one; both reporting \
         {} means it reads neither",
        on_empty.facts
    );
}

#[test]
fn empty_collections_are_reported() {
    let (dir, _ttl_meta) = seeded();

    let report = diagnose(dir.path(), TARGET_MODEL, TARGET_DIM, None).expect("diagnose");

    // Only `_semantic_memory` was seeded; the other two exist and hold nothing.
    let episodic = inventory_of(&report, "_episodic_memory");
    assert!(
        episodic.present,
        "an empty collection still EXISTS, and still pins the store's width — \
         reporting it as absent would understate what the rebuild must recreate"
    );
    assert!(episodic.is_empty(), "nothing was seeded into it");
    assert_eq!(episodic.facts, 0);
    assert_eq!(
        episodic.dimension,
        Some(DIM),
        "an empty collection is still stored at a width, and that width is what \
         refuses the new model"
    );

    // ...and the two states are distinguishable, which is the whole point.
    let missing = tempfile::tempdir().expect("tempdir");
    {
        let db = velesdb_core::Database::open(missing.path()).expect("open");
        db.create_collection(
            "_semantic_memory",
            DIM,
            velesdb_core::DistanceMetric::Cosine,
        )
        .expect("create one collection only");
    }
    let partial = diagnose(missing.path(), TARGET_MODEL, TARGET_DIM, None).expect("diagnose");
    let absent = inventory_of(&partial, "_episodic_memory");
    assert!(
        !absent.present,
        "a collection the store does not have must report ABSENT, not empty: the \
         first means the rebuild creates it, the second means it copies nothing"
    );
    assert_eq!(
        absent.dimension, None,
        "an absent collection has no width to report"
    );
    assert!(
        partial
            .blockers
            .iter()
            .any(|b| b.contains("_episodic_memory")),
        "an absent collection must be a named blocker, got {:?}",
        partial.blockers
    );
}

#[test]
fn a_store_without_provenance_reports_unknown() {
    let (dir, _ttl_meta) = seeded();
    assert!(
        !dir.path()
            .join(crate::embedding_provenance::PROVENANCE_FILE)
            .exists(),
        "this fixture is deliberately a store with no embedding record — the case \
         the real store is in"
    );

    let report = diagnose(dir.path(), TARGET_MODEL, TARGET_DIM, None).expect("diagnose");

    match &report.source_provenance {
        SourceProvenance::Unknown { reason } => assert!(
            reason.contains(crate::embedding_provenance::PROVENANCE_FILE),
            "'unknown' must say WHAT was looked for; a bare 'unknown' reads as a \
             failure rather than as the nominal case, got {reason}"
        ),
        SourceProvenance::Known { model, .. } => panic!(
            "a store with no record must not be credited with a model; got '{model}', \
             which nothing on disk supports"
        ),
    }
    assert!(
        report
            .blockers
            .iter()
            .any(|b| b.starts_with("source_provenance:")),
        "unknown provenance is a BLOCKER, not a footnote: it is what makes an \
         equal-width model change undetectable, got {:?}",
        report.blockers
    );

    // The positive control: a store that DOES record its model is reported as
    // known, so the branch above is a real discrimination and not a constant.
    let recorded = tempfile::tempdir().expect("tempdir");
    {
        let _store = NativeStore::open(recorded.path(), DIM).expect("open store");
    }
    crate::embedding_provenance::write(
        recorded.path(),
        &crate::embedding_provenance::EmbeddingProvenance::new("all-minilm", DIM),
    )
    .expect("write provenance");
    let known = diagnose(recorded.path(), TARGET_MODEL, TARGET_DIM, None).expect("diagnose");
    assert_eq!(
        known.source_provenance,
        SourceProvenance::Known {
            model: "all-minilm".to_owned(),
            dimension: DIM
        },
        "a recorded model must come back verbatim"
    );
}

#[test]
fn a_model_change_at_equal_dimension_is_not_claimed_detected() {
    // A store filled by one model, and a target that is a DIFFERENT model of the
    // SAME width. Nothing on disk distinguishes the two, and the danger is a
    // report that implies it does: the vectors would be silently incomparable
    // while every width check passed.
    let (dir, _ttl_meta) = seeded();

    let report = diagnose(dir.path(), "a-different-model", DIM, None).expect("diagnose");

    assert_eq!(
        report.source_dimension,
        Some(DIM),
        "the widths match — which is exactly why the width cannot settle this"
    );
    assert_eq!(report.target_dimension, DIM);
    assert!(
        matches!(report.source_provenance, SourceProvenance::Unknown { .. }),
        "with no record on disk, the source model is unknown and must stay so"
    );
    assert!(
        !report.is_clear(),
        "a report that came back CLEAR here would be telling the operator a \
         same-width model swap is safe, which is the precise failure this gate exists to stop"
    );
    let provenance_blocker = report
        .blockers
        .iter()
        .find(|b| b.starts_with("source_provenance:"))
        .expect("the undetectable-swap blocker must be present");
    assert!(
        provenance_blocker.contains("EQUAL width"),
        "the blocker must name the equal-width case explicitly, got {provenance_blocker}"
    );
}

/// What `after` holds that `before` did not, or holds differently.
fn drift(
    before: &std::collections::BTreeMap<String, (u64, Vec<u8>)>,
    after: &std::collections::BTreeMap<String, (u64, Vec<u8>)>,
) -> Vec<String> {
    let changed = after
        .iter()
        .filter(|(path, state)| before.get(*path) != Some(*state))
        .map(|(path, _)| path.clone());
    let vanished = before
        .keys()
        .filter(|p| !after.contains_key(*p))
        .map(|p| format!("{p} (vanished)"));
    changed.chain(vanished).collect()
}

/// Copy a store directory, file for file.
fn copy_tree(src: &std::path::Path, dst: &std::path::Path) {
    std::fs::create_dir_all(dst).expect("create destination");
    for entry in std::fs::read_dir(src).expect("read_dir") {
        let entry = entry.expect("entry");
        let (from, to) = (entry.path(), dst.join(entry.file_name()));
        if entry.metadata().expect("metadata").is_dir() {
            copy_tree(&from, &to);
        } else {
            std::fs::copy(&from, &to).expect("copy file");
        }
    }
}

#[test]
fn a_diagnostic_does_not_change_the_directory_tree() {
    // The property that protects the operator is that the store they rely on is
    // untouched — not that some directory somewhere is inert. So the diagnosis
    // runs against a CONTROLLED COPY and the ORIGINAL is what gets compared.
    let (original, _ttl_meta) = seeded();
    let before = tree(original.path());
    assert!(
        !before.is_empty(),
        "positive control: an empty 'before' would make any 'after' equal to it"
    );

    let workspace = tempfile::tempdir().expect("tempdir");
    let copy = workspace.path().join("copy");
    copy_tree(original.path(), &copy);
    assert_eq!(
        tree(&copy),
        before,
        "the copy must start out identical, or the comparison below compares nothing"
    );

    let report = diagnose(&copy, TARGET_MODEL, TARGET_DIM, None).expect("diagnose");
    assert!(report.facts > 0, "the diagnosis must actually have read");

    let untouched = drift(&before, &tree(original.path()));
    assert!(
        untouched.is_empty(),
        "diagnosing a copy must leave the ORIGINAL byte-for-byte as it was; drifted: {untouched:?}"
    );

    // ---------------------------------------------------------------------
    // The positive control, and the finding it encodes.
    //
    // Without this, the assertion above would pass just as well if the copy
    // were decorative — if opening a store never wrote anything, diagnosing
    // the original directly would be equally safe and the whole protocol
    // pointless. It is not: `Database::open` rewrites its derived artifacts
    // before a single fact is read. Measured by isolation — the open alone
    // drifts these files, the cursor walk that follows drifts nothing, and a
    // second open of the now-normalised store drifts nothing either.
    // ---------------------------------------------------------------------
    let direct = diagnose(original.path(), TARGET_MODEL, TARGET_DIM, None).expect("diagnose");
    assert_eq!(
        direct.facts, report.facts,
        "the copy and the original must describe the same store"
    );
    let drifted = drift(&before, &tree(original.path()));
    assert!(
        !drifted.is_empty(),
        "the copy protocol is load-bearing only if opening the original DOES \
         write; if this ever comes back empty, the engine changed and this test \
         is the place that says so"
    );
    assert!(
        drifted
            .iter()
            .all(|p| p.contains("native_") || p.contains("vectors.")),
        "only DERIVED index artifacts may drift — a payload or WAL-of-record file \
         drifting would mean the data itself moved; drifted: {drifted:?}"
    );

    // Once normalised, the store is stable: the drift is a one-time cost of the
    // first open after a write session, not a rewrite on every read.
    let normalised = tree(original.path());
    let _ = diagnose(original.path(), TARGET_MODEL, TARGET_DIM, None).expect("diagnose");
    assert!(
        drift(&normalised, &tree(original.path())).is_empty(),
        "a second diagnosis of an already-normalised store must drift nothing"
    );

    // ...and the report says so itself, so PR B reads the constraint rather
    // than rediscovering it.
    assert!(
        matches!(
            report.capabilities.get("source_open_is_read_only"),
            Some(Capability::Missing { .. })
        ),
        "the report must carry the write-on-open hazard as a blocker"
    );
}

#[test]
fn a_dry_run_creates_no_destination_or_state() {
    let (dir, _ttl_meta) = seeded();
    let parent = tempfile::tempdir().expect("tempdir");
    let destination = parent.path().join("rebuilt");

    let report =
        diagnose(dir.path(), TARGET_MODEL, TARGET_DIM, Some(&destination)).expect("diagnose");

    assert!(
        !destination.exists(),
        "naming a destination must not CREATE it — a diagnosis that left a \
         directory behind would be a migration that started without being asked"
    );
    assert_eq!(
        std::fs::read_dir(parent.path())
            .expect("read parent")
            .count(),
        0,
        "nothing at all may appear beside the destination either"
    );
    assert!(
        report.same_filesystem.is_some() || cfg!(not(unix)),
        "on unix the device comparison must actually be answered, not skipped"
    );

    // No migration state anywhere: not in the source, not beside the
    // destination. A state file is a COMMITMENT, and a question does not commit.
    for root in [dir.path(), parent.path()] {
        for entry in std::fs::read_dir(root).expect("read_dir") {
            let name = entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned();
            assert!(
                !name.contains("migration") && !name.contains("state"),
                "a dry run left `{name}` behind in {}",
                root.display()
            );
        }
    }
}

#[test]
fn no_credential_is_serialized() {
    const FAKE_KEY: &str = "sk-ThisIsAFakeCredentialPlantedByTheTest";
    const FAKE_TOKEN: &str = "veles-token-8f3a1c9e-planted";

    let dir = tempfile::tempdir().expect("tempdir");
    {
        let store = NativeStore::open(dir.path(), DIM).expect("open store");
        // A secret in a payload, and a secret in a reserved-key VALUE: the two
        // places a report that copied too much would pick one up.
        store
            .store_with_metadata(
                1,
                &format!("my api key is {FAKE_KEY}"),
                &EMBEDDING,
                &meta(&[("token", Value::from(FAKE_TOKEN))]),
            )
            .expect("seed");
    }
    // ...and a secret in the store's own config file, which sits in the very
    // directory the diagnosis walks to fingerprint it.
    std::fs::write(
        dir.path().join("velesdb-memory.toml"),
        format!("[embedder]\napi_key = \"{FAKE_KEY}\"\n"),
    )
    .expect("write config");

    let report = diagnose(dir.path(), TARGET_MODEL, TARGET_DIM, None).expect("diagnose");
    let serialized = serde_json::to_string(&report).expect("the report must serialize");

    assert!(
        !serialized.contains(FAKE_KEY),
        "the report carries a credential; a diagnosis is written to logs and \
         issue threads, so anything it serializes is disclosed"
    );
    assert!(
        !serialized.contains(FAKE_TOKEN),
        "the report carries a secret from a payload value"
    );
    assert!(
        !serialized.contains("my api key is"),
        "the report carries fact CONTENT; content is where secrets live and the \
         report has no reason to hold any"
    );

    // The positive control. Without it, a `serialize` that returned "{}" — or a
    // `contains` against the wrong haystack — would pass all three assertions
    // above while proving nothing.
    assert!(
        serialized.contains("_semantic_memory"),
        "the search must be capable of finding a string that IS in the report; \
         otherwise its silence is meaningless"
    );
    let leaky = serde_json::json!({ "report": serialized, "key": FAKE_KEY }).to_string();
    assert!(
        leaky.contains(FAKE_KEY),
        "and it must be capable of finding the credential when one is genuinely there"
    );
}

// ---------------------------------------------------------------------------
// GATE 4 — preservation
//
// Reading every fact out is half the question. The other half is whether it
// goes back the SAME: same id, same content, same ordinary and RESERVED
// metadata, the same absolute instant of expiry, the same edges between the
// same endpoints. Every comparison below is against the SOURCE's own values —
// never against a constant this file made up, which would only prove the file
// agrees with itself.
// ---------------------------------------------------------------------------

use super::{reinsert, Reinsertion};

/// The width the new embedder produces — deliberately NOT [`DIM`], because the
/// whole migration exists to move between two widths and a destination sized
/// like the source would hide every place the old vector leaked through.
const NEW_DIM: usize = 8;
const NEW_EMBEDDING: [f32; NEW_DIM] = [0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];

/// An empty destination store, sized for the NEW embedder.
fn destination() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    {
        let _store = NativeStore::open(dir.path(), NEW_DIM).expect("open destination");
    }
    dir
}

/// Walk `collection` out of the store at `dir`, by cursor.
fn read_out(dir: &std::path::Path, collection: &str) -> Vec<RawFact> {
    let db = velesdb_core::Database::open(dir).expect("open source");
    super::enumerate_by_cursor(&db, collection, 1024).expect("cursor walk")
}

#[test]
fn a_fact_round_trips_with_id_metadata_and_ttl() {
    let (source, _ttl_meta) = seeded();
    let out = read_out(source.path(), "_semantic_memory");
    assert!(!out.is_empty(), "positive control: the source must be read");

    let dest = destination();
    {
        let db = velesdb_core::Database::open(dest.path()).expect("open destination");
        for fact in &out {
            assert_eq!(
                reinsert(&db, "_semantic_memory", fact, &NEW_EMBEDDING).expect("reinsert"),
                Reinsertion::Inserted,
                "an empty destination must accept every fact; a collision here \
                 would mean the ids are not what the walk reported"
            );
        }
    }

    let back = read_out(dest.path(), "_semantic_memory");
    let by_id = |facts: &[RawFact]| -> std::collections::BTreeMap<u64, Value> {
        facts
            .iter()
            .map(|f| (f.id, serde_json::from_str(&f.payload).expect("json")))
            .collect()
    };
    let (source_facts, dest_facts) = (by_id(&out), by_id(&back));

    assert_eq!(
        source_facts.keys().collect::<Vec<_>>(),
        dest_facts.keys().collect::<Vec<_>>(),
        "every id must survive verbatim — a renumbered fact severs its edges, \
         its hub and the working-context index that address it BY id"
    );
    assert_eq!(
        source_facts, dest_facts,
        "content, ordinary metadata and RESERVED metadata must come back \
         byte-identical; a stripped `_veles_*` key is a fact the rebuild quietly \
         demoted"
    );

    // ...and the expiry specifically, because it is the one field a plausible
    // implementation would RECOMPUTE from a duration and silently extend.
    let ttl_source = source_facts
        .get(&100)
        .and_then(|p| p.get("_veles_expires_at"))
        .expect("the source ttl fact carries an absolute expiry");
    assert_eq!(
        dest_facts
            .get(&100)
            .and_then(|p| p.get("_veles_expires_at")),
        Some(ttl_source),
        "the expiry must be the SAME absolute instant, not the same duration \
         measured from migration time"
    );
}

#[test]
fn a_collision_has_an_explicit_result() {
    let (source, _ttl_meta) = seeded();
    let out = read_out(source.path(), "_semantic_memory");
    let first = out.iter().find(|f| f.id == 1).expect("fact 1");

    let dest = destination();
    let db = velesdb_core::Database::open(dest.path()).expect("open destination");

    // The positive control comes first: the same call on a free id must succeed,
    // or "collision" below would just be this function failing at everything.
    assert_eq!(
        reinsert(&db, "_semantic_memory", first, &NEW_EMBEDDING).expect("first insert"),
        Reinsertion::Inserted,
        "a free id must accept the fact"
    );

    // Now the same id, carrying DIFFERENT content — the case where a silent
    // overwrite would destroy a fact and report success.
    let intruder = RawFact {
        id: 1,
        payload: serde_json::json!({ "content": "an intruder that must not land" }).to_string(),
    };
    let outcome = reinsert(&db, "_semantic_memory", &intruder, &NEW_EMBEDDING).expect("second");
    match &outcome {
        Reinsertion::Collision { existing } => {
            let stored: Value = serde_json::from_str(existing).expect("json");
            assert_eq!(
                stored.get("content").and_then(Value::as_str),
                Some("fact number 1"),
                "a collision must report what is ALREADY there, so the caller can \
                 tell a re-run from a genuine clash"
            );
        }
        Reinsertion::Inserted => panic!(
            "the second write reported success — meaning `upsert` overwrote fact 1 \
             without a word, which is exactly how a rebuild destroys what it is \
             preserving"
        ),
    }

    // And nothing was written: the fact under that id is untouched.
    drop(db);
    let back = read_out(dest.path(), "_semantic_memory");
    let stored: Value = serde_json::from_str(&back[0].payload).expect("json");
    assert_eq!(
        stored.get("content").and_then(Value::as_str),
        Some("fact number 1"),
        "a refused collision must leave the destination exactly as it was"
    );
}

/// Write a point straight into the collection, expiry included.
///
/// NO published API produces an already-expired fact: `store_with_metadata`
/// STRIPS `_veles_expires_at` out of caller metadata (`build_payload`), and
/// `store_with_ttl(_, 0)` DELETES the fact rather than expiring it. An expired
/// fact is only ever reached by time passing — which a test cannot wait for and
/// must not fake with a sleep. So the fixture writes the on-disk state such a
/// fact actually has: the engine never rewrites a payload when its expiry
/// passes, it filters at read time.
fn seed_raw(dir: &std::path::Path, id: u64, content: &str, expires_at: Option<u64>) {
    let db = velesdb_core::Database::open(dir).expect("open");
    let any = db
        .get_any_collection("_semantic_memory")
        .expect("collection exists");
    let mut payload = serde_json::Map::new();
    payload.insert("content".to_owned(), Value::from(content));
    if let Some(exp) = expires_at {
        payload.insert("_veles_expires_at".to_owned(), Value::from(exp));
    }
    any.upsert(vec![velesdb_core::Point::new(
        id,
        EMBEDDING.to_vec(),
        Some(Value::Object(payload)),
    )])
    .expect("upsert");
}

#[test]
fn expired_points_are_not_resurrected() {
    let dir = tempfile::tempdir().expect("tempdir");
    {
        let store = NativeStore::open(dir.path(), DIM).expect("open store");
        store
            .store_with_metadata(1, "a live fact", &EMBEDDING, &meta(&[]))
            .expect("seed live");
    }
    seed_raw(
        dir.path(),
        2,
        "a fact whose time has passed",
        Some(1_000_000),
    );

    let out = read_out(dir.path(), "_semantic_memory");
    let ids: BTreeSet<u64> = out.iter().map(|f| f.id).collect();
    assert!(
        ids.contains(&1),
        "positive control: the LIVE fact must come back, or this test proves only \
         that the walk returns nothing"
    );
    assert!(
        !ids.contains(&2),
        "an already-expired fact must not be exported; a rebuild that carried it \
         would resurrect a fact the store had already retired, and the new store \
         would hand it back to the caller"
    );

    // And it really is the expiry that excluded it, not the raw write path: the
    // SAME fixture with a FUTURE expiry does come back.
    let future = tempfile::tempdir().expect("tempdir");
    {
        let _store = NativeStore::open(future.path(), DIM).expect("open store");
    }
    seed_raw(
        future.path(),
        2,
        "a fact whose time has not passed",
        Some(4_000_000_000),
    );
    assert!(
        read_out(future.path(), "_semantic_memory")
            .iter()
            .any(|f| f.id == 2),
        "a fact under a FUTURE expiry must be exported — otherwise the exclusion \
         above was about the write, not about the expiry"
    );
}

#[test]
fn cursor_scan_survives_reorder_for_locality() {
    // `reorder_for_locality` rearranges the physical layout. A walk that paged
    // by POSITION would silently change what it returns; a cursor keyed on the
    // id must not. The ids are scrambled and non-contiguous so that physical
    // order and id order cannot coincide by luck.
    let dir = tempfile::tempdir().expect("tempdir");
    {
        let store = NativeStore::open(dir.path(), DIM).expect("open store");
        for id in SCRAMBLED {
            store
                .store_with_metadata(*id, &format!("fact {id}"), &EMBEDDING, &meta(&[]))
                .expect("seed");
        }
    }
    let expected: BTreeSet<u64> = SCRAMBLED.iter().copied().collect();

    let before = read_out(dir.path(), "_semantic_memory");
    let before_ids: Vec<u64> = before.iter().map(|f| f.id).collect();
    assert_eq!(
        before_ids.iter().copied().collect::<BTreeSet<u64>>(),
        expected,
        "positive control: the walk must be complete BEFORE the reorder, or the \
         comparison after it means nothing"
    );

    {
        let db = velesdb_core::Database::open(dir.path()).expect("open");
        db.get_vector_collection("_semantic_memory")
            .expect("the seeded collection is a vector collection")
            .reorder_for_locality()
            .expect("reorder");
    }

    let after = read_out(dir.path(), "_semantic_memory");
    let after_ids: Vec<u64> = after.iter().map(|f| f.id).collect();
    assert_eq!(
        after_ids.iter().copied().collect::<BTreeSet<u64>>(),
        expected,
        "the reorder dropped or duplicated facts under the cursor walk"
    );
    let mut sorted = after_ids.clone();
    sorted.sort_unstable();
    assert_eq!(
        after_ids, sorted,
        "the cursor is keyed on the id and must stay ASCENDING through a \
         reorder — an order that follows the physical layout is one a checkpoint \
         cannot resume from"
    );
    assert_eq!(
        after, before,
        "the reorder must not change a single payload either"
    );
}

#[test]
fn edge_ids_and_endpoints_survive_the_round_trip() {
    // Edges carry no vector of their own and are addressed by a triplet. If a
    // rebuild re-relates the same endpoints under the same label and gets a
    // DIFFERENT id, every stored reference to that edge is severed.
    let source = tempfile::tempdir().expect("tempdir");
    let mut expected: Vec<(u64, u64, u64, String)> = Vec::new();
    // Both directions between the same pair, and two labels on the same
    // direction: the four cases an id derived from an unordered pair, or from
    // the endpoints alone, would collapse together.
    let triplets = [
        (1_u64, 2_u64, "mentions"),
        (2, 1, "mentions"),
        (1, 2, "contradicts"),
        (1, 3, "mentions"),
    ];
    {
        let store = NativeStore::open(source.path(), DIM).expect("open source");
        for id in 1..=3_u64 {
            store
                .store_with_metadata(id, &format!("fact {id}"), &EMBEDDING, &meta(&[]))
                .expect("seed");
        }
        for (from, to, label) in triplets {
            let edge_id = store.relate(from, to, label).expect("relate");
            expected.push((edge_id, from, to, label.to_owned()));
        }
    }
    assert_eq!(
        expected
            .iter()
            .map(|(id, ..)| *id)
            .collect::<BTreeSet<u64>>()
            .len(),
        triplets.len(),
        "positive control: the four triplets must yield four DISTINCT edge ids, \
         or the comparison below cannot tell them apart"
    );

    let out = read_out(source.path(), "_semantic_memory");
    let dest = destination();
    {
        let db = velesdb_core::Database::open(dest.path()).expect("open destination");
        for fact in &out {
            reinsert(&db, "_semantic_memory", fact, &NEW_EMBEDDING).expect("reinsert");
        }
    }

    // Re-relate the same triplets in the destination and compare ids.
    let mut rebuilt: Vec<(u64, u64, u64, String)> = Vec::new();
    {
        let store = NativeStore::open(dest.path(), NEW_DIM).expect("open destination store");
        for (from, to, label) in triplets {
            let edge_id = store.relate(from, to, label).expect("re-relate");
            rebuilt.push((edge_id, from, to, label.to_owned()));
        }
        // ...and the endpoints as the destination reports them back.
        for (edge_id, from, to, label) in &expected {
            let found = store
                .relations(*from)
                .expect("relations")
                .into_iter()
                .find(|e| e.id == *edge_id)
                .unwrap_or_else(|| {
                    panic!("edge {edge_id} ({from}->{to} :{label}) is missing from the destination")
                });
            assert_eq!(
                (found.from, found.to, found.relation.as_str()),
                (*from, *to, label.as_str()),
                "the edge came back under the right id but the wrong endpoints \
                 or label"
            );
        }
    }
    assert_eq!(
        rebuilt, expected,
        "re-relating the same triplet must yield the SAME edge id; a different \
         one severs every reference the store holds to that edge"
    );
}

// ---------------------------------------------------------------------------
// GATE 5 — the lock and the phase journal
//
// A rebuild can stop anywhere. What has to hold is not that it never stops, but
// that every place it CAN stop has one defined action, and that a stop whose
// meaning the disk does not determine changes nothing at all.
// ---------------------------------------------------------------------------

use super::{MigrationLock, MigrationState, Phase, Recovery, SwitchState, PHASES};

/// A state that would resume cleanly, so each test can change exactly one thing
/// and attribute the refusal to it.
fn resumable_state() -> MigrationState {
    MigrationState {
        format_version: super::STATE_FORMAT_VERSION,
        phase: Phase::Prepared,
        source_path: std::path::PathBuf::from("/store"),
        source_fingerprint: "fnv1a64:0123456789abcdef".to_owned(),
        target_model: TARGET_MODEL.to_owned(),
        target_dimension: TARGET_DIM,
    }
}

/// The explanation a recovery carries, whichever action it names.
fn stated(recovery: &Recovery) -> &str {
    match recovery {
        Recovery::Continue { rationale, .. } => rationale,
        Recovery::Restore { action } => action,
        Recovery::Refuse { reason } => reason,
    }
}

/// Every switch layout that does NOT determine what happened.
const AMBIGUOUS: &[(bool, bool, bool)] = &[
    (true, true, true),
    (true, true, false),
    (false, false, true),
    (false, false, false),
];

/// Every switch layout that DOES.
const DECIDABLE: &[(bool, bool, bool)] = &[
    (false, true, true),
    (false, true, false),
    (true, false, true),
    (true, false, false),
];

fn switch(triple: (bool, bool, bool)) -> SwitchState {
    SwitchState {
        source: triple.0,
        archive: triple.1,
        destination: triple.2,
    }
}

#[test]
fn two_migrations_cannot_hold_the_lock() {
    let workspace = tempfile::tempdir().expect("tempdir");

    // (1) a free lock is taken.
    let first = MigrationLock::acquire(workspace.path(), "run-A").expect("the first must succeed");

    // (2) a second acquisition fails while the first is held...
    let refusal = MigrationLock::acquire(workspace.path(), "run-B")
        .expect_err("two migrations must not hold one workspace");
    // (3) ...and the refusal names who has it.
    assert!(
        refusal.contains("run-A"),
        "the refusal must name the holder, or an operator cannot tell a stale \
         lock from a live one: {refusal}"
    );
    assert!(
        refusal.contains("delete"),
        "the refusal must say what a human can do about it: {refusal}"
    );

    // (6) and it is REFUSED, not stolen: no pid, no port, no liveness check.
    assert!(
        !refusal.to_lowercase().contains("pid")
            || refusal.contains("no process id or port is consulted"),
        "the lock must not be broken on a liveness check: {refusal}"
    );
    assert!(
        workspace.path().join(super::LOCK_FILE).exists(),
        "a refused acquisition must leave the existing lock exactly where it was"
    );

    // (4) releasing frees it, and (5) it can then be taken again.
    first.release().expect("release");
    assert!(
        !workspace.path().join(super::LOCK_FILE).exists(),
        "release must remove the lock file"
    );
    let second =
        MigrationLock::acquire(workspace.path(), "run-B").expect("a released lock is free again");

    // The positive control for the refusal above: without this, an `acquire`
    // that always failed would satisfy every assertion so far.
    assert_eq!(
        MigrationLock::holder(workspace.path()).as_deref(),
        Some("held_by=run-B"),
        "the lock must record its new holder"
    );
    second.release().expect("release");

    // (6) again, deliberately: a lock left behind by a process that is gone is
    // still refused. Nothing here is alive, and that changes nothing.
    std::fs::write(
        workspace.path().join(super::LOCK_FILE),
        "held_by=a-dead-run\n",
    )
    .expect("plant a stale lock");
    let stale = MigrationLock::acquire(workspace.path(), "run-C")
        .expect_err("a stale lock must be refused, never stolen");
    assert!(
        stale.contains("a-dead-run"),
        "the refusal must name the holder recorded in the stale lock: {stale}"
    );
}

#[test]
fn the_lock_never_lives_in_the_source() {
    // Property (7). The diagnosis contract is that the source is not written
    // to; a lock file placed there would make the act of asking a write.
    let (source, _ttl) = seeded();
    let before = tree(source.path());
    let workspace = tempfile::tempdir().expect("tempdir");

    let lock = MigrationLock::acquire(workspace.path(), "run-A").expect("acquire");

    assert!(
        workspace.path().join(super::LOCK_FILE).exists(),
        "positive control: the lock must actually have been created somewhere"
    );
    assert!(
        !source.path().join(super::LOCK_FILE).exists(),
        "the migration lock must never be placed in the source"
    );
    assert!(
        drift(&before, &tree(source.path())).is_empty(),
        "taking the lock must not touch the source at all"
    );
    lock.release().expect("release");
}

#[test]
fn a_newer_state_version_is_refused() {
    let workspace = tempfile::tempdir().expect("tempdir");

    // The positive control first: a state at THIS version reads back.
    resumable_state().write(workspace.path()).expect("write");
    let read = MigrationState::read(workspace.path())
        .expect("a state at the current version must read")
        .expect("it exists");
    assert_eq!(read, resumable_state(), "a state must round-trip verbatim");

    // Now a state from the future, carrying a field this build knows nothing
    // about — the shape a newer version would genuinely have.
    let newer = serde_json::json!({
        "format_version": super::STATE_FORMAT_VERSION + 1,
        "phase": "prepared",
        "source_path": "/store",
        "source_fingerprint": "fnv1a64:0123456789abcdef",
        "target_model": TARGET_MODEL,
        "target_dimension": TARGET_DIM,
        "a_field_from_the_future": { "that": "this build cannot interpret" },
    });
    std::fs::write(
        workspace.path().join(super::STATE_FILE),
        serde_json::to_string_pretty(&newer).expect("json"),
    )
    .expect("write newer state");

    let refusal = MigrationState::read(workspace.path())
        .expect_err("a state from a newer version must be refused");
    assert!(
        refusal.contains(&format!("version {}", super::STATE_FORMAT_VERSION + 1)),
        "the refusal must name the version it found: {refusal}"
    );
    assert!(
        !refusal.contains("does not parse"),
        "the refusal must be about the VERSION, not about a parse failure — a \
         newer state is expected to carry fields this build cannot read, and \
         reporting that as corruption would send the operator after the wrong \
         problem: {refusal}"
    );

    // ...and the same refusal is reachable without going through the file, so
    // an in-memory state cannot bypass it.
    let mut from_future = resumable_state();
    from_future.format_version = super::STATE_FORMAT_VERSION + 1;
    assert!(
        from_future
            .may_resume(&from_future.source_fingerprint.clone(), TARGET_MODEL)
            .is_err(),
        "may_resume must refuse a newer version too"
    );
}

#[test]
fn a_changed_source_fingerprint_refuses_resume() {
    let state = resumable_state();

    // Positive control: the unchanged fingerprint resumes.
    state
        .may_resume(&state.source_fingerprint, TARGET_MODEL)
        .expect("an unchanged source must resume");

    let refusal = state
        .may_resume("fnv1a64:ffffffffffffffff", TARGET_MODEL)
        .expect_err("a source that changed under a prepared migration must refuse");
    assert!(
        refusal.contains(&state.source_fingerprint) && refusal.contains("ffffffffffffffff"),
        "the refusal must name BOTH fingerprints — one of them alone leaves the \
         operator guessing which side moved: {refusal}"
    );

    // And the fingerprint really is sensitive to a changed store, or the check
    // above guards nothing.
    let (dir, _ttl) = seeded();
    let before = super::fingerprint(dir.path()).expect("fingerprint");
    std::fs::write(dir.path().join("a-new-file"), b"something").expect("write");
    let after = super::fingerprint(dir.path()).expect("fingerprint");
    assert_ne!(
        before, after,
        "the fingerprint must move when the store does; a constant would make \
         every resume look safe"
    );
    assert_eq!(
        super::fingerprint(dir.path()).expect("fingerprint"),
        after,
        "and it must be stable when the store is not — a fingerprint that \
         changed on its own would refuse every legitimate resume"
    );
}

#[test]
fn a_changed_target_model_refuses_resume() {
    let state = resumable_state();

    // Positive control.
    state
        .may_resume(&state.source_fingerprint, TARGET_MODEL)
        .expect("the prepared model must resume");

    let refusal = state
        .may_resume(&state.source_fingerprint, "some-other-model")
        .expect_err("a migration prepared for one model must not resume against another");
    assert!(
        refusal.contains(TARGET_MODEL) && refusal.contains("some-other-model"),
        "the refusal must name both models: {refusal}"
    );
    assert!(
        refusal.contains("not searchable") || refusal.contains("Half"),
        "the refusal must say WHY — half a store embedded by one model and half \
         by another is the failure, and it is invisible at read time: {refusal}"
    );
}

#[test]
fn every_phase_has_an_explicit_recovery_action() {
    assert_eq!(
        PHASES.len(),
        5,
        "the five phases are Prepared, DestinationValidated, SourceArchived, \
         DestinationActivated, Committed"
    );

    for phase in PHASES {
        let recovery = phase.recovery();
        // Each action must carry a stated reason, not just a verdict: a bare
        // 'Continue' is something an operator has to trust rather than check.
        let stated = stated(&recovery);
        assert!(
            stated.len() > 40,
            "{phase:?} has an action with no usable explanation: {stated:?}"
        );
    }

    // The actions are not all the same — a `recovery()` that returned one
    // constant would satisfy the loop above while deciding nothing.
    let distinct: BTreeSet<String> = PHASES
        .iter()
        .map(|p| format!("{:?}", p.recovery()))
        .collect();
    assert_eq!(
        distinct.len(),
        PHASES.len(),
        "each phase must have its OWN action; identical ones mean the phase was \
         not actually considered"
    );

    // The two that matter most, named rather than inferred: the phase where the
    // source has been moved aside and nothing replaced it must RESTORE, and the
    // finished migration must refuse to run again.
    assert!(
        matches!(Phase::SourceArchived.recovery(), Recovery::Restore { .. }),
        "with the source archived and the destination not yet activated, the \
         source is the only authority and must go back"
    );
    assert!(
        matches!(Phase::Committed.recovery(), Recovery::Refuse { .. }),
        "a finished migration has nothing to resume"
    );
    assert!(
        matches!(
            Phase::DestinationActivated.recovery(),
            Recovery::Continue {
                next: Phase::Committed,
                ..
            }
        ),
        "once the destination is live, going BACK would discard the store the \
         caller is already reading from"
    );
}

#[test]
fn an_ambiguous_switch_state_changes_nothing() {
    let layouts = SwitchState::all();
    assert_eq!(
        layouts.len(),
        8,
        "three directories, present or not, is eight states — an enumeration \
         that missed one would leave a disk layout with no defined action"
    );
    assert_eq!(
        layouts
            .iter()
            .map(|l| (l.source, l.archive, l.destination))
            .collect::<BTreeSet<_>>()
            .len(),
        8,
        "the eight must be distinct"
    );
    for layout in &layouts {
        let recovery = layout.recovery();
        let stated = stated(&recovery);
        assert!(
            stated.len() > 40,
            "{layout:?} has no usable explanation: {stated:?}"
        );
    }

    // The layouts no sequence of this migration produces — or that two
    // different histories both produce — must REFUSE.
    for triple in AMBIGUOUS {
        let layout = switch(*triple);
        assert!(
            matches!(layout.recovery(), Recovery::Refuse { .. }),
            "{layout:?} does not determine what happened and must change nothing"
        );
    }

    // The positive control: "refuse everything" would satisfy the loop above
    // while making every interrupted migration unrecoverable.
    for triple in DECIDABLE {
        let layout = switch(*triple);
        assert!(
            !matches!(layout.recovery(), Recovery::Refuse { .. }),
            "{layout:?} DOES determine what happened; refusing it would strand a \
             recoverable migration"
        );
    }
}

#[test]
fn deciding_what_to_do_does_not_already_do_it() {
    // Separating the decision from the action is what keeps a WRONG decision
    // from having destroyed the evidence before anyone reads it. Every layout
    // is asked against a real directory holding all three names.
    let workspace = tempfile::tempdir().expect("tempdir");
    for name in ["store", "store.archive", "store.rebuilt"] {
        std::fs::create_dir(workspace.path().join(name)).expect("create");
        std::fs::write(workspace.path().join(name).join("data"), name).expect("write");
    }
    let before = tree(workspace.path());
    assert_eq!(before.len(), 3, "positive control: three files must exist");

    for layout in &SwitchState::all() {
        let _ = layout.recovery();
    }

    assert!(
        drift(&before, &tree(workspace.path())).is_empty(),
        "deciding must not delete, rename or create anything"
    );
}

#[test]
fn a_batch_reinsertion_loses_no_id_reserved_key_or_ttl() {
    // Batching is where a fact goes missing without anything failing: the write
    // succeeds, the count looks plausible, and one payload came back thinner
    // than it went in. So the batch is compared to the source field by field,
    // exactly as the one-at-a-time round trip is.
    let (source, _ttl_meta) = seeded();
    let out = read_out(source.path(), "_semantic_memory");
    assert!(
        out.len() > 1,
        "positive control: a batch needs several facts"
    );

    let dest = destination();
    let batch: Vec<(RawFact, Vec<f32>)> = out
        .iter()
        .map(|f| (f.clone(), NEW_EMBEDDING.to_vec()))
        .collect();
    {
        let db = velesdb_core::Database::open(dest.path()).expect("open destination");
        let outcome = super::reinsert_batch(&db, "_semantic_memory", &batch).expect("batch");
        assert_eq!(
            outcome.inserted,
            out.len() as u64,
            "every fact of the batch must land; a short count is the loss this \
             test exists to catch"
        );
        assert!(
            outcome.collisions.is_empty(),
            "an empty destination has nothing to collide with, got {:?}",
            outcome.collisions
        );
    }

    let by_id = |facts: &[RawFact]| -> std::collections::BTreeMap<u64, Value> {
        facts
            .iter()
            .map(|f| (f.id, serde_json::from_str(&f.payload).expect("json")))
            .collect()
    };
    assert_eq!(
        by_id(&read_out(dest.path(), "_semantic_memory")),
        by_id(&out),
        "a batched write must preserve exactly what a single write does — every \
         id, every reserved key, every absolute expiry"
    );

    // A batch carrying one occupied id must land the REST and overwrite nothing.
    let mixed = destination();
    {
        let db = velesdb_core::Database::open(mixed.path()).expect("open");
        let first = out.iter().find(|f| f.id == 1).expect("fact 1").clone();
        super::reinsert_batch(&db, "_semantic_memory", &[(first, NEW_EMBEDDING.to_vec())])
            .expect("seed one");
        let outcome = super::reinsert_batch(&db, "_semantic_memory", &batch).expect("batch");
        assert_eq!(
            outcome.collisions,
            vec![1],
            "the occupied id must be reported, and only it"
        );
        assert_eq!(
            outcome.inserted,
            out.len() as u64 - 1,
            "one collision must not cost the batch its other facts"
        );
    }
    assert_eq!(
        by_id(&read_out(mixed.path(), "_semantic_memory")),
        by_id(&out),
        "and the collided fact must be the one already there, unchanged"
    );
}

#[test]
fn the_embedding_cost_is_declared_unestablished_rather_than_guessed() {
    let (dir, _ttl) = seeded();
    let report = diagnose(dir.path(), TARGET_MODEL, TARGET_DIM, None).expect("diagnose");

    let Some(Capability::Missing { blocker }) = report.capabilities.get("embedder_cost") else {
        panic!(
            "the embedding cost must appear as an explicit MISSING capability. \
             Omitting it would read as 'nothing to worry about', and claiming it \
             Proven would be a number nobody measured. Got: {:?}",
            report.capabilities.get("embedder_cost")
        );
    };
    assert!(
        blocker.contains("16.3 us/fact"),
        "the blocker must say what WAS measured, so the unmeasured part is not \
         confused with the measured one: {blocker}"
    );
    assert!(
        blocker.contains("Ollama") || blocker.contains("network"),
        "and it must say why a unit test cannot supply it: {blocker}"
    );
}
