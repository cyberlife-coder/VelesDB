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
    assert!(
        matches!(
            known.capabilities.get("source_provenance"),
            Some(Capability::Proven { .. })
        ),
        "a provenance record whose width agrees with the source collections \
         must be usable evidence, got {:?}",
        known.capabilities.get("source_provenance")
    );
}

#[test]
fn known_provenance_that_disagrees_with_collection_width_is_a_blocker() {
    let (dir, _ttl_meta) = seeded();
    crate::embedding_provenance::write(
        dir.path(),
        &crate::embedding_provenance::EmbeddingProvenance::new("all-minilm", DIM + 1),
    )
    .expect("write incompatible provenance");

    let report = diagnose(dir.path(), TARGET_MODEL, TARGET_DIM, None).expect("diagnose");
    let Some(Capability::Missing { blocker }) = report.capabilities.get("source_provenance") else {
        panic!(
            "a record that claims a different width from the vectors must block, got {:?}",
            report.capabilities.get("source_provenance")
        );
    };
    assert!(
        blocker.contains(&(DIM + 1).to_string()) && blocker.contains(&DIM.to_string()),
        "the refusal must name both incompatible widths: {blocker}"
    );
    assert!(
        report
            .blockers
            .iter()
            .any(|entry| entry.starts_with("source_provenance:")),
        "a missing capability must also appear in the report's blocker list"
    );
    assert!(!report.is_clear());
}

