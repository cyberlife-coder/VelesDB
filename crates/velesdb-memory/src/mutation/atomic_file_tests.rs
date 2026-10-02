//! The shared atomic-replace primitives: each check names the store that
//! called it in its message and refuses a symlink or a path of the wrong
//! kind, `path_exists` does not follow links, and `promote` replaces a file
//! whole.

use std::fs;

#[cfg(unix)]
use super::open_regular_file_allow_hard_links;
use super::{open_regular_file, path_exists, promote, validate_regular_file, validate_workspace};

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
fn open_regular_file_reads_and_writes_a_real_file() {
    let root = tempfile::tempdir().expect("root");
    let path = root.path().join("state");
    fs::write(&path, b"first").expect("seed");

    let mut options = fs::OpenOptions::new();
    options.read(true).write(true);
    let mut file = open_regular_file(&path, "test", options).expect("a regular file opens");
    let mut read = Vec::new();
    std::io::Read::read_to_end(&mut file, &mut read).expect("read");
    assert_eq!(read, b"first");
}

#[cfg(unix)]
#[test]
fn open_regular_file_refuses_a_symlink_and_says_so_under_each_stores_name() {
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir().expect("root");
    let victim = root.path().join("victim");
    fs::write(&victim, b"untouched").expect("victim");
    let link = root.path().join("link");
    symlink(&victim, &link).expect("link");

    for entity in ENTITIES {
        let mut options = fs::OpenOptions::new();
        options.read(true);
        let refused = open_regular_file(&link, entity, options).expect_err("a symlink");
        assert!(
            refused
                .to_string()
                .contains(&format!("{entity} path must be a regular file")),
            "{refused}"
        );
    }
    assert_eq!(fs::read(&victim).expect("victim"), b"untouched");
}

/// #2404: `validate_regular_file` followed by a plain `open` were two
/// separate syscalls against the *path*, with a window between them for a
/// symlink to be swapped in. This test does not, and cannot, exercise that
/// window from a single thread — it swaps the link in before calling
/// `open_regular_file` at all, which the old two-step code would have caught
/// too if the swap landed before its own first step. What it does pin: a
/// symlink present at call time is refused, and, unlike the two-step code,
/// `open_regular_file` has no separate first step for a check to pass and a
/// later step to race against — the one check it makes is `fstat` on the
/// file descriptor `open(O_NOFOLLOW)` already returned, not a second `stat`
/// of the path, so there is no second syscall left to land a swap between.
/// A mutant that reverts this function to `validate_regular_file(path,
/// entity)?; options.open(path)` still passes this specific test (the swap
/// is already in place before either step runs), and no other test in this
/// file catches it either — two attempts at a concurrent swap-under-load
/// test were tried and dropped: neither reliably won a kernel-level race
/// that is only a couple of syscalls wide (measured kill rates from ~3% to
/// ~55% across designs and runs, nowhere near a bound a CI gate could rely
/// on). That mutant is refused by construction, not by a test: `stat` then
/// `open` are two operations on the *path*, so anything can happen to the
/// path between them; `open(O_NOFOLLOW)` then `fstat` on the file
/// descriptor it returns are two operations on the *same already-opened
/// file*, and nothing done to the path afterward can change which file
/// that descriptor points to. Forcing and observing the first kind of race
/// deterministically from a plain unit test — reliably enough to gate a
/// merge on it — needs syscall-level interposition (`ptrace`, a seccomp
/// user-space notifier, or an `LD_PRELOAD` shim), which is out of
/// proportion for this fix.
#[cfg(unix)]
#[test]
fn open_regular_file_refuses_a_symlink_present_at_call_time_with_no_check_step_of_its_own() {
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir().expect("root");
    let victim = root.path().join("victim");
    fs::write(&victim, b"untouched").expect("victim");
    let path = root.path().join("state");
    fs::write(&path, b"regular").expect("a real file first");

    // What a prior `validate_regular_file(&path, ..)` check would have seen:
    // a regular file. The swap happens right after, before the only
    // remaining step (open_regular_file's single open+fstat).
    fs::remove_file(&path).expect("remove the regular file");
    symlink(&victim, &path).expect("swap in a link to the victim");

    let mut options = fs::OpenOptions::new();
    options.read(true);
    let refused = open_regular_file(&path, "test", options).expect_err("swapped-in symlink");
    assert!(
        refused
            .to_string()
            .contains("test path must be a regular file"),
        "{refused}"
    );
    assert_eq!(fs::read(&victim).expect("victim"), b"untouched");
}

