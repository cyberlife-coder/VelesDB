//! A genuine racing proof for `owns_current_lock`, kept out of `lock.rs`
//! itself (the repository's inline-test-module budget forbids growing it
//! there) but still a child module of it, so it keeps access to the
//! private items this proof needs.

use super::*;
use std::os::unix::fs::symlink;
use std::sync::atomic::AtomicUsize;

/// `migration::tests::state::a_lock_file_swapped_for_a_symlink_is_refused_not_followed`
/// only swaps the symlink in BEFORE calling `release` — the pre-fix
/// `symlink_metadata`-then-reopen-by-name shape refuses that static case
/// too, since its own check already sees the symlink; it never
/// exercises the WINDOW between a check and a later reopen by name,
/// which only a continuously racing swap can reach. `owns_current_lock`
/// is private, so this lives here rather than in
/// `migration::tests::state`, which cannot reach it (module privacy,
/// not an oversight).
///
/// The genuine on-disk record always carries a token that does NOT
/// match this lock's own, so `owns_current_lock` must return `false`
/// for it regardless of race timing; a `true` result can then only be
/// explained by having read through the symlink to the planted secret,
/// whose token does match. Round 23 proved this against that exact
/// pre-fix shape at `ITERATIONS = 20_000`, but round 24 found the
/// proof unreliable at that size: 13 repeats gave `true`-counts of 1,
/// 0, 1, 2, 0, 1, 0, 1, 0, 1, 0, 1, 2 — 5 of 13 a false negative. The
/// window is real but narrow. Raised to `200_000`: 10 repeats against
/// the same pre-fix shape all failed (1 to 9 `true` results each), and
/// the fixed code passed cleanly 5 times in a row at this size.
#[test]
fn owns_current_lock_is_never_trusted_through_a_racing_symlink_swap() {
    const ITERATIONS: usize = 200_000;

    let workspace = tempfile::tempdir().expect("tempdir");
    let outside = tempfile::tempdir().expect("outside tempdir");

    let lock = MigrationLock::acquire(workspace.path(), "run-A").expect("acquire");
    let lock_path = workspace.path().join(LOCK_FILE);

    let mismatched = LockRecord {
        format_version: LOCK_FORMAT_VERSION,
        held_by: "run-A".to_owned(),
        token: "mismatched-token".to_owned(),
    };
    let mismatched_body = serde_json::to_vec(&mismatched).expect("serialize mismatched");

    let secret = LockRecord {
        format_version: LOCK_FORMAT_VERSION,
        held_by: "attacker".to_owned(),
        token: lock.token.clone(),
    };
    let secret_path = outside.path().join("secret.lock");
    std::fs::write(
        &secret_path,
        serde_json::to_vec(&secret).expect("serialize secret"),
    )
    .expect("plant secret");

    // `acquire` above left the GENUINE record (matching `lock.token`) at
    // `lock_path`; overwrite it with the mismatched one before the race
    // starts, so every later `true` is attributable to the race loop
    // below rather than to this one-time setup state.
    std::fs::write(&lock_path, &mismatched_body).expect("seed mismatched");

    let trusted = AtomicUsize::new(0);

    std::thread::scope(|scope| {
        scope.spawn(|| {
            for _ in 0..ITERATIONS {
                let _ = std::fs::remove_file(&lock_path);
                symlink(&secret_path, &lock_path).expect("plant symlink");
                let _ = std::fs::remove_file(&lock_path);
                std::fs::write(&lock_path, &mismatched_body).expect("restore mismatched");
            }
        });

        for _ in 0..ITERATIONS {
            if lock.owns_current_lock() {
                trusted.fetch_add(1, Ordering::Relaxed);
            }
        }
    });

    assert_eq!(
        trusted.load(Ordering::Relaxed),
        0,
        "a racing symlink swap must never be trusted as this lock's own record"
    );
}
