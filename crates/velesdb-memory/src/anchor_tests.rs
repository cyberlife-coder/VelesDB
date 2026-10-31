use super::*;

fn digest(text: &str) -> ContentDigest {
    ContentDigest::of(text.as_bytes())
}

fn anchor(path: &str, text: &str) -> FileAnchor {
    FileAnchor {
        path: RepoPath::parse(path).unwrap(),
        digest: digest(text),
    }
}

/// The checkout root every test anchor set is recorded under.
const ROOT: &str = "/work/repo";

/// An anchor set recorded under [`ROOT`].
fn set(files: Vec<FileAnchor>) -> AnchorSet {
    AnchorSet {
        root: ROOT.to_owned(),
        files,
    }
}

/// A probe of [`ROOT`] answering from a fixed table, `Unverifiable` for
/// anything else.
struct TableProbe(Vec<(&'static str, AnchorObservation)>);

impl AnchorProbe for TableProbe {
    fn root(&self) -> &str {
        ROOT
    }

    fn observe(&mut self, anchor: &FileAnchor) -> AnchorObservation {
        self.0
            .iter()
            .find(|(path, _)| *path == anchor.path.as_str())
            .map_or(AnchorObservation::Unverifiable, |(_, seen)| *seen)
    }
}

#[test]
fn repo_path_accepts_plain_relative_paths() {
    for ok in [
        "src/lib.rs",
        "Cargo.toml",
        "a/b/c.d.e",
        "crates/x-y_z/src/main.rs",
    ] {
        assert_eq!(RepoPath::parse(ok).unwrap().as_str(), ok);
    }
}

#[test]
fn repo_path_refuses_anything_that_could_leave_the_root() {
    let long = "a/".repeat(300);
    for bad in [
        "",
        "/etc/passwd",
        "../secret",
        "src/../../x",
        "./src/lib.rs",
        "src//lib.rs",
        "src/lib.rs/",
        "src\\lib.rs",
        "src/\nlib.rs",
        "src/\0lib.rs",
        long.as_str(),
    ] {
        assert!(
            matches!(RepoPath::parse(bad), Err(AnchorError::InvalidPath { .. })),
            "{bad:?} must be refused"
        );
    }
}

#[test]
fn repo_path_deserialization_applies_the_same_rules() {
    let refused: Result<RepoPath, _> = serde_json::from_str("\"../x\"");
    assert!(refused.is_err(), "a stored path cannot bypass validation");
}

#[test]
fn content_digest_is_sha256_hex_and_round_trips() {
    let empty = ContentDigest::of(b"");
    assert_eq!(
        empty.as_str(),
        "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    let parsed = ContentDigest::try_from(empty.as_str().to_owned()).unwrap();
    assert_eq!(parsed, empty);
    for bad in [
        "e3b0",
        "sha256:XYZ",
        "sha1:abcd",
        "sha256:E3B0C44298FC1C149AFBF4C8996FB92427AE41E4649B934CA495991B7852B855",
    ] {
        assert!(
            ContentDigest::try_from(bad.to_owned()).is_err(),
            "{bad} must be refused"
        );
    }
}

#[test]
fn no_anchor_means_unanchored() {
    let mut probe = TableProbe(vec![]);
    assert_eq!(
        assess_freshness(&set(vec![]), &mut probe),
        Freshness::Unanchored
    );
}

#[test]
fn unchanged_files_mean_fresh() {
    let mut probe = TableProbe(vec![
        ("src/a.rs", AnchorObservation::Unchanged),
        ("src/b.rs", AnchorObservation::Unchanged),
    ]);
    let anchors = set(vec![anchor("src/a.rs", "a"), anchor("src/b.rs", "b")]);
    assert_eq!(assess_freshness(&anchors, &mut probe), Freshness::Fresh);
}

#[test]
fn a_changed_or_missing_file_makes_the_lesson_stale_and_names_it() {
    let mut probe = TableProbe(vec![
        ("src/a.rs", AnchorObservation::Unchanged),
        ("src/b.rs", AnchorObservation::Changed),
        ("src/c.rs", AnchorObservation::Missing),
        ("src/d.rs", AnchorObservation::Unverifiable),
    ]);
    let anchors = set(vec![
        anchor("src/a.rs", "a"),
        anchor("src/b.rs", "b"),
        anchor("src/c.rs", "c"),
        anchor("src/d.rs", "d"),
    ]);
    let verdict = assess_freshness(&anchors, &mut probe);
    assert_eq!(
        verdict,
        Freshness::Stale {
            changed: vec![
                RepoPath::parse("src/b.rs").unwrap(),
                RepoPath::parse("src/c.rs").unwrap(),
            ]
        }
    );
    assert!((verdict.rank_factor() - STALE_RANK_FACTOR).abs() < f32::EPSILON);
}

#[test]
fn an_unverifiable_file_is_never_evidence_of_staleness() {
    let mut probe = TableProbe(vec![("src/a.rs", AnchorObservation::Unchanged)]);
    let anchors = set(vec![
        anchor("src/a.rs", "a"),
        anchor("src/elsewhere.rs", "x"),
    ]);
    let verdict = assess_freshness(&anchors, &mut probe);
    assert_eq!(verdict, Freshness::Unverified);
    assert!((verdict.rank_factor() - 1.0).abs() < f32::EPSILON);
}

/// Given anchors recorded under one checkout, when a probe of another
/// checkout judges them — even one where every anchored path changed — then
/// the lesson is unverified, never stale, and nothing is looked at.
#[test]
fn anchors_from_another_checkout_are_unverified_never_stale() {
    const OTHER_ROOT: &str = "/work/other-clone";
    struct Elsewhere(usize);
    impl AnchorProbe for Elsewhere {
        fn root(&self) -> &str {
            OTHER_ROOT
        }
        fn observe(&mut self, _anchor: &FileAnchor) -> AnchorObservation {
            self.0 += 1;
            AnchorObservation::Changed
        }
    }
    let mut probe = Elsewhere(0);
    let verdict = assess_freshness(&set(vec![anchor("src/lib.rs", "a")]), &mut probe);
    assert_eq!(verdict, Freshness::Unverified);
    assert!((verdict.rank_factor() - 1.0).abs() < f32::EPSILON);
    assert_eq!(probe.0, 0, "no file of another checkout is read");
}

#[test]
fn stored_anchors_round_trip_through_the_reserved_payload_entry() {
    let anchors = set(vec![anchor("src/a.rs", "a")]);
    let payload = anchors_entry(&anchors);
    assert!(
        crate::storage::is_reserved_key(ANCHORS_KEY),
        "anchors stay out of caller metadata"
    );
    assert_eq!(
        payload.get(ANCHORS_KEY).and_then(|entry| entry.get("root")),
        Some(&serde_json::json!(ROOT)),
        "the checkout root is stored with the files"
    );
    assert_eq!(
        stored_anchors(Some(&payload)),
        StoredAnchors::Anchored(anchors)
    );
    assert_eq!(stored_anchors(None), StoredAnchors::Unanchored);
}

/// An entry written without its root — the shape anchors had before the root
/// was recorded — cannot say which checkout it is about: unverified.
#[test]
fn an_anchor_entry_without_its_root_reads_as_unverified() {
    let mut payload = Metadata::new();
    payload.insert(
        ANCHORS_KEY.to_owned(),
        serde_json::to_value(vec![anchor("src/a.rs", "a")]).unwrap(),
    );
    let mut probe = TableProbe(vec![("src/a.rs", AnchorObservation::Changed)]);
    assert_eq!(stored_anchors(Some(&payload)), StoredAnchors::Unreadable);
    assert_eq!(
        payload_freshness(Some(&payload), &mut probe),
        Freshness::Unverified
    );
}

#[test]
fn a_corrupt_anchor_entry_reads_as_unverified_never_as_fresh() {
    let mut payload = Metadata::new();
    payload.insert(
        ANCHORS_KEY.to_owned(),
        serde_json::json!({ "root": ROOT, "files": [{ "path": "../escape", "digest": "sha256:00" }] }),
    );
    let mut probe = TableProbe(vec![]);
    assert_eq!(stored_anchors(Some(&payload)), StoredAnchors::Unreadable);
    assert_eq!(
        payload_freshness(Some(&payload), &mut probe),
        Freshness::Unverified
    );
}

#[test]
fn anchor_count_is_capped() {
    assert!(check_anchor_count(MAX_ANCHORS_PER_FACT).is_ok());
    assert_eq!(
        check_anchor_count(MAX_ANCHORS_PER_FACT + 1),
        Err(AnchorError::TooMany {
            count: MAX_ANCHORS_PER_FACT + 1,
            max: MAX_ANCHORS_PER_FACT
        })
    );
}