/// A planted FIFO must be refused promptly, not hang the caller. Opening a
/// FIFO for read blocks until a writer opens the other end; without
/// `O_NONBLOCK` this call would wait forever instead of reaching the
/// "not a regular file" refusal below (a mutant that drops `O_NONBLOCK`
/// times out this test rather than failing it cleanly).
#[cfg(unix)]
#[test]
fn open_regular_file_refuses_a_fifo_without_blocking() {
    let root = tempfile::tempdir().expect("root");
    let fifo = root.path().join("pipe");
    let status = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .expect("test: run mkfifo");
    assert!(status.success(), "mkfifo failed: {status}");

    let (tx, rx) = std::sync::mpsc::channel();
    let path = fifo.clone();
    std::thread::spawn(move || {
        let mut options = fs::OpenOptions::new();
        options.read(true);
        let result = open_regular_file(&path, "test", options).map(|_| ());
        let _ = tx.send(result.map_err(|err| err.to_string()));
    });

    let refused = rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("open_regular_file must return instead of blocking on the FIFO")
        .expect_err("a FIFO is not a regular file");
    assert!(
        refused.contains("test path must be a regular file"),
        "{refused}"
    );
}

/// #2407 (round 5): a hard link to a file outside the store is,
/// structurally, an ordinary regular file, so `is_file` alone accepts it —
/// but `fstat` also reports `nlink`, which this check refuses above 1 for
/// every current caller of `open_regular_file` (the journal, and —
/// deliberately, per #2426 — the online-migration controller state and job
/// state too, even though a hard-link based backup of either can trigger
/// this exact refusal on an untampered file).
#[cfg(unix)]
#[test]
fn open_regular_file_refuses_a_hard_link() {
    let root = tempfile::tempdir().expect("root");
    let victim = root.path().join("victim");
    fs::write(&victim, b"untouched").expect("victim");
    let path = root.path().join("state");
    fs::hard_link(&victim, &path).expect("hard link");

    let mut options = fs::OpenOptions::new();
    options.read(true);
    let refused = open_regular_file(&path, "test", options).expect_err("a hard link");
    assert!(
        refused
            .to_string()
            .contains("test path must be a regular file"),
        "{refused}"
    );
    assert_eq!(fs::read(&victim).expect("victim"), b"untouched");
}

/// #2409 round 10: `nlink > 1` (not `!= 1`) is the refusal, specifically so
/// a reader racing a legitimate rename-replace of the same path — which
/// transiently drops the OLD inode's `nlink` to 0 on a handle a reader
/// already has open — is never refused. `controller/state.rs` and
/// `online_migration/job_state.rs` are exactly this shape (`write_synced` to
/// a staging file, then `promote`), and this race had no direct test:
/// round 7 moved the extraction-job-record call site this PR originally
/// cited off `open_regular_file` entirely, so the two stress-tested
/// extraction-recovery tests no longer exercise this guard at all. Proven
/// by mutation: reverting to `nlink() != 1` makes this test fail (observed
/// on macOS: roughly 1,700 of 20,000 iterations, about 8-9%, varying by
/// run and machine load).
#[cfg(unix)]
#[test]
fn open_regular_file_never_refuses_a_legitimate_rename_replace_race() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    const ITERATIONS: usize = 20_000;

    let root = tempfile::tempdir().expect("root");
    let path = root.path().join("state");
    let staging = root.path().join("state.tmp");
    fs::write(&path, b"v0").expect("seed");

    let spurious_refusals = AtomicUsize::new(0);

    std::thread::scope(|scope| {
        scope.spawn(|| {
            for i in 0..ITERATIONS {
                fs::write(&staging, format!("v{i}").as_bytes()).expect("write staging");
                promote(&staging, &path).expect("promote");
            }
        });

        for _ in 0..ITERATIONS {
            let mut options = fs::OpenOptions::new();
            options.read(true);
            if open_regular_file(&path, "test", options).is_err() {
                spurious_refusals.fetch_add(1, Ordering::Relaxed);
            }
        }
    });

    assert_eq!(
        spurious_refusals.load(Ordering::Relaxed),
        0,
        "a reader racing a legitimate rename-replace must never be refused as a hard link"
    );
}

