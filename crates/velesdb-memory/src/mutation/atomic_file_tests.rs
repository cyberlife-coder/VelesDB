//! The shared atomic-replace primitives: each check names the store that
//! called it in its message and refuses a symlink or a path of the wrong
//! kind, `path_exists` does not follow links, and `promote` replaces a file
//! whole.

use std::fs;

use super::{path_exists, promote, validate_regular_file, validate_workspace};

/// The two stores that share these primitives each name themselves in every
/// message, so each check runs under both names.
const ENTITIES: [&str; 2] = ["controller", "online migration"];

#[test]
fn a_workspace_must_be_a_real_directory_and_says_so_under_each_stores_name() {
    let root = tempfile::tempdir().expect("root");
    let file = root.path().join("file");
    fs::write(&file, b"x").expect("file");
    for entity in ENTITIES {
        validate_workspace(root.path(), entity).expect("a real directory");

        let refused = validate_workspace(&file, entity).expect_err("a file is no workspace");
        assert!(
            refused
                .to_string()
                .contains(&format!("{entity} workspace must be a real directory")),
            "{refused}"
        );
        let missing = validate_workspace(&root.path().join("absent"), entity).expect_err("absent");
        assert!(
            missing
                .to_string()
                .contains(&format!("cannot inspect {entity} workspace: ")),
            "{missing}"
        );
    }
}

#[test]
fn a_file_must_be_a_regular_file_and_says_so_under_each_stores_name() {
    let root = tempfile::tempdir().expect("root");
    let file = root.path().join("file");
    fs::write(&file, b"x").expect("file");
    for entity in ENTITIES {
        validate_regular_file(&file, entity).expect("a regular file");

        let refused = validate_regular_file(root.path(), entity).expect_err("a directory");
        assert!(
            refused
                .to_string()
                .contains(&format!("{entity} path must be a regular file")),
            "{refused}"
        );
        let missing =
            validate_regular_file(&root.path().join("absent"), entity).expect_err("absent");
        assert!(
            missing
                .to_string()
                .contains(&format!("cannot inspect {entity} file: ")),
            "{missing}"
        );
    }
}

#[cfg(unix)]
#[test]
fn a_symlink_is_refused_as_a_workspace_and_as_a_file() {
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir().expect("root");
    let real_dir = root.path().join("real");
    fs::create_dir(&real_dir).expect("dir");
    let victim = root.path().join("victim");
    fs::write(&victim, b"untouched").expect("victim");
    let dir_link = root.path().join("dir-link");
    let file_link = root.path().join("file-link");
    symlink(&real_dir, &dir_link).expect("dir link");
    symlink(&victim, &file_link).expect("file link");

    for entity in ENTITIES {
        let workspace = validate_workspace(&dir_link, entity).expect_err("linked workspace");
        assert!(
            workspace
                .to_string()
                .contains(&format!("{entity} workspace must be a real directory")),
            "{workspace}"
        );
        let file = validate_regular_file(&file_link, entity).expect_err("linked file");
        assert!(
            file.to_string()
                .contains(&format!("{entity} path must be a regular file")),
            "{file}"
        );
    }
    assert_eq!(fs::read(&victim).expect("victim"), b"untouched");
}

#[test]
fn path_exists_tells_an_absent_path_from_a_present_one() {
    let root = tempfile::tempdir().expect("root");
    assert!(!path_exists(&root.path().join("absent")).expect("absent"));
    assert!(path_exists(root.path()).expect("present"));
}

#[cfg(unix)]
#[test]
fn path_exists_does_not_follow_a_link_and_reports_any_other_error() {
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir().expect("root");
    // A link whose target is gone still exists: it is the link that counts.
    let dangling = root.path().join("dangling");
    symlink(root.path().join("gone"), &dangling).expect("dangling link");
    assert!(path_exists(&dangling).expect("a dangling link exists"));

    // A path below a regular file is an error, not an absent path.
    let file = root.path().join("file");
    fs::write(&file, b"x").expect("file");
    let error = path_exists(&file.join("child")).expect_err("below a file");
    assert!(error.to_string().contains("cannot inspect "), "{error}");
}

#[test]
fn promote_replaces_the_target_whole_and_consumes_the_staging_file() {
    let root = tempfile::tempdir().expect("root");
    let staging = root.path().join("state.json.tmp");
    let target = root.path().join("state.json");

    fs::write(&staging, b"first").expect("staging");
    promote(&staging, &target).expect("publish over nothing");
    assert_eq!(fs::read(&target).expect("target"), b"first");
    assert!(!staging.exists());

    fs::write(&staging, b"second").expect("staging");
    promote(&staging, &target).expect("publish over a file");
    assert_eq!(fs::read(&target).expect("target"), b"second");
    assert!(!staging.exists());
}

/// On unix the barrier opens the directory, so an absent one fails. The
/// effect of the `fsync` itself is not observable from a test.
#[cfg(unix)]
#[test]
fn sync_directory_fails_on_an_absent_directory() {
    use super::sync_directory;

    let root = tempfile::tempdir().expect("root");
    sync_directory(root.path()).expect("sync a directory");
    sync_directory(&root.path().join("absent")).expect_err("an absent directory");
}
