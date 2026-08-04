use super::enumeration::{enumerate_page, scrambled_store};
use super::*;

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

/// The cursor's per-fact cost across the SAME sizes, measured beside the walk
/// above rather than assumed from it.
///
/// `paging_cost_grows_faster_than_the_store` measures the `OFFSET` side and
/// concludes it is quadratic. The cursor side — the walk the rebuild is meant to
/// use — had no multi-size measurement anywhere: the only cursor timing in this
/// file is a single size inside a correctness test. So "the cursor scales" was
/// an expectation, not a result, and this module was measuring one walk while
/// asserting about the other.
///
/// **No threshold is written down here.** The claim is a comparison between two
/// quantities measured in the same run, on the same stores: the cursor's growth
/// in per-fact cost must not exceed the `OFFSET` walk's. A constant would age
/// with the machine and would have to be re-tuned by whoever it eventually
/// failed for; a comparison between two measurements does not.
///
/// The cursor is timed FIRST at each size, so the page cache is warm for the
/// `OFFSET` walk and not for it. That biases the comparison AGAINST the
/// conclusion this test expects, which is the direction a measurement should
/// lean when the person writing it already has an expectation.
///
/// Measured 2026-08-05, aarch64-apple-darwin, debug profile, machine at rest:
///
/// | facts | cursor   | µs/fact | offset walk | µs/fact |
/// |-------|----------|---------|-------------|---------|
/// | 250   | 2.55 ms  | 10.2    | 9.87 ms     | 39.5    |
/// | 500   | 4.20 ms  | 8.4     | 24.12 ms    | 48.2    |
/// | 1 000 | 10.04 ms | 10.0    | 66.52 ms    | 66.5    |
/// | 2 000 | 30.89 ms | 15.4    | 242.21 ms   | 121.1   |
///
/// Over an eightfold volume the per-fact cost grew ×1.52 for the cursor and
/// ×3.07 for the `OFFSET` walk.
///
/// The cursor is NOT flat, and this file should not be read as saying it is. At
/// 2 000 facts a fact costs half again what it cost at 250. What the numbers
/// support is narrower and enough: the cursor degrades markedly less than the
/// walk it replaces.
///
/// One caveat on the ×1.52, stated because the figure invites over-reading: the
/// 250-fact point is the most polluted by fixed overhead — it is the only size
/// whose per-fact cost is HIGHER than the next one up (10.2 against 8.4). Taking
/// 500 as the baseline instead gives ×1.83. The growth ratio is therefore
/// sensitive to which end you anchor it on, which is why the assertion below
/// compares two ratios computed the same way rather than testing either against
/// a number.
#[test]
#[ignore = "seeds thousands of facts; run deliberately, on a machine at rest"]
fn the_cursor_cost_per_fact_does_not_grow_like_the_offset_walk() {
    let mut cursor_costs: Vec<f64> = Vec::new();
    let mut offset_costs: Vec<f64> = Vec::new();
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
        let expected = usize::try_from(n).expect("fits");

        let start = std::time::Instant::now();
        let walked = super::enumerate_by_cursor(&db, "_semantic_memory", 100).expect("cursor walk");
        let cursor_elapsed = start.elapsed();
        assert_eq!(
            walked.len(),
            expected,
            "positive control: a cursor walk that returned the wrong count would \
             make its timing meaningless"
        );

        let start = std::time::Instant::now();
        let paged = enumerate_collection(&db, "_semantic_memory", 100).expect("page walk");
        let offset_elapsed = start.elapsed();
        assert_eq!(paged.len(), expected, "positive control for the page walk");

        let (cursor, offset) = (per_fact(cursor_elapsed, n), per_fact(offset_elapsed, n));
        println!(
            "  n={n:5}  cursor={cursor_elapsed:>9.2?} ({cursor:>7.1} us/fact)  \
             offset={offset_elapsed:>9.2?} ({offset:>7.1} us/fact)"
        );
        cursor_costs.push(cursor);
        offset_costs.push(offset);
    }

    let growth = |costs: &[f64]| -> f64 {
        let first = costs.first().copied().expect("measured");
        let last = costs.last().copied().expect("measured");
        last / first
    };
    let (cursor_growth, offset_growth) = (growth(&cursor_costs), growth(&offset_costs));
    println!(
        "  per-fact cost grew x{cursor_growth:.2} for the cursor, \
         x{offset_growth:.2} for the offset walk, over an 8x volume"
    );
    assert!(
        cursor_growth <= offset_growth,
        "the cursor's per-fact cost grew at least as fast as the OFFSET walk's \
         (cursor x{cursor_growth:.2}, offset x{offset_growth:.2}). The rebuild is \
         built on the cursor precisely because the other walk is quadratic; if \
         they now degrade alike, that premise no longer holds and the choice has \
         to be re-argued rather than inherited"
    );
}