/// `open_regular_file_allow_hard_links` is for a caller that either only
/// reads someone else's tree (a diagnostic source) or owns a file it never
/// reopens to write into (a migration lock, an extraction job record): it
/// accepts the exact shape `open_regular_file_refuses_a_hard_link` above
/// refuses, because a hard-link based backup routinely produces it on
/// ordinary files. The symlink-swap protection (#2404) is shared code and
/// still applies in full.
#[cfg(unix)]
#[test]
fn open_regular_file_allow_hard_links_accepts_a_hard_link_but_still_refuses_a_symlink() {
    let root = tempfile::tempdir().expect("root");
    let victim = root.path().join("victim");
    fs::write(&victim, b"content").expect("victim");
    let aliased = root.path().join("aliased");
    fs::hard_link(&victim, &aliased).expect("hard link");

    let mut options = fs::OpenOptions::new();
    options.read(true);
    open_regular_file_allow_hard_links(&aliased, "test", options)
        .expect("a hard link must be accepted by the hard-link-tolerant open");

    let outside = tempfile::tempdir().expect("outside");
    let secret = outside.path().join("secret");
    fs::write(&secret, b"do-not-read-me").expect("secret");
    let linked = root.path().join("state");
    std::os::unix::fs::symlink(&secret, &linked).expect("symlink");

    let mut options = fs::OpenOptions::new();
    options.read(true);
    let refused = open_regular_file_allow_hard_links(&linked, "test", options)
        .expect_err("a symlink must still be refused");
    assert!(
        refused
            .to_string()
            .contains("test path must be a regular file"),
        "{refused}"
    );
}

/// A Unix domain socket is refused the same way a FIFO is, and with the
/// same clean message rather than a raw, platform-specific `open` errno —
/// `open_error` falls back to a `symlink_metadata` lookup, purely to choose
/// the message, once `open` has already failed and the refusal is decided.
#[cfg(unix)]
#[test]
fn open_regular_file_refuses_a_socket_with_the_clean_message() {
    use std::os::unix::net::UnixListener;

    let root = tempfile::tempdir().expect("root");
    let path = root.path().join("state");
    let _listener = UnixListener::bind(&path).expect("bind a unix socket");

    let mut options = fs::OpenOptions::new();
    options.read(true);
    let refused = open_regular_file(&path, "test", options).expect_err("a socket");
    assert!(
        refused
            .to_string()
            .contains("test path must be a regular file"),
        "{refused}"
    );
}

/// The round-5 `open_error` fallback must not relabel a genuine regular
/// file that failed to open for an unrelated reason (permissions, here) as
/// "must be a regular file" — that message is reserved for a file that
/// really is the wrong kind. Skipped wherever permission bits aren't
/// enforced (running as root, some container/CI filesystems): checked
/// directly, by trying the same open the assertion depends on.
#[cfg(unix)]
#[test]
fn open_regular_file_keeps_the_raw_message_for_a_permission_denied_regular_file() {
    use std::os::unix::fs::PermissionsExt;

    let root = tempfile::tempdir().expect("root");
    let path = root.path().join("state");
    fs::write(&path, b"regular").expect("a real file");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).expect("chmod 000");

    if fs::File::open(&path).is_ok() {
        eprintln!("test: skipped, permission bits are not enforced here");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("restore");
        return;
    }

    let mut options = fs::OpenOptions::new();
    options.read(true);
    let refused = open_regular_file(&path, "test", options).expect_err("permission denied");
    let message = refused.to_string();
    assert!(
        message.contains("cannot open test file"),
        "a permission error must keep the raw message, got: {message}"
    );
    assert!(
        !message.contains("must be a regular file"),
        "a real regular file must never be relabeled as the wrong kind, got: {message}"
    );

    // Restore permissions so the tempdir can clean itself up.
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("restore permissions");
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
