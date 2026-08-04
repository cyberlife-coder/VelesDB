use super::*;

#[test]
fn diagnosis_works_while_the_original_lock_is_held_and_leaves_source_unchanged() {
    let source = tempfile::tempdir().expect("source");
    let store = NativeStore::open(source.path(), DIM).expect("open live store");
    store
        .store_with_metadata(
            42,
            "fact held by the live daemon",
            &EMBEDDING,
            &meta(&[("project", Value::from("veles"))]),
        )
        .expect("seed live store");
    let before = diagnosis::tree(source.path());
    let staging = tempfile::tempdir().expect("staging");

    let report = super::super::diagnose(
        source.path(),
        staging.path(),
        diagnosis::TARGET_MODEL,
        diagnosis::TARGET_DIM,
        None,
    )
    .expect("diagnosis must not contend on the live source lock");

    assert_eq!(report.source_path, source.path());
    assert_eq!(report.facts, 1, "the verified copy must actually be read");
    assert!(
        matches!(
            report.capabilities.get("source_access_is_read_only"),
            Some(Capability::Proven { .. })
        ),
        "the report must carry the controlled-copy evidence"
    );
    assert!(
        diagnosis::drift(&before, &diagnosis::tree(source.path())).is_empty(),
        "diagnosing a live store must leave every source byte unchanged"
    );
    assert_eq!(
        std::fs::read_dir(staging.path())
            .expect("read staging")
            .count(),
        0,
        "the ephemeral copy must be removed before success is returned"
    );
    assert_eq!(store.count(), 1, "the live store must still respond");
}

#[test]
fn a_mutation_during_capture_is_refused_and_scratch_is_removed() {
    let source = tempfile::tempdir().expect("source");
    let file = source.path().join("payload.bin");
    std::fs::write(&file, b"AAAA").expect("seed");
    let staging = tempfile::tempdir().expect("staging");
    let probe = |_path: &std::path::Path| Ok(u64::MAX);
    let mut mutated = false;
    let mut mutate_once = |copied: &std::path::Path| {
        if !mutated && copied == file {
            std::fs::write(&file, b"BBBB").map_err(|err| {
                crate::MemoryError::Storage(velesdb_core::Error::Query(format!(
                    "test mutation failed: {err}"
                )))
            })?;
            mutated = true;
        }
        Ok(())
    };

    let error = super::super::diagnostic_copy::DiagnosticCopy::capture_with(
        source.path(),
        staging.path(),
        &probe,
        &mut mutate_once,
    )
    .err()
    .expect("a moving source must be refused");
    assert!(error.to_string().contains("source changed"), "{error}");
    assert!(
        mutated,
        "positive control: the hook must have changed the file"
    );
    assert_eq!(
        std::fs::read_dir(staging.path())
            .expect("read staging")
            .count(),
        0,
        "ordinary capture failure must clean its owned scratch"
    );

    let mut no_mutation = |_path: &std::path::Path| Ok(());
    let copy = super::super::diagnostic_copy::DiagnosticCopy::capture_with(
        source.path(),
        staging.path(),
        &probe,
        &mut no_mutation,
    )
    .expect("stable positive control");
    copy.finish(Ok(())).expect("cleanup stable copy");
}

#[test]
fn a_source_mutation_after_capture_refuses_the_inventory_report() {
    let source = tempfile::tempdir().expect("source");
    {
        let store = NativeStore::open(source.path(), DIM).expect("open store");
        store
            .store_with_metadata(1, "original", &EMBEDDING, &Metadata::new())
            .expect("seed");
    }
    let staging = tempfile::tempdir().expect("staging");
    let copy =
        super::super::diagnostic_copy::DiagnosticCopy::capture(source.path(), staging.path())
            .expect("capture stable source");

    std::fs::write(source.path().join("concurrent-write"), b"changed")
        .expect("simulate concurrent daemon write");
    let result = super::super::diagnosis::diagnose_copy(
        source.path(),
        diagnosis::TARGET_MODEL,
        diagnosis::TARGET_DIM,
        None,
        &copy,
    );
    let error = copy
        .finish(result)
        .expect_err("a report over a stale capture must be refused");

    assert!(
        error
            .to_string()
            .contains("source changed during diagnosis"),
        "{error}"
    );
    assert_eq!(
        std::fs::read_dir(staging.path())
            .expect("read staging")
            .count(),
        0,
        "post-capture refusal must still clean the owned scratch"
    );
}

