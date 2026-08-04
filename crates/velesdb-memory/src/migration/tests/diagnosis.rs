use super::*;

// ---------------------------------------------------------------------------
// GATE 3 — the inventory
//
// A rebuild has to be told what it is about to move before it moves it, and by
// something that CANNOT move it. Every test below runs the diagnosis and then
// checks the store is exactly as it was — because a read-only claim that is
// never checked is a claim, not a property.
// ---------------------------------------------------------------------------

pub(super) const TARGET_MODEL: &str = "bge-m3";
pub(super) const TARGET_DIM: usize = 1024;

/// Every file under `dir`, by relative path, with its length and its bytes.
///
/// Timestamps are deliberately absent: `atime` moves when a file is READ, so a
/// comparison including it would report every diagnosis as a modification and
/// prove nothing. Content and length are what a rebuild would actually lose.
pub(super) fn tree(dir: &std::path::Path) -> std::collections::BTreeMap<String, (u64, Vec<u8>)> {
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
pub(super) fn drift(
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

fn diagnose_controlled_copy(
    original: &std::path::Path,
    before: &std::collections::BTreeMap<String, (u64, Vec<u8>)>,
) -> DiagnosisReport {
    let workspace = tempfile::tempdir().expect("tempdir");
    let copy = workspace.path().join("copy");
    copy_tree(original, &copy);
    assert_eq!(
        &tree(&copy),
        before,
        "the copy must start out identical, or the comparison below compares nothing"
    );

    let report = diagnose(&copy, TARGET_MODEL, TARGET_DIM, None).expect("diagnose");
    assert!(report.facts > 0, "the diagnosis must actually have read");
    let untouched = drift(before, &tree(original));
    assert!(
        untouched.is_empty(),
        "diagnosing a copy must leave the ORIGINAL byte-for-byte as it was; drifted: {untouched:?}"
    );
    report
}

fn assert_direct_diagnosis_writes_only_derived_files(
    original: &std::path::Path,
    before: &std::collections::BTreeMap<String, (u64, Vec<u8>)>,
    expected_facts: u64,
) {
    let direct = diagnose(original, TARGET_MODEL, TARGET_DIM, None).expect("diagnose");
    assert_eq!(
        direct.facts, expected_facts,
        "the copy and the original must describe the same store"
    );
    let drifted = drift(before, &tree(original));
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
    let normalised = tree(original);
    let _ = diagnose(original, TARGET_MODEL, TARGET_DIM, None).expect("diagnose");
    assert!(
        drift(&normalised, &tree(original)).is_empty(),
        "a second diagnosis of an already-normalised store must drift nothing"
    );
}

#[test]
fn a_diagnostic_does_not_change_the_directory_tree() {
    // The diagnosis runs against a controlled copy because `Database::open`
    // rewrites derived artifacts before a single fact is read.
    let (original, _ttl_meta) = seeded();
    let before = tree(original.path());
    assert!(
        !before.is_empty(),
        "positive control: an empty 'before' would make any 'after' equal to it"
    );
    let report = diagnose_controlled_copy(original.path(), &before);
    assert_direct_diagnosis_writes_only_derived_files(original.path(), &before, report.facts);

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
