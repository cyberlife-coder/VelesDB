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
        &super::super::TargetContract::automatic(diagnosis::TARGET_MODEL, diagnosis::TARGET_DIM),
        None,
    )
    .expect("diagnosis must not contend on the live source lock");

    assert_eq!(
        report.source_path,
        std::fs::canonicalize(source.path()).expect("canonical source")
    );
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
        &super::super::TargetContract::automatic(diagnosis::TARGET_MODEL, diagnosis::TARGET_DIM),
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
        &super::super::TargetContract::automatic(diagnosis::TARGET_MODEL, diagnosis::TARGET_DIM),
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

/// A hard-link based backup (`cp -al`, `rsync --link-dest`) routinely
/// produces `nlink > 1` on an ordinary file. The diagnostic copy only
/// reads the source, so it must accept that shape rather than refuse it as
/// if it were a link planted to redirect a write (#2409) — a symlink swap
/// or any other non-regular type is still refused, just not a hard link
/// (see `copy_regular_file_itself_refuses_a_symlink_past_copy_entry_own_check`
/// and `copy_regular_file_itself_refuses_a_special_file_past_copy_entry_own_check`
/// below, both of which isolate `copy_regular_file`'s own refusal rather
/// than `copy_entry`'s earlier walk-time check).
#[cfg(unix)]
#[test]
fn a_hard_linked_source_file_is_copied_not_refused() {
    let source = tempfile::tempdir().expect("source");
    let original = source.path().join("payload.bin");
    std::fs::write(&original, b"payload").expect("seed");
    let alias = source.path().join("payload-alias.bin");
    std::fs::hard_link(&original, &alias).expect("plant in-tree hard link");
    let staging = tempfile::tempdir().expect("staging");

    let copy =
        super::super::diagnostic_copy::DiagnosticCopy::capture(source.path(), staging.path())
            .expect("a hard-linked source file must be copied, not refused");
    copy.finish(Ok(())).expect("cleanup");
}

/// A genuine I/O failure (here, permission denied) opening the source must
/// keep its own detail, not the internal "migration capture error:" prefix
/// `MemoryError::MigrationCapture`'s `Display` carries for its usual caller,
/// the online-migration observer — this copy is unrelated to that (#2409
/// round 7).
#[cfg(unix)]
#[test]
fn a_permission_denied_source_keeps_its_io_detail_not_the_migration_capture_prefix() {
    use std::os::unix::fs::PermissionsExt;

    let source = tempfile::tempdir().expect("source");
    let original = source.path().join("payload.bin");
    std::fs::write(&original, b"payload").expect("seed");
    std::fs::set_permissions(&original, std::fs::Permissions::from_mode(0o000)).expect("chmod 000");

    if std::fs::File::open(&original).is_ok() {
        eprintln!("test: skipped, permission bits are not enforced here");
        std::fs::set_permissions(&original, std::fs::Permissions::from_mode(0o644))
            .expect("restore");
        return;
    }

    let destination = source.path().join("copy.bin");
    let error = super::super::diagnostic_copy::copy_regular_file(&original, &destination, 7)
        .expect_err("a permission-denied source must surface as an error");
    let message = error.to_string();
    assert!(
        message.contains("Permission denied") || message.contains("permission denied"),
        "an I/O failure must keep its real detail: {message}"
    );
    assert!(
        !message.contains("migration capture error"),
        "an unrelated I/O failure must not carry the online-migration observer's own \
         wording: {message}"
    );

    std::fs::set_permissions(&original, std::fs::Permissions::from_mode(0o644))
        .expect("restore permissions");
}