#[test]
fn insufficient_space_is_refused_before_creating_scratch() {
    let source = tempfile::tempdir().expect("source");
    std::fs::write(source.path().join("payload.bin"), b"payload").expect("seed");
    let staging = tempfile::tempdir().expect("staging");
    let no_space = |_path: &std::path::Path| Ok(0);
    let mut no_hook = |_path: &std::path::Path| Ok(());

    let error = super::super::diagnostic_copy::DiagnosticCopy::capture_with(
        source.path(),
        staging.path(),
        &no_space,
        &mut no_hook,
    )
    .err()
    .expect("insufficient space must refuse");
    assert!(error.to_string().contains("insufficient"), "{error}");
    assert_eq!(
        std::fs::read_dir(staging.path())
            .expect("read staging")
            .count(),
        0,
        "space must be checked before the first scratch directory is created"
    );

    let enough = |_path: &std::path::Path| Ok(u64::MAX);
    let copy = super::super::diagnostic_copy::DiagnosticCopy::capture_with(
        source.path(),
        staging.path(),
        &enough,
        &mut no_hook,
    )
    .expect("ample-space positive control");
    copy.finish(Ok(())).expect("cleanup");
}

#[test]
fn scratch_inside_source_is_refused_without_mutating_the_source() {
    let source = tempfile::tempdir().expect("source");
    std::fs::write(source.path().join("payload.bin"), b"payload").expect("seed");
    let inside = source.path().join("staging");
    std::fs::create_dir(&inside).expect("inside staging");
    let before = diagnosis::tree(source.path());

    let error = super::super::diagnose(
        source.path(),
        &inside,
        diagnosis::TARGET_MODEL,
        diagnosis::TARGET_DIM,
        None,
    )
    .expect_err("scratch inside source must be refused");
    assert!(error.to_string().contains("inside source"), "{error}");
    assert!(
        diagnosis::drift(&before, &diagnosis::tree(source.path())).is_empty(),
        "refusal must not alter the source"
    );
}

#[cfg(unix)]
#[test]
fn root_and_nested_symlinks_are_refused_without_following_them() {
    use std::os::unix::fs::symlink;

    let source = tempfile::tempdir().expect("source");
    let outside = tempfile::tempdir().expect("outside");
    let secret = outside.path().join("secret");
    std::fs::write(&secret, b"outside").expect("outside file");
    symlink(&secret, source.path().join("nested-link")).expect("nested symlink");
    let staging = tempfile::tempdir().expect("staging");

    let error =
        super::super::diagnostic_copy::DiagnosticCopy::capture(source.path(), staging.path())
            .err()
            .expect("nested symlink must be refused");
    assert!(error.to_string().contains("symlink"), "{error}");
    assert_eq!(std::fs::read(&secret).expect("outside intact"), b"outside");

    std::fs::remove_file(source.path().join("nested-link")).expect("remove nested link");
    std::fs::write(source.path().join("regular"), b"inside").expect("regular file");
    let root_link_parent = tempfile::tempdir().expect("root link parent");
    let root_link = root_link_parent.path().join("source-link");
    symlink(source.path(), &root_link).expect("root symlink");
    let error = super::super::diagnostic_copy::DiagnosticCopy::capture(&root_link, staging.path())
        .err()
        .expect("root symlink must be refused");
    assert!(error.to_string().contains("symlink"), "{error}");

    let copy =
        super::super::diagnostic_copy::DiagnosticCopy::capture(source.path(), staging.path())
            .expect("regular-tree positive control");
    copy.finish(Ok(())).expect("cleanup");
}

#[cfg(unix)]
#[test]
fn a_special_file_is_refused_and_unrelated_scratch_is_never_swept() {
    use std::os::unix::net::UnixListener;

    let source = tempfile::tempdir().expect("source");
    let socket = source.path().join("live.socket");
    let listener = UnixListener::bind(&socket).expect("bind socket");
    let staging = tempfile::tempdir().expect("staging");
    let unrelated = staging.path().join(".velesdb-diagnosis-unrelated");
    std::fs::create_dir(&unrelated).expect("unrelated scratch-like directory");
    std::fs::write(unrelated.join("sentinel"), b"keep").expect("sentinel");

    let error =
        super::super::diagnostic_copy::DiagnosticCopy::capture(source.path(), staging.path())
            .err()
            .expect("special file must be refused");
    assert!(error.to_string().contains("special file"), "{error}");
    assert_eq!(
        std::fs::read(unrelated.join("sentinel")).expect("unrelated retained"),
        b"keep",
        "cleanup must never sweep a pre-existing scratch-like directory"
    );

    drop(listener);
    std::fs::remove_file(socket).expect("remove socket");
    std::fs::write(source.path().join("regular"), b"inside").expect("regular file");
    let copy =
        super::super::diagnostic_copy::DiagnosticCopy::capture(source.path(), staging.path())
            .expect("regular-tree positive control");
    copy.finish(Ok(())).expect("cleanup");
    assert!(
        unrelated.exists(),
        "owned cleanup must retain unrelated data"
    );
}
