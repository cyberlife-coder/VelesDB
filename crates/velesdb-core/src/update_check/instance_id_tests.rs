use super::*;
use std::collections::HashMap;
use tempfile::TempDir;

fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
    let table: HashMap<String, OsString> = pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), OsString::from(*v)))
        .collect();
    move |key| table.get(key).cloned()
}

fn is_id(s: &str) -> bool {
    s.len() == 32
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// A directory nothing can be created in, on every platform and for every
/// user including root: its parent is a regular file.
fn unusable_dir(tmp: &TempDir) -> PathBuf {
    let file = tmp.path().join("not-a-dir");
    std::fs::write(&file, b"x").expect("write blocker file");
    file.join("velesdb")
}

#[test]
fn test_config_dir_per_platform() {
    let home = [("HOME", "/home/u")];
    assert_eq!(
        user_config_dir(Platform::Linux, env(&home)),
        Some(PathBuf::from("/home/u/.config/velesdb"))
    );
    assert_eq!(
        user_config_dir(
            Platform::Linux,
            env(&[("XDG_CONFIG_HOME", "/x/cfg"), ("HOME", "/home/u")])
        ),
        Some(PathBuf::from("/x/cfg/velesdb"))
    );
    assert_eq!(
        user_config_dir(Platform::MacOs, env(&[("HOME", "/Users/u")])),
        Some(PathBuf::from(
            "/Users/u/Library/Application Support/velesdb"
        ))
    );
    assert_eq!(
        user_config_dir(
            Platform::Windows,
            env(&[("APPDATA", "C:\\Users\\u\\AppData\\Roaming")])
        ),
        Some(PathBuf::from("C:\\Users\\u\\AppData\\Roaming").join("velesdb"))
    );
}

#[test]
fn test_config_dir_absent_when_environment_gives_none() {
    assert_eq!(user_config_dir(Platform::Linux, env(&[])), None);
    assert_eq!(user_config_dir(Platform::MacOs, env(&[])), None);
    assert_eq!(
        user_config_dir(Platform::Windows, env(&[("HOME", "/h")])),
        None
    );
    // A relative XDG_CONFIG_HOME is ignored per the XDG spec, as is an empty one.
    assert_eq!(
        user_config_dir(
            Platform::Linux,
            env(&[("XDG_CONFIG_HOME", "rel"), ("HOME", "/h")])
        ),
        Some(PathBuf::from("/h/.config/velesdb"))
    );
    assert_eq!(
        user_config_dir(
            Platform::Linux,
            env(&[("XDG_CONFIG_HOME", ""), ("HOME", "/h")])
        ),
        Some(PathBuf::from("/h/.config/velesdb"))
    );
}

#[test]
fn test_candidates_put_user_config_before_data_dir() {
    let dirs = candidate_dirs(
        Platform::Linux,
        env(&[("HOME", "/h")]),
        Some(Path::new("/data")),
    );
    assert_eq!(
        dirs,
        vec![PathBuf::from("/h/.config/velesdb"), PathBuf::from("/data")]
    );
    let no_data = candidate_dirs(Platform::Linux, env(&[("HOME", "/h")]), None);
    assert_eq!(no_data, vec![PathBuf::from("/h/.config/velesdb")]);
}

#[test]
fn test_install_id_persists_across_calls() {
    let tmp = TempDir::new().expect("tempdir");
    let dirs = vec![tmp.path().join("cfg")];
    let first = resolve_install_id(&dirs);
    let second = resolve_install_id(&dirs);
    assert!(is_id(&first), "id {first:?} is 32 lowercase hex");
    assert_eq!(first, second, "second call reads the file the first wrote");
    let on_disk = std::fs::read_to_string(dirs[0].join(FILE_NAME)).expect("id file written");
    assert_eq!(on_disk.trim(), first);
}

#[test]
fn test_install_id_differs_between_installs() {
    let a = TempDir::new().expect("tempdir a");
    let b = TempDir::new().expect("tempdir b");
    assert_ne!(
        resolve_install_id(&[a.path().to_path_buf()]),
        resolve_install_id(&[b.path().to_path_buf()])
    );
}

#[test]
fn test_corrupt_id_file_is_regenerated_then_stable() {
    let tmp = TempDir::new().expect("tempdir");
    let path = tmp.path().join(FILE_NAME);
    for garbage in ["", "not an id", "ABCDEF0123456789ABCDEF0123456789", "abc"] {
        std::fs::write(&path, garbage).expect("write garbage");
        let id = resolve_install_id(&[tmp.path().to_path_buf()]);
        assert!(is_id(&id), "regenerated id {id:?} after {garbage:?}");
        assert_eq!(resolve_install_id(&[tmp.path().to_path_buf()]), id);
    }
}

#[test]
fn test_unusable_dir_falls_through_to_next_candidate() {
    let tmp = TempDir::new().expect("tempdir");
    let good = tmp.path().join("data");
    let id = resolve_install_id(&[unusable_dir(&tmp), good.clone()]);
    assert!(is_id(&id));
    assert_eq!(
        std::fs::read_to_string(good.join(FILE_NAME))
            .expect("fallback file")
            .trim(),
        id
    );
}

#[test]
fn test_nothing_writable_degrades_to_ephemeral_id() {
    let tmp = TempDir::new().expect("tempdir");
    let dirs = vec![unusable_dir(&tmp)];
    let first = resolve_install_id(&dirs);
    let second = resolve_install_id(&dirs);
    assert!(is_id(&first) && is_id(&second));
    assert_ne!(first, second, "nothing persisted, so each call is a new id");
    assert!(
        is_id(&resolve_install_id(&[])),
        "no candidate at all still yields an id"
    );
}

#[cfg(unix)]
#[test]
fn test_read_only_dir_degrades_to_ephemeral_id() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = TempDir::new().expect("tempdir");
    let dir = tmp.path().join("ro");
    std::fs::create_dir(&dir).expect("mkdir");
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).expect("chmod");
    // Root ignores the mode bits; the case is then not a read-only directory.
    let writable = std::fs::write(dir.join("probe"), b"").is_ok();
    let id = resolve_install_id(std::slice::from_ref(&dir));
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).expect("restore");
    assert!(is_id(&id));
    if !writable {
        assert!(
            !dir.join(FILE_NAME).exists(),
            "nothing written to a read-only dir"
        );
    }
}

