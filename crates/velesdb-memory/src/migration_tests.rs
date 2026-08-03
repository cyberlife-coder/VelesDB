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
//! offset 100_000, while the cursor — which an earlier reading of this same
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
/// grid cannot answer. At a constant ~3.4 ms per point a million-fact rebuild
/// is slow but finite; if the per-fact cost RISES with the volume, the rebuild
/// does not scale and `scalable_reconstruction` is `Missing` however good the
/// chunk size looks at 2 000 facts.
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
