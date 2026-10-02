use super::STATE_FILE;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// The file that marks a migration in progress.
pub const LOCK_FILE: &str = "migration.lock";

/// The persistent sibling whose OS lock serializes every canonical lock check.
///
/// Unlike [`LOCK_FILE`], this file is never removed. Its inode must stay stable:
/// the advisory lock on its open handle closes the delete/recreate ABA window
/// around the human-readable canonical record.
pub(in crate::migration) const LOCK_GUARD_FILE: &str = "migration.lock.guard";

const LOCK_FORMAT_VERSION: u32 = 1;
static LOCK_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Exclusive possession of a migration workspace.
///
/// The persistent OS guard is held before the canonical record is inspected
/// and remains held until explicit release or drop. A canonical record is
/// deliberately retained after drop or panic as fail-closed evidence.
#[derive(Debug)]
pub struct MigrationLock {
    path: PathBuf,
    token: String,
    guard: std::fs::File,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct LockRecord {
    format_version: u32,
    held_by: String,
    token: String,
}

impl MigrationLock {
    /// Take the lock in `workspace` on behalf of `holder`.
    ///
    /// # Errors
    /// The OS guard is held, a canonical record remains, or the workspace is
    /// unwritable. Neither an active nor a dead lock is stolen automatically.
    pub fn acquire(workspace: &Path, holder: &str) -> Result<Self, String> {
        let path = workspace.join(LOCK_FILE);
        let guard = open_and_lock_guard(workspace)?;
        ensure_lock_record_absent(workspace, &path)?;
        let token = create_lock_record(&path, holder)?;
        Ok(Self { path, token, guard })
    }

    /// Who holds the lock in `workspace`, as recorded, or `None` when free.
    #[must_use]
    pub fn holder(workspace: &Path) -> Option<String> {
        std::fs::read_to_string(workspace.join(LOCK_FILE))
            .ok()
            .map(|body| {
                serde_json::from_str::<LockRecord>(&body).map_or_else(
                    |_| body.trim().to_owned(),
                    |record| format!("held_by={}", record.held_by),
                )
            })
    }

    pub(super) fn verify_workspace(&self, workspace: &Path) -> Result<(), String> {
        let expected = workspace.join(LOCK_FILE);
        if self.path != expected || !self.owns_current_lock() {
            return Err(format!(
                "cannot write {STATE_FILE} without the exact live migration lock identity for {}; acquire MigrationLock for this exact workspace first",
                workspace.display()
            ));
        }
        Ok(())
    }

    fn owns_current_lock(&self) -> bool {
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        // The lock record is created once via `create_new` (below) and, from
        // then on, only ever read (here, and by `holder` — #2424 tracks that
        // one still following a symlink) or removed by `release` — never
        // reopened to write into. A hard link planted before that creation
        // makes `create_new` itself fail (`EEXIST`), so by the time a
        // legitimate record exists, a later alias to it (a hard-link based
        // backup, `cp -al`) has nothing left to intercept: `nlink > 1` would
        // only refuse an untampered record and break lock checks on a
        // backed-up workspace (#2409 round 8). `_allow_hard_links` keeps the
        // symlink-swap refusal (#2404) this call still needs.
        let Ok((mut file, _metadata)) =
            crate::mutation::atomic_file::open_regular_file_allow_hard_links(
                &self.path,
                "migration lock",
                options,
            )
        else {
            return false;
        };
        let mut body = String::new();
        file.read_to_string(&mut body)
            .ok()
            .and_then(|_| serde_json::from_str::<LockRecord>(&body).ok())
            .is_some_and(|record| {
                record.format_version == LOCK_FORMAT_VERSION && record.token == self.token
            })
    }

    fn remove_if_owned(&self) -> Result<(), String> {
        if !self.owns_current_lock() {
            return Err(format!(
                "cannot release {LOCK_FILE}: the lock at {} is absent, invalid, or belongs to a later acquisition",
                self.path.display()
            ));
        }
        std::fs::remove_file(&self.path).map_err(|err| format!("cannot release {LOCK_FILE}: {err}"))
    }