/// `root_and_nested_symlinks_are_refused_without_following_them` above
/// never reaches `copy_regular_file`'s own protection: `copy_entry`'s
/// earlier `symlink_metadata` check catches a symlink present at walk time
/// first, every time, so a test that only goes through `DiagnosticCopy::capture`
/// cannot tell whether the refusal comes from that check or from
/// `copy_regular_file`'s `open_regular_file_allow_hard_links` call — the one
/// that actually matters for a symlink swapped in AFTER `copy_entry` looked
/// (#2409 round-3 finding). Calling `copy_regular_file` directly, on a
/// source that is a symlink from the start, isolates its own refusal
/// (`copy_entry` is never involved at all).
#[cfg(unix)]
#[test]
fn copy_regular_file_itself_refuses_a_symlink_past_copy_entry_own_check() {
    use std::os::unix::fs::symlink;

    let outside = tempfile::tempdir().expect("outside");
    let secret = outside.path().join("secret");
    let content = b"do-not-read-me";
    std::fs::write(&secret, content).expect("plant secret");
    let staging = tempfile::tempdir().expect("staging");
    let source = staging.path().join("source-link");
    symlink(&secret, &source).expect("plant symlink");
    let destination = staging.path().join("destination");

    // The REAL length of the symlink's target, not a placeholder: a plain
    // `open` that followed the symlink would read exactly this many bytes
    // and satisfy the length check downstream, making the function return
    // `Ok(())` outright rather than an unrelated length-mismatch error — the
    // only way this test then correctly fails on that mutant is through
    // `expect_err` below, not through a coincidental byte-count refusal.
    let error = super::super::diagnostic_copy::copy_regular_file(
        &source,
        &destination,
        content.len() as u64,
    )
    .expect_err(
        "copy_regular_file must refuse a symlinked source on its own, not rely on copy_entry",
    );
    assert!(error.to_string().contains("regular file"), "{error}");
    assert!(
        !destination.exists(),
        "a refused copy must not create the destination"
    );
    assert_eq!(
        std::fs::read(&secret).expect("secret must be untouched"),
        content
    );
}

/// Same gap as the symlink isolation test just above, for a non-symlink
/// special type: `a_special_file_is_refused_and_unrelated_scratch_is_never_swept`
/// below only proves `copy_entry`'s own `symlink_metadata` pre-check refuses
/// a socket present at walk time — it goes through `DiagnosticCopy::capture`,
/// so `copy_regular_file` is never reached at all for that case, and cannot
/// tell whether ITS OWN `open_regular_file_allow_hard_links` call would also
/// refuse one. Proven by mutation: narrowing `copy_regular_file` to refuse
/// only a symlink (not any other non-regular type) leaves this test failing
/// while `a_special_file_is_refused_and_unrelated_scratch_is_never_swept`
/// keeps passing (#2409 round-16 finding).
#[cfg(unix)]
#[test]
fn copy_regular_file_itself_refuses_a_special_file_past_copy_entry_own_check() {
    use std::os::unix::fs::PermissionsExt;

    let staging = tempfile::tempdir().expect("staging");
    let source = staging.path().join("source-fifo");
    let destination = staging.path().join("destination");
    let status = std::process::Command::new("mkfifo")
        .arg(&source)
        .status()
        .expect("run mkfifo");
    assert!(status.success(), "mkfifo must succeed");
    std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o644)).expect("chmod fifo");

    // Bounded on a background thread, not called inline: the real
    // `copy_regular_file` never blocks on a FIFO (its guard opens
    // `O_NONBLOCK`, proven by `atomic_file_tests`'s own FIFO test), but a
    // mutant that bypasses that guard and opens the FIFO for read directly
    // would block forever waiting for a writer — on the main thread, that
    // hangs the whole test binary instead of failing it (#2409 round 24).
    let (tx, rx) = std::sync::mpsc::channel();
    let destination_copy = destination.clone();
    std::thread::spawn(move || {
        let result =
            super::super::diagnostic_copy::copy_regular_file(&source, &destination_copy, 0);
        let _ = tx.send(result.map_err(|err| err.to_string()));
    });

    let error = rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("copy_regular_file must return instead of blocking on the FIFO")
        .expect_err(
            "copy_regular_file must refuse a FIFO source on its own, not rely on copy_entry",
        );
    assert!(error.contains("regular file"), "{error}");
    assert!(
        !destination.exists(),
        "a refused copy must not create the destination"
    );
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