#[test]
fn one_model_identity_cannot_claim_two_dimensions_across_source_and_target() {
    let (dir, _ttl_meta) = seeded();
    crate::embedding_provenance::write(
        dir.path(),
        &crate::embedding_provenance::EmbeddingProvenance::new("same-model", DIM),
    )
    .expect("write provenance");

    let incompatible =
        diagnose(dir.path(), "same-model", DIM + 1, None).expect("diagnose mismatch");
    let Some(Capability::Missing { blocker }) = incompatible.capabilities.get("source_provenance")
    else {
        panic!(
            "one model name at two widths must be rejected, got {:?}",
            incompatible.capabilities.get("source_provenance")
        );
    };
    assert!(
        blocker.contains("same-model")
            && blocker.contains(&DIM.to_string())
            && blocker.contains(&(DIM + 1).to_string()),
        "the mismatch must name the model and both dimensions: {blocker}"
    );

    let compatible = diagnose(dir.path(), "same-model", DIM, None).expect("diagnose match");
    assert!(
        matches!(
            compatible.capabilities.get("source_provenance"),
            Some(Capability::Proven { .. })
        ),
        "positive control: the same model at the same width must reconcile"
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

#[test]
fn a_diagnostic_does_not_change_the_directory_tree() {
    let (original, _ttl_meta) = seeded();
    let before = tree(original.path());
    assert!(
        !before.is_empty(),
        "positive control: an empty 'before' would make any 'after' equal to it"
    );
    let report = diagnose(original.path(), TARGET_MODEL, TARGET_DIM, None).expect("diagnose");
    assert!(report.facts > 0, "the diagnosis must actually have read");
    assert!(
        drift(&before, &tree(original.path())).is_empty(),
        "the public diagnosis path must leave the source byte-for-byte unchanged"
    );

    assert!(
        matches!(
            report.capabilities.get("source_access_is_read_only"),
            Some(Capability::Proven { .. })
        ),
        "the report must carry evidence that Database::open ran only on a verified copy"
    );
}

#[test]
fn a_relative_source_is_recorded_as_its_canonical_absolute_path() {
    let working_directory = std::env::current_dir().expect("current directory");
    let source = tempfile::tempdir_in(&working_directory).expect("source under current directory");
    {
        let store = NativeStore::open(source.path(), DIM).expect("open source");
        store
            .store(1, "canonical path proof", &EMBEDDING)
            .expect("seed source");
    }
    let relative = source
        .path()
        .strip_prefix(&working_directory)
        .expect("source is below current directory");

    let report = diagnose(relative, TARGET_MODEL, TARGET_DIM, None).expect("diagnose relative");
    let canonical = std::fs::canonicalize(source.path()).expect("canonical source");
    assert!(report.source_path.is_absolute());
    assert_eq!(report.source_path, canonical);
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
    #[cfg(unix)]
    {
        assert_eq!(
            report.same_filesystem,
            Some(true),
            "both temp directories share a filesystem in this fixture"
        );
        assert!(
            matches!(
                report.capabilities.get("switch_same_filesystem"),
                Some(Capability::Proven { .. })
            ),
            "a measured same-filesystem destination is the positive control for the rename gate"
        );
    }
    #[cfg(not(unix))]
    assert!(
        matches!(
            report.capabilities.get("switch_same_filesystem"),
            Some(Capability::Missing { .. })
        ),
        "unknown topology must block on platforms where it cannot be established"
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

#[test]
fn unknown_or_different_filesystems_are_explicit_switch_blockers() {
    for topology in [None, Some(false)] {
        let capability = super::super::diagnosis::switch_filesystem_capability(topology);
        let Capability::Missing { blocker } = capability else {
            panic!("topology {topology:?} must not authorize a rename switch");
        };
        assert!(
            blocker.contains("rename"),
            "the blocker must name the switch primitive whose contract failed: {blocker}"
        );
    }

    let (dir, _ttl) = seeded();
    let report = diagnose(dir.path(), TARGET_MODEL, TARGET_DIM, None).expect("diagnose");
    assert_eq!(report.same_filesystem, None);
    assert!(
        report
            .blockers
            .iter()
            .any(|entry| entry.starts_with("switch_same_filesystem:")),
        "the missing destination must reach the serialized blocker list, got {:?}",
        report.blockers
    );
}

#[test]
fn edge_counts_do_not_masquerade_as_a_lossless_edge_export() {
    let dir = tempfile::tempdir().expect("tempdir");
    {
        let store = NativeStore::open(dir.path(), DIM).expect("open store");
        for id in 1..=2 {
            store
                .store_with_metadata(id, &format!("fact {id}"), &EMBEDDING, &meta(&[]))
                .expect("seed fact");
        }
        store.relate(1, 2, "supports").expect("seed edge");
    }

    let report = diagnose(dir.path(), TARGET_MODEL, TARGET_DIM, None).expect("diagnose");
    assert_eq!(
        report.edges, 1,
        "positive control: the diagnosis really observed the source relation"
    );
    assert!(
        matches!(
            report.capabilities.get("edge_counts"),
            Some(Capability::Proven { .. })
        ),
        "counting should remain separately evidenced"
    );
    let Some(Capability::Missing { blocker }) = report.capabilities.get("edge_export") else {
        panic!(
            "a count-only report must not claim complete edge export, got {:?}",
            report.capabilities.get("edge_export")
        );
    };
    assert!(
        blocker.contains("edge id") && blocker.contains("properties"),
        "the blocker must name the fields a lossless export needs: {blocker}"
    );
    assert!(
        report
            .blockers
            .iter()
            .any(|entry| entry.starts_with("edge_export:")),
        "the missing export must be a blocker, not a footnote"
    );
    assert!(!report.is_clear());
}

#[test]
fn a_diagnosis_report_round_trips_through_its_versioned_parser() {
    let (dir, _ttl) = seeded();
    let report = diagnose(dir.path(), TARGET_MODEL, TARGET_DIM, None).expect("diagnose");
    let json = serde_json::to_string_pretty(&report).expect("serialize report");

    let parsed = DiagnosisReport::parse(&json).expect("parse current report");
    assert_eq!(
        parsed, report,
        "every reported field must survive round-trip"
    );

    let directly_deserialized: DiagnosisReport =
        serde_json::from_str(&json).expect("DiagnosisReport implements Deserialize");
    assert_eq!(
        directly_deserialized, report,
        "the report type itself must be deserializable in addition to the guarded parser"
    );
}

#[test]
fn a_report_from_a_future_format_is_refused_before_its_shape_is_interpreted() {
    let (dir, _ttl) = seeded();
    let report = diagnose(dir.path(), TARGET_MODEL, TARGET_DIM, None).expect("diagnose");
    let mut future = serde_json::to_value(report).expect("serialize report");
    future["format_version"] = serde_json::json!(DIAGNOSIS_FORMAT_VERSION + 1);
    future["a_field_from_the_future"] = serde_json::json!({ "shape": "unknown" });
    let json = serde_json::to_string(&future).expect("serialize future report");

    let refusal = DiagnosisReport::parse(&json).expect_err("future format must be refused");
    assert!(
        refusal.contains(&(DIAGNOSIS_FORMAT_VERSION + 1).to_string())
            && refusal.contains("newer")
            && refusal.contains("fresh diagnosis"),
        "the refusal must identify version skew and the recovery action: {refusal}"
    );
    assert!(
        !refusal.contains("a_field_from_the_future"),
        "the parser must refuse on version before interpreting unknown future shape: {refusal}"
    );

    let direct = serde_json::from_str::<DiagnosisReport>(&json)
        .expect_err("Deserialize itself must refuse a future report");
    assert!(
        direct.to_string().contains("newer")
            && direct
                .to_string()
                .contains(&(DIAGNOSIS_FORMAT_VERSION + 1).to_string()),
        "direct Deserialize must enforce the same version gate: {direct}"
    );
}

fn direct_deserialization_refusal(report: &DiagnosisReport) -> String {
    let json = serde_json::to_string(report).expect("serialize forged report");
    serde_json::from_str::<DiagnosisReport>(&json)
        .expect_err("forged report must be refused")
        .to_string()
}

#[test]
fn empty_capabilities_cannot_forge_a_vacuously_clear_report() {
    let (dir, _ttl) = seeded();
    let mut forged = diagnose(dir.path(), TARGET_MODEL, TARGET_DIM, None).expect("diagnose");
    forged.capabilities.clear();
    forged.blockers.clear();

    assert!(
        !forged.is_clear(),
        "an object assembled in memory with two empty collections of gates must not be clear"
    );
    let refusal = direct_deserialization_refusal(&forged);
    assert!(
        refusal.contains("capabilities") && refusal.contains("expected exactly"),
        "the refusal must identify the incomplete capability vocabulary: {refusal}"
    );
}

#[test]
fn a_removed_blocker_is_refused_even_when_the_capabilities_remain_missing() {
    let (dir, _ttl) = seeded();
    let mut forged = diagnose(dir.path(), TARGET_MODEL, TARGET_DIM, None).expect("diagnose");
    assert!(
        forged.blockers.len() > 1,
        "positive control: the generated report has blockers to remove"
    );
    forged.blockers.remove(0);

    assert!(!forged.is_clear());
    let refusal = direct_deserialization_refusal(&forged);
    assert!(
        refusal.contains("blockers do not match"),
        "blockers must be re-derived rather than trusted: {refusal}"
    );
}

#[test]
fn topology_and_provenance_capabilities_cannot_be_forged() {
    let (dir, _ttl) = seeded();
    let report = diagnose(dir.path(), TARGET_MODEL, TARGET_DIM, None).expect("diagnose");
    assert_eq!(
        report.same_filesystem, None,
        "this report has no destination, so its canonical switch verdict is Missing"
    );
    assert!(matches!(
        report.source_provenance,
        SourceProvenance::Unknown { .. }
    ));

    for capability_name in ["switch_same_filesystem", "source_provenance"] {
        let mut forged = report.clone();
        forged.capabilities.insert(
            capability_name.to_owned(),
            Capability::Proven {
                evidence: "operator asserted it".to_owned(),
            },
        );
        let refusal = direct_deserialization_refusal(&forged);
        assert!(
            refusal.contains(capability_name) && refusal.contains("inconsistent"),
            "{capability_name} must be recalculated from report fields: {refusal}"
        );
    }
}

#[test]
fn permanent_v4_gates_cannot_be_promoted_by_editing_json() {
    let (dir, _ttl) = seeded();
    let report = diagnose(dir.path(), TARGET_MODEL, TARGET_DIM, None).expect("diagnose");

    for capability_name in ["edge_export", "disk_headroom", "embedder_cost"] {
        let mut forged = report.clone();
        forged.capabilities.insert(
            capability_name.to_owned(),
            Capability::Proven {
                evidence: "claimed outside the diagnosis".to_owned(),
            },
        );
        let refusal = direct_deserialization_refusal(&forged);
        assert!(
            refusal.contains(capability_name) && refusal.contains("inconsistent"),
            "the canonical v4 Missing verdict for {capability_name} must be immutable: {refusal}"
        );
    }
}