    /// Release the lock.
    ///
    /// # Errors
    /// The canonical lock identity changed, the lock file cannot be removed,
    /// or the OS guard cannot be unlocked.
    pub fn release(self) -> Result<(), String> {
        self.remove_if_owned()?;
        fs2::FileExt::unlock(&self.guard).map_err(|err| {
            format!("removed {LOCK_FILE} but cannot unlock {LOCK_GUARD_FILE}: {err}")
        })
    }
}

fn open_and_lock_guard(workspace: &Path) -> Result<std::fs::File, String> {
    let guard_path = workspace.join(LOCK_GUARD_FILE);
    let guard = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&guard_path)
        .map_err(|err| format!("cannot open persistent {LOCK_GUARD_FILE}: {err}"))?;
    validate_guard_file(&guard_path, &guard)?;
    lock_guard(workspace, &guard)?;
    Ok(guard)
}

fn validate_guard_file(path: &Path, guard: &std::fs::File) -> Result<(), String> {
    let path_metadata = std::fs::symlink_metadata(path)
        .map_err(|err| format!("cannot inspect {LOCK_GUARD_FILE}: {err}"))?;
    let handle_is_file = guard.metadata().is_ok_and(|metadata| metadata.is_file());
    if !path_metadata.file_type().is_symlink() && handle_is_file {
        return Ok(());
    }
    Err(format!(
        "refusing {LOCK_GUARD_FILE} at {}: the persistent guard must be a regular, non-symlink file",
        path.display()
    ))
}

fn lock_guard(workspace: &Path, guard: &std::fs::File) -> Result<(), String> {
    match fs2::FileExt::try_lock_exclusive(guard) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => Err(format!(
            "a live migration still holds this workspace guard ({}). The OS guard is NOT stolen and deleting {LOCK_FILE} cannot release it; wait for the owner or stop it explicitly.",
            MigrationLock::holder(workspace).unwrap_or_else(|| "holder record missing".to_owned()),
        )),
        Err(err) => Err(format!("cannot lock {LOCK_GUARD_FILE}: {err}")),
    }
}

fn ensure_lock_record_absent(workspace: &Path, path: &Path) -> Result<(), String> {
    match std::fs::symlink_metadata(path) {
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(format!("cannot inspect {LOCK_FILE}: {err}")),
        Ok(_) => Err(format!(
            "a migration lock record remains in this workspace ({}). It is NOT stolen automatically: a dead process releases the OS guard but leaves this evidence behind. If you are certain no migration is running, delete {} yourself.",
            MigrationLock::holder(workspace).unwrap_or_else(|| "holder unknown".to_owned()),
            path.display()
        )),
    }
}

fn create_lock_record(path: &Path, holder: &str) -> Result<String, String> {
    let token = next_lock_token();
    let record = LockRecord {
        format_version: LOCK_FORMAT_VERSION,
        held_by: holder.to_owned(),
        token: token.clone(),
    };
    let body = serde_json::to_vec_pretty(&record)
        .map_err(|err| format!("cannot serialise {LOCK_FILE}: {err}"))?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|err| format!("cannot create {LOCK_FILE}: {err}"))?;
    file.write_all(&body)
        .map_err(|err| format!("cannot write {LOCK_FILE}: {err}"))?;
    file.flush()
        .and_then(|()| file.sync_all())
        .map_err(|err| format!("cannot persist {LOCK_FILE}: {err}"))?;
    Ok(token)
}

fn next_lock_token() -> String {
    let sequence = LOCK_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    format!(
        "lock-v{LOCK_FORMAT_VERSION}-{:08x}-{nanos:032x}-{sequence:016x}",
        std::process::id()
    )
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use std::sync::atomic::AtomicUsize;

    /// `migration::tests::state::a_lock_file_swapped_for_a_symlink_is_refused_not_followed`
    /// only swaps the symlink in BEFORE calling `release` — the pre-fix
    /// `symlink_metadata`-then-reopen-by-name shape refuses that static
    /// case too, since its own check already sees the symlink; it never
    /// exercises the WINDOW between a check and a later reopen by name,
    /// which only a continuously racing swap can reach. `owns_current_lock`
    /// is private, so this lives here rather than in
    /// `migration::tests::state`, which cannot reach it (module privacy,
    /// not an oversight).
    ///
    /// The genuine on-disk record always carries a token that does NOT
    /// match this lock's own, so `owns_current_lock` must return `false`
    /// for it regardless of race timing; a `true` result can then only be
    /// explained by having read through the symlink to the planted
    /// secret, whose token does match. Proven by mutation: reverting
    /// `owns_current_lock` to a `symlink_metadata`-then-reopen-by-name
    /// shape makes this fail (observed: thousands of `true` results
    /// across 20,000 iterations; 0 on the fixed code).
    #[test]
    fn owns_current_lock_is_never_trusted_through_a_racing_symlink_swap() {
        const ITERATIONS: usize = 20_000;

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
}
