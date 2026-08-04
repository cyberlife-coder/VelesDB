use super::*;

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
pub(super) fn scrambled_store() -> (tempfile::TempDir, BTreeSet<u64>, usize) {
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
pub(super) fn enumerate_page(
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

/// Both walks must exclude exactly the SAME expired facts.
///
/// They filter with the same predicate (`is_payload_expired`) and the same clock
/// function (`now_unix_secs`), but at two different sites: `collection/core/scroll.rs`
/// for the cursor, `search/query/similarity_filter.rs` for the scan. The predicate
/// is therefore not where they can disagree.
///
/// Pagination is. `execute_scan_query` collects the first `limit + offset` LIVE
/// points in PHYSICAL order and only then lets `ORDER BY id` sort them, so
/// excluding an expired point shifts the physical window and not merely the
/// sorted list. With ids that are neither contiguous nor written in ascending
/// order, and expired facts interleaved so that no page boundary lands on a run
/// of one kind, a walk that paged by position would show a gap or a repeat here.
///
/// `expired_points_are_not_resurrected` already pins the cursor side. This is
/// the missing half: that the independent verification path agrees with it.
#[test]
fn both_walks_exclude_exactly_the_same_expired_facts() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut live: Vec<u64> = Vec::new();
    let mut expired: Vec<u64> = Vec::new();
    {
        let store = NativeStore::open(dir.path(), DIM).expect("open store");
        for (n, id) in SCRAMBLED.iter().copied().enumerate() {
            if n % 2 == 0 {
                store
                    .store_with_metadata(id, &format!("live {id}"), &EMBEDDING, &meta(&[]))
                    .expect("seed live");
                live.push(id);
            } else {
                expired.push(id);
            }
        }
    }
    // A past instant, not `now`: the published zero-ttl route stamps
    // `expires_at = now` and the predicate is `exp <= now`, which is expired but
    // sits on the second boundary. A fixture must not race the clock.
    for id in &expired {
        super::preservation::seed_raw(dir.path(), *id, &format!("expired {id}"), Some(1_000_000));
    }

    let db = velesdb_core::Database::open(dir.path()).expect("open source");
    let ids = |facts: Vec<RawFact>| -> Vec<u64> { facts.iter().map(|f| f.id).collect() };
    let by_cursor = ids(enumerate_by_cursor(&db, "_semantic_memory", PAGE).expect("cursor walk"));
    let by_offset = ids(enumerate_collection(&db, "_semantic_memory", PAGE).expect("offset walk"));
    drop(db);

    let mut only_live = live.clone();
    only_live.sort_unstable();
    assert_eq!(
        by_cursor, only_live,
        "positive control: the cursor must return every LIVE id and nothing else, \
         or the comparison below would only prove two empty walks agree"
    );
    assert_eq!(
        by_offset, by_cursor,
        "the two walks disagree on which facts are expired; a rebuild verified by \
         one and performed by the other would carry, or drop, exactly this difference"
    );

    // The other regime, on the SAME fixture shape. Without it the agreement above
    // could hold because both walks return the live set for a reason that has
    // nothing to do with expiry — and the comparison would never be seen doing
    // work. Here the two must agree on a set that is strictly LARGER.
    let future = tempfile::tempdir().expect("tempdir");
    {
        let store = NativeStore::open(future.path(), DIM).expect("open store");
        for id in &live {
            store
                .store_with_metadata(*id, &format!("live {id}"), &EMBEDDING, &meta(&[]))
                .expect("seed live");
        }
    }
    for id in &expired {
        super::preservation::seed_raw(
            future.path(),
            *id,
            &format!("not yet expired {id}"),
            Some(4_000_000_000),
        );
    }
    let db = velesdb_core::Database::open(future.path()).expect("open source");
    let future_cursor =
        ids(enumerate_by_cursor(&db, "_semantic_memory", PAGE).expect("cursor walk"));
    let future_offset =
        ids(enumerate_collection(&db, "_semantic_memory", PAGE).expect("offset walk"));

    let mut everything = SCRAMBLED.to_vec();
    everything.sort_unstable();
    assert_eq!(
        future_cursor, everything,
        "control: under a FUTURE expiry every seeded fact must come back, so the \
         exclusion above was about the expiry and not about the raw write path"
    );
    assert_eq!(
        future_offset, future_cursor,
        "the two walks must also agree when nothing is expired"
    );
    println!(
        "  past expiry:   {} live / {} expired over pages of {PAGE} -> both walks {by_cursor:?}",
        live.len(),
        expired.len()
    );
    println!(
        "  future expiry: all {} seeded -> both walks {future_cursor:?}",
        everything.len()
    );
}

/// Files allowed to name the `OFFSET` walk, because they DEFINE it or exercise
/// it as the independent verification path.
const OFFSET_WALK_OWNERS: &[&str] = &[
    "migration.rs",
    "migration/enumeration.rs",
    "migration/tests/enumeration.rs",
    "migration/tests/mod.rs",
    "migration/tests/performance.rs",
];

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

/// Every Rust source that belongs to the migration facade or one of its
/// descendants.
fn migration_sources(src: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut sources = vec![src.join("migration.rs")];
    collect_migration_sources(&src.join("migration"), &mut sources);
    sources.sort();
    sources
}

fn collect_migration_sources(dir: &std::path::Path, sources: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("read migration module") {
        let entry = entry.expect("migration entry");
        let path = entry.path();
        if entry.metadata().expect("migration metadata").is_dir() {
            collect_migration_sources(&path, sources);
        } else if path.extension().and_then(std::ffi::OsStr::to_str) == Some("rs") {
            sources.push(path);
        }
    }
}

/// No migration module beyond the verification path may page by `OFFSET`.
///
/// The walk is correct only below 100 000 facts and goes silently empty above
/// it, so a rebuild that used it would drop the tail of any large store and
/// report success. `scroll_page` is the supported route.
///
/// The owner list is explicit and the traversal is recursive: splitting this
/// module cannot make a future migration source invisible to the guard.
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
    for path in migration_sources(&src) {
        let relative = path
            .strip_prefix(&src)
            .expect("migration source under src/")
            .to_string_lossy()
            .replace('\\', "/");
        if OFFSET_WALK_OWNERS.contains(&relative.as_str()) {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("read migration source");
        assert!(
            !uses_the_offset_walk(&text),
            "{relative} reaches for the bounded OFFSET walk; a rebuild must page by \
             cursor (`scroll_page`), which carries no 100 000-fact ceiling"
        );
        scanned.push(relative);
    }
    println!("  guard armed; migration modules scanned beyond the walk's owners: {scanned:?}");
}