#[test]
fn test_hash_is_64_lowercase_hex_and_versioned() {
    let hash = hash_install_id("0123456789abcdef0123456789abcdef");
    assert_eq!(hash.len(), 64);
    assert!(hash
        .bytes()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
    assert_eq!(hash, hash_install_id("0123456789abcdef0123456789abcdef"));
    assert_ne!(hash, hash_install_id("fedcba9876543210fedcba9876543210"));
    // Fixed vector: sha256("velesdb-install-v2:" + id). Changing the prefix
    // re-identifies every install, so it must be a deliberate edit of this line.
    assert_eq!(
        hash,
        "b0a080e4c1d9a83f8cb2f15086b993de06aee26f0ce3ea3c018870161e025c4d"
    );
}

/// One round of [`race`]: its directory and what each thread returned.
type Round = (PathBuf, Vec<Option<String>>);

/// Starts four threads together on a fresh path per round, after `prepare`
/// has run on it, and returns what each of them got from `load_or_create`
/// along with that path, round by round. The path does not exist unless
/// `prepare` creates it: a real first start finds no `velesdb/` directory,
/// and the racing starts must create it together.
fn race(rounds: usize, prepare: impl Fn(&Path)) -> (TempDir, Vec<Round>) {
    use std::sync::{Arc, Barrier};
    const THREADS: usize = 4;
    let tmp = TempDir::new().expect("tempdir");
    let base = tmp.path().to_path_buf();
    let results = (0..rounds)
        .map(|round| {
            let dir = base.join(format!("round-{round}"));
            prepare(&dir);
            let (shared, barrier) = (Arc::new(dir.clone()), Arc::new(Barrier::new(THREADS)));
            let handles: Vec<_> = (0..THREADS)
                .map(|_| {
                    let (shared, barrier) = (Arc::clone(&shared), Arc::clone(&barrier));
                    std::thread::spawn(move || {
                        barrier.wait();
                        load_or_create(&shared)
                    })
                })
                .collect();
            let ids = handles
                .into_iter()
                .map(|h| h.join().expect("thread"))
                .collect();
            (dir, ids)
        })
        .collect();
    (tmp, results)
}

fn assert_one_id(round: usize, ids: &[Option<String>]) {
    assert!(ids.iter().all(Option::is_some), "round {round}: {ids:?}");
    assert!(
        ids.iter().all(|id| *id == ids[0]),
        "round {round}: processes disagree: {ids:?}"
    );
}

/// Two processes starting for the first time must end with one id. The old
/// implementation created the file, then wrote it: a racer saw the winner's
/// empty file as corrupt, deleted it, and the two ended with different ids.
#[test]
fn test_concurrent_first_starts_agree_on_one_id() {
    let (_tmp, rounds) = race(300, |_| {});
    for (round, (_, ids)) in rounds.iter().enumerate() {
        assert_one_id(round, ids);
    }
}

#[test]
fn test_no_temporary_file_is_left_behind() {
    let tmp = TempDir::new().expect("tempdir");
    let id = load_or_create(tmp.path()).expect("id");
    let temporaries: Vec<_> = std::fs::read_dir(tmp.path())
        .expect("read_dir")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .filter(|name| {
            std::path::Path::new(name)
                .extension()
                .is_some_and(|ext| ext == "tmp")
        })
        .collect();
    assert!(temporaries.is_empty(), "{id}: {temporaries:?}");
}

/// The same race, started from a damaged file instead of a missing one (an
/// empty `install-id` is what a crash used to leave): every racer sees the
/// corruption, and they must still end with one id.
#[test]
fn test_concurrent_starts_over_a_corrupt_file_agree_on_one_id() {
    let (_tmp, rounds) = race(100, |dir| {
        std::fs::create_dir(dir).expect("mkdir");
        std::fs::write(dir.join(FILE_NAME), "").expect("write corrupt file");
    });
    for (round, (dir, ids)) in rounds.iter().enumerate() {
        assert_one_id(round, ids);
        assert_eq!(
            read_id(&dir.join(FILE_NAME)),
            ids[0],
            "round {round}: the file holds the id everyone returned"
        );
        let mut leftovers: Vec<_> = std::fs::read_dir(dir)
            .expect("read_dir")
            .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
            .collect();
        leftovers.sort();
        assert_eq!(leftovers, [LOCK_NAME, FILE_NAME], "round {round}");
    }
}

/// A file that merely looks like a lock must never turn into a loop: the
/// old generation scheme unlinked every number up to the highest one it
/// found, so `.install-id.lock.1000000` cost seconds and a larger one
/// minutes.
#[test]
fn test_stray_lock_like_files_cost_nothing() {
    let tmp = TempDir::new().expect("tempdir");
    std::fs::write(tmp.path().join(FILE_NAME), "").expect("corrupt id file");
    for name in [
        ".install-id.lock.1000000",
        ".install-id.lock.4000000000",
        ".install-id.lock.4294967295",
        ".install-id.lock.not-a-number",
    ] {
        std::fs::write(tmp.path().join(name), "").expect("stray file");
    }

    let started = std::time::Instant::now();
    let id = load_or_create(tmp.path());

    assert!(id.is_some());
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "took {:?}",
        started.elapsed()
    );
}

