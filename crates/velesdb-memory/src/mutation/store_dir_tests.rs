//! `StoreDir`: every method resolves a bare name against the directory
//! descriptor pinned at `open`, so a workspace renamed or swapped after
//! `open` is never followed, while names inside the pinned directory keep
//! working exactly like the path-based helpers they replace.

use std::fs;

use super::StoreDir;

#[test]
fn open_refuses_a_symlinked_or_missing_workspace_with_the_stores_name() {
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir().expect("root");
    let real = root.path().join("real");
    fs::create_dir(&real).expect("real");
    let link = root.path().join("link");
    symlink(&real, &link).expect("link");

    let refused = StoreDir::open(&link, "test").expect_err("linked workspace");
    assert!(
        refused
            .to_string()
            .contains("test workspace must be a real directory"),
        "{refused}"
    );
    let missing = StoreDir::open(&root.path().join("absent"), "test").expect_err("absent");
    assert!(
        missing
            .to_string()
            .contains("cannot inspect test workspace: "),
        "{missing}"
    );
    let file = root.path().join("file");
    fs::write(&file, b"x").expect("file");
    let not_dir = StoreDir::open(&file, "test").expect_err("a file is no workspace");
    assert!(
        not_dir
            .to_string()
            .contains("test workspace must be a real directory"),
        "{not_dir}"
    );
}

/// #2408: the descriptor pins the workspace *inode*, so a directory renamed
/// away and replaced after `open` — by a symlink here, a plain rename in a
/// second pass — is never followed: every op keeps landing in the original.
#[test]
fn a_directory_swapped_after_open_is_never_followed() {
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir().expect("root");
    let workspace = root.path().join("workspace");
    fs::create_dir(&workspace).expect("workspace");
    fs::write(workspace.join("state"), b"original").expect("seed");
    let dir = StoreDir::open(&workspace, "test").expect("open");

    let foreign = root.path().join("foreign");
    fs::create_dir(&foreign).expect("foreign");
    fs::write(foreign.join("state"), b"foreign").expect("foreign seed");
    fs::write(foreign.join("planted"), b"planted").expect("planted");

    let moved = root.path().join("moved");
    fs::rename(&workspace, &moved).expect("move aside");
    symlink(&foreign, &workspace).expect("swap in a foreign directory");

    let mut opened = dir
        .open_regular_file("state", "test")
        .expect("open in pinned dir");
    let mut content = String::new();
    std::io::Read::read_to_string(&mut opened, &mut content).expect("read");
    assert_eq!(content, "original");
    assert!(!dir.exists("planted").expect("planted is foreign-only"));
    assert_eq!(dir.entries().expect("entries"), vec!["state".to_owned()]);

    dir.promote("state", "renamed").expect("promote in pinned");
    dir.sync().expect("sync pinned dir");
    assert_eq!(
        fs::read(moved.join("renamed")).expect("renamed in original"),
        b"original"
    );
    assert_eq!(
        fs::read_dir(&foreign).expect("foreign").count(),
        2,
        "the swapped-in directory must be untouched"
    );
}

#[test]
fn create_new_refuses_any_existing_entry_and_promote_replaces_names() {
    let root = tempfile::tempdir().expect("root");
    let dir = StoreDir::open(root.path(), "test").expect("open");

    fs::write(root.path().join("existing"), b"x").expect("existing");
    dir.create_new("existing", 0o666)
        .expect_err("an existing entry is refused");

    let mut staging = dir.create_new("state.tmp", 0o666).expect("staging");
    std::io::Write::write_all(&mut staging, b"v1").expect("write");
    dir.promote("state.tmp", "state").expect("promote");
    assert_eq!(fs::read(root.path().join("state")).expect("state"), b"v1");

    dir.remove("state").expect("remove");
    assert!(!dir.exists("state").expect("removed"));
}

#[test]
fn validate_regular_refuses_a_link_and_create_new_does_not_follow_it() {
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir().expect("root");
    let dir = StoreDir::open(root.path(), "test").expect("open");
    let victim = root.path().join("victim");
    fs::write(&victim, b"untouched").expect("victim");
    symlink(&victim, root.path().join("link")).expect("link");

    dir.validate_regular("link", "test")
        .expect_err("a symlink is not a regular file");
    dir.create_new("link", 0o666)
        .expect_err("create_new does not follow a link");
    dir.open_regular_file_allow_hard_links("link", "test")
        .expect_err("a symlink is refused on open");
    assert_eq!(fs::read(&victim).expect("victim"), b"untouched");
}
