use std::fs;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

use super::super::{
    AnchorError, AnchorObservation, AnchorProbe, ContentDigest, FileAnchor, RepoPath,
};
use super::AnchorRoot;

/// An allowlist holding `dir` (canonical), and a checkout root inside it.
fn checkout() -> (TempDir, Vec<PathBuf>, PathBuf) {
    let dir = TempDir::new().unwrap();
    let allowed = fs::canonicalize(dir.path()).unwrap();
    let repo = allowed.join("repo");
    fs::create_dir_all(repo.join("src")).unwrap();
    fs::write(repo.join("src/lib.rs"), "fn main() {}\n").unwrap();
    (dir, vec![allowed], repo)
}

fn path(text: &str) -> RepoPath {
    RepoPath::parse(text).unwrap()
}

fn root(repo: &Path, allowlist: &[PathBuf]) -> AnchorRoot {
    AnchorRoot::open(repo.to_str().unwrap(), allowlist).unwrap()
}

#[test]
fn no_allowlist_disables_anchoring() {
    let (_dir, _allowlist, repo) = checkout();
    assert_eq!(
        AnchorRoot::open(repo.to_str().unwrap(), &[]).unwrap_err(),
        AnchorError::Disabled
    );
}

#[test]
fn a_root_outside_the_allowlist_is_refused_citing_the_requested_root() {
    let (_dir, allowlist, _repo) = checkout();
    let outside = TempDir::new().unwrap();
    let requested = outside.path().to_str().unwrap().to_owned();
    match AnchorRoot::open(&requested, &allowlist) {
        Err(AnchorError::RootRefused { root, .. }) => assert_eq!(root, requested),
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn a_relative_root_is_refused() {
    let (_dir, allowlist, _repo) = checkout();
    assert!(matches!(
        AnchorRoot::open("repo", &allowlist),
        Err(AnchorError::RootRefused { .. })
    ));
}

#[cfg(unix)]
#[test]
fn a_symlinked_root_escaping_the_allowlist_is_refused() {
    let (dir, allowlist, _repo) = checkout();
    let outside = TempDir::new().unwrap();
    let link = dir.path().join("escape");
    std::os::unix::fs::symlink(outside.path(), &link).unwrap();
    assert!(matches!(
        AnchorRoot::open(link.to_str().unwrap(), &allowlist),
        Err(AnchorError::RootRefused { .. })
    ));
}

#[test]
fn digest_matches_the_bytes_on_disk() {
    let (_dir, allowlist, repo) = checkout();
    let mut root = root(&repo, &allowlist);
    assert_eq!(
        root.digest(&path("src/lib.rs")).unwrap(),
        ContentDigest::of(b"fn main() {}\n")
    );
}

#[test]
fn a_missing_file_cannot_be_anchored_and_observes_as_missing() {
    let (_dir, allowlist, repo) = checkout();
    let mut root = root(&repo, &allowlist);
    assert!(matches!(
        root.digest(&path("src/gone.rs")),
        Err(AnchorError::Unreadable { .. })
    ));
    let anchor = FileAnchor {
        path: path("src/gone.rs"),
        digest: ContentDigest::of(b"x"),
    };
    assert_eq!(root.observe(&anchor), AnchorObservation::Missing);
}

#[test]
fn observation_compares_the_anchored_digest_with_the_current_bytes() {
    let (_dir, allowlist, repo) = checkout();
    let unchanged = FileAnchor {
        path: path("src/lib.rs"),
        digest: ContentDigest::of(b"fn main() {}\n"),
    };
    let changed = FileAnchor {
        path: path("src/lib.rs"),
        digest: ContentDigest::of(b"older bytes"),
    };
    let mut root = root(&repo, &allowlist);
    assert_eq!(root.observe(&unchanged), AnchorObservation::Unchanged);
    assert_eq!(root.observe(&changed), AnchorObservation::Changed);
}

#[test]
fn a_directory_is_unverifiable_never_stale() {
    let (_dir, allowlist, repo) = checkout();
    let mut root = root(&repo, &allowlist);
    let anchor = FileAnchor {
        path: path("src"),
        digest: ContentDigest::of(b""),
    };
    assert_eq!(root.observe(&anchor), AnchorObservation::Unverifiable);
}

#[test]
fn a_file_over_the_size_cap_is_unverifiable() {
    let (_dir, allowlist, repo) = checkout();
    let big = vec![b'x'; crate::limits::MAX_ANCHOR_FILE_BYTES + 1];
    fs::write(repo.join("big.bin"), big).unwrap();
    let mut root = root(&repo, &allowlist);
    assert!(root.digest(&path("big.bin")).is_err());
    let anchor = FileAnchor {
        path: path("big.bin"),
        digest: ContentDigest::of(b""),
    };
    assert_eq!(root.observe(&anchor), AnchorObservation::Unverifiable);
}

#[cfg(unix)]
#[test]
fn a_symlink_leaving_the_checkout_is_refused_not_followed() {
    let (dir, allowlist, repo) = checkout();
    // The target is inside the allowlist but outside the checkout root:
    // the checkout root is the boundary, not the allowlist.
    fs::write(dir.path().join("sibling.txt"), "secret").unwrap();
    std::os::unix::fs::symlink(dir.path().join("sibling.txt"), repo.join("src/link.txt")).unwrap();
    let mut root = root(&repo, &allowlist);
    assert!(root.digest(&path("src/link.txt")).is_err());
    let anchor = FileAnchor {
        path: path("src/link.txt"),
        digest: ContentDigest::of(b"secret"),
    };
    assert_eq!(root.observe(&anchor), AnchorObservation::Unverifiable);
}

#[cfg(unix)]
#[test]
fn a_symlink_inside_the_checkout_is_followed() {
    let (_dir, allowlist, repo) = checkout();
    std::os::unix::fs::symlink(repo.join("src/lib.rs"), repo.join("alias.rs")).unwrap();
    let mut root = root(&repo, &allowlist);
    assert_eq!(
        root.digest(&path("alias.rs")).unwrap(),
        ContentDigest::of(b"fn main() {}\n")
    );
}

#[test]
fn a_file_is_read_once_per_request() {
    let (_dir, allowlist, repo) = checkout();
    let mut root = root(&repo, &allowlist);
    let first = root.digest(&path("src/lib.rs")).unwrap();
    fs::write(repo.join("src/lib.rs"), "rewritten").unwrap();
    assert_eq!(
        root.digest(&path("src/lib.rs")).unwrap(),
        first,
        "one request judges one snapshot of each file"
    );
}

/// Given an anchored file that is swapped, again and again, between a
/// regular file and a FIFO, when the probe observes it meanwhile, then no
/// observation ever blocks. A FIFO opened for reading waits for a writer
/// forever; checking the path's type and then opening the path leaves room
/// for that swap, and the recall — with the daemon queued behind it — would
/// hang. Reproduced at iteration 737 before the probe checked the opened
/// handle instead.
#[cfg(unix)]
#[test]
fn a_fifo_swapped_in_under_an_anchor_never_blocks_the_probe() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{mpsc, Arc};
    use std::time::{Duration, Instant};

    const RACE: Duration = Duration::from_secs(3);
    const WATCHDOG: Duration = Duration::from_secs(10);

    let (_dir, allowlist, repo) = checkout();
    let anchor = FileAnchor {
        path: path("src/lib.rs"),
        digest: ContentDigest::of(b"fn main() {}\n"),
    };
    let stop = Arc::new(AtomicBool::new(false));
    let swapper = {
        let stop = Arc::clone(&stop);
        let src = repo.join("src");
        std::thread::spawn(move || {
            let target = src.join("lib.rs");
            let (regular, fifo) = (src.join("regular.tmp"), src.join("fifo.tmp"));
            while !stop.load(Ordering::Relaxed) {
                fs::write(&regular, "fn main() {}\n").unwrap();
                fs::rename(&regular, &target).unwrap();
                let made = std::process::Command::new("mkfifo")
                    .arg(&fifo)
                    .status()
                    .expect("run mkfifo");
                assert!(made.success(), "mkfifo failed: {made}");
                fs::rename(&fifo, &target).unwrap();
            }
        })
    };
    let (done, finished) = mpsc::channel();
    let observer = std::thread::spawn(move || {
        let started = Instant::now();
        let mut looks = 0_u64;
        while started.elapsed() < RACE {
            root(&repo, &allowlist).observe(&anchor);
            looks += 1;
        }
        let _ = done.send(looks);
    });
    let outcome = finished.recv_timeout(WATCHDOG);
    stop.store(true, Ordering::Relaxed);
    swapper.join().expect("swapper thread");
    match outcome {
        Ok(looks) => {
            observer.join().expect("observer thread");
            assert!(looks > 0, "the race ran no observation at all");
        }
        // The blocked observer is left behind: nothing will ever open the
        // FIFO it waits on, and the test process exits without it.
        Err(_) => panic!("an observation blocked on a FIFO swapped in under the anchor"),
    }
}

/// Anchors record the CANONICAL root, and a probe opened through another
/// spelling of the same checkout reports that same identity.
#[cfg(unix)]
#[test]
fn anchors_record_the_canonical_root_the_probe_reports() {
    let (dir, allowlist, repo) = checkout();
    let alias = dir.path().join("alias");
    std::os::unix::fs::symlink(&repo, &alias).unwrap();
    let set = root(&alias, &allowlist).anchors(&["src/lib.rs"]).unwrap();
    assert_eq!(set.root, repo.to_str().unwrap());
    assert_eq!(set.files.len(), 1);
    assert_eq!(root(&repo, &allowlist).root(), set.root);
}

/// N7: a root sent without any anchor is still validated, so a mistyped or
/// refused root is reported instead of silently ignored.
#[test]
fn a_root_without_anchors_is_still_validated() {
    let (_dir, allowlist, repo) = checkout();
    let outside = TempDir::new().unwrap();
    let outside = outside.path().to_str().unwrap();
    assert!(matches!(
        AnchorRoot::resolve_request(Some(outside), &[], &allowlist),
        Err(AnchorError::RootRefused { .. })
    ));
    assert_eq!(
        AnchorRoot::resolve_request(Some(repo.to_str().unwrap()), &[], &[]).unwrap_err(),
        AnchorError::Disabled
    );
    assert_eq!(
        AnchorRoot::resolve_request(Some(repo.to_str().unwrap()), &[], &allowlist).unwrap(),
        None
    );
    assert_eq!(
        AnchorRoot::resolve_request(None, &[], &[]).unwrap(),
        None,
        "neither a root nor an anchor is a plain remember"
    );
}

/// A FIFO left at an anchored path is unverifiable, and looking at it
/// returns at once.
#[cfg(unix)]
#[test]
fn a_fifo_at_an_anchored_path_is_unverifiable() {
    let (_dir, allowlist, repo) = checkout();
    let made = std::process::Command::new("mkfifo")
        .arg(repo.join("src/pipe"))
        .status()
        .expect("run mkfifo");
    assert!(made.success(), "mkfifo failed: {made}");
    let anchor = FileAnchor {
        path: path("src/pipe"),
        digest: ContentDigest::of(b""),
    };
    assert_eq!(
        root(&repo, &allowlist).observe(&anchor),
        AnchorObservation::Unverifiable
    );
}