/// A lock another process holds for longer than the wait must not hang the
/// start: it gives up on this directory and the caller falls through to the
/// next candidate, or to an id of its own.
#[test]
fn test_a_held_lock_is_waited_for_then_given_up_on() {
    let tmp = TempDir::new().expect("tempdir");
    let holder = lock_exclusive(&tmp.path().join(LOCK_NAME), Duration::from_secs(1))
        .expect("the test takes the lock first");

    let started = std::time::Instant::now();
    let id = load_or_create_within(tmp.path(), Duration::from_millis(100));

    assert_eq!(
        id, None,
        "no id is written while someone else holds the lock"
    );
    assert!(started.elapsed() >= Duration::from_millis(100));
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
    assert!(!tmp.path().join(FILE_NAME).exists());
    drop(holder);
    assert!(
        load_or_create(tmp.path()).is_some(),
        "free again once released"
    );
}

/// A lock file left behind by a process that died is just a file: the
/// kernel dropped its lock with the process.
#[test]
fn test_a_leftover_lock_file_is_harmless() {
    let tmp = TempDir::new().expect("tempdir");
    std::fs::write(tmp.path().join(LOCK_NAME), "").expect("leftover lock file");
    std::fs::write(tmp.path().join(FILE_NAME), "damaged").expect("corrupt id file");

    let started = std::time::Instant::now();
    let id = load_or_create(tmp.path()).expect("an id");

    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(read_id(&tmp.path().join(FILE_NAME)), Some(id));
}

