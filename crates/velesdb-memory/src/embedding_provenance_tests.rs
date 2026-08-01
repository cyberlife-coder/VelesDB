//! What the store records about the embedder that filled it, and what it
//! refuses when the two stop matching (#1751, arbitration A1).

use super::*;

fn store_dir() -> tempfile::TempDir {
    tempfile::tempdir().expect("tempdir")
}

#[test]
fn a_record_round_trips_through_the_store_directory() {
    let dir = store_dir();
    let written = EmbeddingProvenance::new("bge-m3", 1024);
    write(dir.path(), &written).expect("write provenance");
    let read_back = read(dir.path()).expect("read provenance");
    assert_eq!(read_back.as_ref(), Some(&written));
}

#[test]
fn an_absent_record_is_not_an_error() {
    // The normal state of every store created before this existed. It has to
    // be distinguishable from a failure, because the two lead to opposite
    // actions: one degrades the check, the other stops the daemon.
    let dir = store_dir();
    assert_eq!(
        read(dir.path()).expect("an absent record is a normal outcome"),
        None
    );
}

#[test]
fn the_backend_is_not_part_of_the_record() {
    // The correction that shaped this whole design: a backend is a TRANSPORT.
    // The same model served by Ollama, by oMLX or by a hosted OpenAI-compatible
    // API produces the same vectors, so recording who served them would refuse
    // a perfectly valid open for the sole reason that the transport changed —
    // which is exactly the migration #1751 exists to allow.
    let dir = store_dir();
    write(dir.path(), &EmbeddingProvenance::new("bge-m3", 1024)).expect("write");
    let raw = std::fs::read_to_string(dir.path().join(PROVENANCE_FILE)).expect("read raw");
    assert!(
        !raw.contains("backend") && !raw.contains("ollama") && !raw.contains("openai"),
        "the record must carry the model and the dimension and nothing else, got: {raw}"
    );
    assert!(raw.contains("bge-m3") && raw.contains("1024"), "got: {raw}");
}

#[test]
fn the_same_model_at_the_same_dimension_opens() {
    let stored = EmbeddingProvenance::new("bge-m3", 1024);
    assert!(check(Some(&stored), "bge-m3", 1024).is_ok());
}

#[test]
fn a_different_model_is_refused_naming_both_configurations() {
    let stored = EmbeddingProvenance::new("bge-m3", 1024);
    let err = check(Some(&stored), "all-minilm", 384)
        .expect_err("a store filled with one model cannot be searched with another");
    for expected in ["bge-m3", "1024", "all-minilm", "384"] {
        assert!(
            err.contains(expected),
            "the refusal must name BOTH configurations — the operator has to know \
             which one to change. Missing {expected} in: {err}"
        );
    }
    assert!(
        err.contains("VELESDB_MEMORY_EMBEDDER_MODEL"),
        "and it must name the variable that changes it, got: {err}"
    );
}

#[test]
fn the_same_model_at_a_different_dimension_is_refused() {
    // Edge case, and a real one: a served model can change what its name means
    // (a quantisation swap, a provider re-pointing an alias). The name matching
    // is then a false reassurance, so the dimension is checked on its own.
    let stored = EmbeddingProvenance::new("bge-m3", 1024);
    let err = check(Some(&stored), "bge-m3", 768).expect_err("same name, different vectors");
    assert!(
        err.contains("1024") && err.contains("768"),
        "the refusal must name both dimensions, got: {err}"
    );
}

#[test]
fn an_unrecorded_model_discloses_that_only_the_dimension_was_compared() {
    // A store created before this record existed cannot be checked fully, and
    // saying nothing about that would be the worse failure: the operator would
    // read a successful open as "my model matches", which was never verified.
    let note = unrecorded_model_note("bge-m3");
    assert!(
        note.contains("bge-m3"),
        "the note must name what the daemon is configured for, got: {note}"
    );
    assert!(
        note.contains("dimension"),
        "and it must say the comparison was dimension-only, got: {note}"
    );
}

#[test]
fn nothing_recorded_means_nothing_to_refuse() {
    assert!(
        check(None, "any-model", 1).is_ok(),
        "an unrecorded model is not a mismatch — it is an unknown, and the \
         dimension check in the core still applies underneath"
    );
}

#[test]
fn a_corrupt_record_is_an_error_rather_than_a_silent_pass() {
    // Treating an unreadable record as "absent" would silently disable the
    // guard on exactly the store whose metadata is already damaged.
    let dir = store_dir();
    std::fs::write(dir.path().join(PROVENANCE_FILE), "{not json").expect("write junk");
    let err = read(dir.path()).expect_err("a damaged record must be reported");
    assert!(
        err.contains(PROVENANCE_FILE),
        "the error must name the file to delete, got: {err}"
    );
}

#[test]
fn a_record_from_a_newer_version_still_reads_its_known_fields() {
    // Forward compatibility, deliberately: an older binary opening a store
    // stamped by a newer one must still be able to run the check it does
    // understand, rather than refuse the store outright.
    let dir = store_dir();
    std::fs::write(
        dir.path().join(PROVENANCE_FILE),
        r#"{"model":"bge-m3","dimension":1024,"future_field":"whatever"}"#,
    )
    .expect("write");
    let read_back = read(dir.path()).expect("unknown fields must not break the read");
    assert_eq!(
        read_back,
        Some(EmbeddingProvenance::new("bge-m3", 1024)),
        "the fields this version knows must survive an unknown one"
    );
}
