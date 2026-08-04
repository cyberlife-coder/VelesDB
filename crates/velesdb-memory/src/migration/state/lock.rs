use super::STATE_FILE;
use std::io::Write;
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
        let is_live_regular_file = std::fs::symlink_metadata(&self.path)
            .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink());
        if !is_live_regular_file {
            return false;
        }
        std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|body| serde_json::from_str::<LockRecord>(&body).ok())
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