#[test]
fn test_lock_step_covers_every_outcome_of_try_lock() {
    use std::fs::TryLockError;
    let wait = Duration::from_millis(100);

    assert_eq!(lock_step(&Ok(()), Duration::ZERO, wait), LockStep::Acquired);
    assert_eq!(
        lock_step(&Ok(()), wait * 10, wait),
        LockStep::Acquired,
        "late but ours"
    );

    assert_eq!(
        lock_step(
            &Err(TryLockError::WouldBlock),
            Duration::from_millis(10),
            wait
        ),
        LockStep::Retry,
        "contention within the wait is retried"
    );
    assert_eq!(
        lock_step(&Err(TryLockError::WouldBlock), wait, wait),
        LockStep::GiveUp,
        "contention past the wait is given up on"
    );

    let unsupported = || TryLockError::Error(std::io::Error::from(std::io::ErrorKind::Unsupported));
    assert_eq!(
        lock_step(&Err(unsupported()), Duration::ZERO, wait),
        LockStep::GiveUp,
        "a filesystem that cannot lock is given up on at once, never written without the lock"
    );
}

/// Runs `f` on a thread and fails the test if it has not returned within
/// `secs`, so a regression to a blocking `open` fails instead of hanging.
fn within<T: Send + 'static>(secs: u64, f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(Duration::from_secs(secs))
        .expect("call did not return: it blocked on a special file")
}

#[cfg(unix)]
fn mkfifo(path: &Path) {
    let status = std::process::Command::new("mkfifo")
        .arg(path)
        .status()
        .expect("run mkfifo");
    assert!(status.success(), "mkfifo failed");
}

#[cfg(unix)]
#[test]
fn test_fifo_named_like_an_install_file_is_unusable_not_blocking() {
    for name in [LOCK_NAME, FILE_NAME] {
        let tmp = TempDir::new().expect("tempdir");
        mkfifo(&tmp.path().join(name));
        let dir = tmp.path().to_path_buf();
        let loaded = within(10, move || load_or_create(&dir));
        // A special lock file makes the directory unusable; a special id file
        // is not read and is replaced by the rename.
        assert_eq!(loaded.is_none(), name == LOCK_NAME, "{name}");
        // Reading the id file afterwards must not block either: it holds the
        // id the rename put in place of a special id file, and nothing when
        // the special lock file left the directory unusable.
        let file = tmp.path().join(FILE_NAME);
        let read = within(10, move || read_id(&file));
        assert_eq!(read.is_some(), name == FILE_NAME, "{name}");
    }
}

#[cfg(unix)]
#[test]
fn test_dangling_symlink_named_like_an_install_file_is_not_followed() {
    for name in [LOCK_NAME, FILE_NAME] {
        let tmp = TempDir::new().expect("tempdir");
        let target = tmp.path().join("elsewhere");
        std::os::unix::fs::symlink(&target, tmp.path().join(name)).expect("symlink");
        let dir = tmp.path().to_path_buf();
        let loaded = within(10, move || load_or_create(&dir));
        assert_eq!(loaded.is_none(), name == LOCK_NAME, "{name}");
        assert!(
            !target.exists(),
            "{name} was followed and created its target"
        );
    }
}

#[test]
fn test_directory_named_like_an_install_file_is_unusable() {
    for name in [LOCK_NAME, FILE_NAME] {
        let tmp = TempDir::new().expect("tempdir");
        std::fs::create_dir(tmp.path().join(name)).expect("create directory");
        let dir = tmp.path().to_path_buf();
        let loaded = within(10, move || load_or_create(&dir));
        assert!(
            loaded.is_none(),
            "{name}: a directory in the place of an install file must make the dir unusable"
        );
    }
}
