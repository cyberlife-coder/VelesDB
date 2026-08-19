use super::{atomic_write, AtomicWriteBoundary, FaultGuard};

#[test]
fn test_atomic_write_round_trips_and_leaves_no_temp() {
    let dir = tempfile::TempDir::new().expect("test: temp dir");
    let path = dir.path().join("snap.bin");

    atomic_write(&path, b"hello").expect("test: write");
    assert_eq!(std::fs::read(&path).expect("test: read"), b"hello");

    // No stray temp files remain after a successful write.
    let leftovers: Vec<_> = std::fs::read_dir(dir.path())
        .expect("test: read dir")
        .filter_map(std::result::Result::ok)
        .filter(|e| e.file_name().to_string_lossy().contains(".tmp."))
        .collect();
    assert!(leftovers.is_empty(), "no .tmp files should remain");
}

#[test]
fn test_atomic_write_overwrites_existing() {
    let dir = tempfile::TempDir::new().expect("test: temp dir");
    let path = dir.path().join("snap.bin");
    atomic_write(&path, b"first").expect("test: first");
    atomic_write(&path, b"second").expect("test: overwrite");
    assert_eq!(std::fs::read(&path).expect("test: read"), b"second");
}

#[test]
fn temporary_sync_failure_preserves_previous_file() {
    let dir = tempfile::TempDir::new().expect("test: temp dir");
    let path = dir.path().join("snap.bin");
    atomic_write(&path, b"previous").expect("test: seed file");

    let _fault = FaultGuard::inject(AtomicWriteBoundary::TemporaryFileSync);
    assert!(atomic_write(&path, b"replacement").is_err());
    assert_eq!(std::fs::read(path).expect("test: read"), b"previous");
    let leftovers = std::fs::read_dir(dir.path())
        .expect("test: read dir")
        .filter_map(std::result::Result::ok)
        .any(|entry| entry.file_name().to_string_lossy().contains(".tmp."));
    assert!(!leftovers, "failed writes must remove their temp file");
}

#[cfg(unix)]
#[test]
fn parent_directory_sync_failure_is_propagated() {
    let dir = tempfile::TempDir::new().expect("test: temp dir");
    let path = dir.path().join("snap.bin");

    let _fault = FaultGuard::inject(AtomicWriteBoundary::ParentDirectorySync);
    let error = atomic_write(&path, b"replacement").expect_err("barrier must fail");
    assert!(error.to_string().contains("ParentDirectorySync"));
}