/// What the two rebuild regimes cost per fact, measured instead of inferred.
///
/// `embedder_cost` is [`Capability::Missing`], and its blocker text quotes
/// `16.3 us/fact to re-insert`. That is the REINSERTION cost, measured on the
/// store, in the regime where the embedder is never called at all. Reading it as
/// an embedder cost is what makes "the embedder dominates a rebuild" look
/// established when it is regime-dependent.
///
/// `reinsert` takes the vector FROM THE CALLER. A rebuild that does not change
/// the embedding model can therefore replay the source vectors and never embed
/// anything; the embedder only dominates when the model CHANGES. Both unit costs
/// are measured here, in the same run, and printed side by side.
///
/// What this does NOT do is compose them into a rebuild duration. No rebuild is
/// callable — the module has no consumer — so a total would be an extrapolation
/// wearing a measurement's clothes. It is also a one-call-per-fact figure: the
/// backend accepts batched requests, and a batched re-embedding would not cost
/// this.
///
/// A live backend is required, and `OllamaEmbedder::new` probes the dimension on
/// construction, so an absent one fails here loudly rather than leaving a cost
/// test quietly passing on nothing.
///
/// Measured 2026-08-05, aarch64-apple-darwin, debug profile, machine at rest,
/// bge-m3 at 1024 dimensions, 32 facts, one call per fact:
///
/// | regime                          | per fact   |
/// |---------------------------------|------------|
/// | re-embedding (model CHANGED)    | 88 462 µs  |
/// | reinsertion (vectors REUSED)    |  3 900 µs  |
/// | ratio                           | ×23        |
///
/// **×23, not orders of magnitude — and the ratio depends on the payload.**
/// Comparing an embedding time against the `16.3 us/fact` in the blocker text
/// suggests a factor near ten thousand. It is wrong on both terms.
///
/// Probed the same day, same 1024 dimensions, payload carrying NO text:
/// reinsertion drops to 460 µs/fact and the ratio rises to ×194. So the
/// dominant term in reinsertion is not the vector width — it is the BM25 text
/// WAL, which the table in `the_per_point_write_cost_is_attributed_to_payload_or_vector`
/// already attributes at `DIM = 4` (3 457.8 µs/fact with text against 15.1
/// without). Decomposed from the three measurements: text indexing ≈ 3 440
/// µs/fact, vector width from 4 to 1024 ≈ 445, the bare write ≈ 15.
///
/// The blocker's 16.3 is therefore a no-text, `DIM = 4` figure. Agent facts
/// always carry `content`, so for this product it understates reinsertion by
/// roughly two hundred fold, and quoting it beside an embedding time compounds
/// that with the mislabelling.
///
/// The operative number here is ×23: a rebuild that changes the model pays
/// about twenty times one that replays its vectors. That is a real penalty and
/// an affordable one. Ten thousand would have made a model change prohibitive,
/// and deciding otherwise on that figure is the mistake this measurement exists
/// to prevent.
///
/// Both figures are debug-profile and one-call-per-fact. Release timings and a
/// batched embedding request would both move them, in the same direction but not
/// by the same amount, which is why the assertion below compares the two
/// measurements to each other and not to any number written here.
#[test]
#[cfg(feature = "ollama")]
#[ignore = "needs a live embedding backend; run deliberately, on a machine at rest"]
fn the_embedder_dominates_only_when_the_model_changes() {
    use crate::embedder::{Embedder, OllamaEmbedder};

    const FACTS: usize = 32;
    let n = u64::try_from(FACTS).expect("fits");
    let texts: Vec<String> = (0..FACTS)
        .map(|i| format!("fact number {i}: what a rebuild has to carry across"))
        .collect();

    let url = std::env::var("VELESDB_MEMORY_OLLAMA_URL")
        .unwrap_or_else(|_| "http://localhost:11434".to_owned());
    let model =
        std::env::var("VELESDB_MEMORY_OLLAMA_MODEL").unwrap_or_else(|_| "bge-m3".to_owned());
    let embedder = OllamaEmbedder::new(&url, &model).unwrap_or_else(|e| {
        panic!(
            "this test measures a REAL embedder and {url} / {model} is unreachable ({e}); \
             it must fail here rather than report a cost for a backend that is not there"
        )
    });
    let dimension = embedder.dimension();
    // The first call pays model load. Timing it would spread a one-off over
    // every fact and inflate the very figure this test exists to establish.
    embedder.embed("warm up").expect("warm up");

    let start = std::time::Instant::now();
    let vectors: Vec<Vec<f32>> = texts
        .iter()
        .map(|text| embedder.embed(text).expect("embed"))
        .collect();
    let embed_elapsed = start.elapsed();

    let batch: Vec<(RawFact, Vec<f32>)> = vectors
        .into_iter()
        .enumerate()
        .map(|(i, vector)| {
            let id = u64::try_from(i).expect("fits") + 1;
            let payload = serde_json::json!({ "content": texts[i] }).to_string();
            (RawFact { id, payload }, vector)
        })
        .collect();

    let dir = tempfile::tempdir().expect("tempdir");
    {
        let _store = NativeStore::open(dir.path(), dimension).expect("open destination");
    }
    let db = database(&dir);
    let start = std::time::Instant::now();
    let written = reinsert_batch(&db, "_semantic_memory", &batch).expect("reinsert");
    let reinsert_elapsed = start.elapsed();
    assert_eq!(
        written.inserted, n,
        "positive control: a batch that did not land would make its timing meaningless"
    );

    let embed_cost = per_fact(embed_elapsed, n);
    let reinsert_cost = per_fact(reinsert_elapsed, n);
    println!("  model={model}  dimension={dimension}  facts={FACTS}");
    println!(
        "  re-embedding (model CHANGED):   {embed_elapsed:>10.2?}  {embed_cost:>10.1} us/fact"
    );
    println!("  reinsertion  (vectors REUSED):  {reinsert_elapsed:>10.2?}  {reinsert_cost:>10.1} us/fact");
    println!(
        "  embedding costs x{:.0} what reinsertion costs, per fact",
        embed_cost / reinsert_cost
    );
    assert!(
        embed_cost > reinsert_cost,
        "re-embedding was not more expensive than reinsertion (embed \
         {embed_cost:.1} us/fact, reinsert {reinsert_cost:.1} us/fact). The whole \
         reason a rebuild reuses source vectors when the model is unchanged is \
         that it is not; if that stops holding, the choice has to be re-argued \
         rather than inherited"
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
const OFFSET_CEILING: u64 = 100_000;
const SEEDED_ABOVE_CEILING: u64 = OFFSET_CEILING + 1;
const CEILING_PAGE: usize = 10;

fn seed_store_above_offset_ceiling() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let seed_start = std::time::Instant::now();
    {
        let store = NativeStore::open(dir.path(), DIM).expect("open");
        for id in 1..=SEEDED_ABOVE_CEILING {
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
        "  seeded {SEEDED_ABOVE_CEILING} facts in {:?}",
        seed_start.elapsed()
    );
    dir
}

fn assert_offset_walk_stops_at_ceiling(db: &velesdb_core::Database) {
    let straddling = enumerate_page(
        db,
        "_semantic_memory",
        CEILING_PAGE,
        usize::try_from(OFFSET_CEILING - 5).expect("fits"),
    );
    let past = enumerate_page(
        db,
        "_semantic_memory",
        CEILING_PAGE,
        usize::try_from(OFFSET_CEILING).expect("fits"),
    );
    println!(
        "  OFFSET page at {} -> {} rows (asked {CEILING_PAGE})",
        OFFSET_CEILING - 5,
        straddling.len()
    );
    println!(
        "  OFFSET page at {OFFSET_CEILING} -> {} rows (fact {SEEDED_ABOVE_CEILING} exists)",
        past.len()
    );
    assert!(
        straddling.len() < CEILING_PAGE,
        "expected the cap to SHORTEN a straddling page; it returned a full page \
         of {}, so the executor no longer clamps `limit + offset`",
        straddling.len()
    );
    assert!(
        past.is_empty(),
        "expected the cap to EMPTY a page at the ceiling; it returned {} rows",
        past.len()
    );
}

fn assert_cursor_crosses_offset_ceiling(db: &velesdb_core::Database) {
    let cursor_start = std::time::Instant::now();
    let by_cursor =
        super::enumerate_by_cursor(db, "_semantic_memory", 10_000).expect("cursor enumeration");
    let cursor_elapsed = cursor_start.elapsed();
    let ids: BTreeSet<u64> = by_cursor.iter().map(|f| f.id).collect();
    println!(
        "  CURSOR walk -> {} rows in {cursor_elapsed:?} ({:.1} us/fact)",
        by_cursor.len(),
        per_fact(cursor_elapsed, SEEDED_ABOVE_CEILING)
    );
    assert_eq!(
        by_cursor.len(),
        ids.len(),
        "the cursor walk returned a fact twice"
    );
    assert_eq!(
        ids.len(),
        usize::try_from(SEEDED_ABOVE_CEILING).expect("fits"),
        "the cursor walk must reach EVERY fact past the cap that bounds the \
         OFFSET walk; it stopped at {} of {SEEDED_ABOVE_CEILING}",
        ids.len()
    );
    assert!(
        ids.contains(&SEEDED_ABOVE_CEILING),
        "the fact beyond the cap is precisely the one the OFFSET walk drops; the \
         cursor must carry it"
    );
}

#[test]
#[ignore = "seeds 100_001 facts; run deliberately, on a machine at rest"]
fn past_the_ceiling_the_offset_walk_truncates_and_the_cursor_does_not() {
    let dir = seed_store_above_offset_ceiling();
    let db = database(&dir);
    assert_offset_walk_stops_at_ceiling(&db);
    assert_cursor_crosses_offset_ceiling(&db);
}
