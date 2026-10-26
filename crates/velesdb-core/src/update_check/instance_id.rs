//! Install identity.
//!
//! One random id per installation, generated once and kept in a per-user
//! configuration directory. Nothing about the machine, the user or the data
//! directory goes into it, so it identifies an install and nothing else.

use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Versioned prefix hashed in front of the id, so a future scheme cannot
/// collide with this one.
const HASH_PREFIX: &[u8] = b"velesdb-install-v2:";

/// Name of the file holding the id, in each candidate directory.
const FILE_NAME: &str = "install-id";

/// Operating-system families that place per-user configuration differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Platform {
    Linux,
    MacOs,
    Windows,
}

impl Platform {
    pub(super) const CURRENT: Self = if cfg!(target_os = "windows") {
        Self::Windows
    } else if cfg!(target_os = "macos") {
        Self::MacOs
    } else {
        Self::Linux
    };
}

/// Per-user configuration directory of `platform`, `velesdb` subdirectory
/// included. `lookup` reads one environment variable. `XDG_CONFIG_HOME` is
/// honoured only when absolute, as the XDG specification requires.
pub(super) fn user_config_dir(
    platform: Platform,
    lookup: impl Fn(&str) -> Option<OsString>,
) -> Option<PathBuf> {
    let base = match platform {
        Platform::Linux => lookup("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| non_empty(lookup("HOME")).map(|home| home.join(".config"))),
        Platform::MacOs => {
            non_empty(lookup("HOME")).map(|home| home.join("Library").join("Application Support"))
        }
        Platform::Windows => non_empty(lookup("APPDATA")),
    };
    base.map(|dir| dir.join("velesdb"))
}

fn non_empty(value: Option<OsString>) -> Option<PathBuf> {
    value.filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// Candidate directories for the id file, in order of preference.
pub(super) fn candidate_dirs(
    platform: Platform,
    lookup: impl Fn(&str) -> Option<OsString>,
    data_dir: Option<&Path>,
) -> Vec<PathBuf> {
    user_config_dir(platform, lookup)
        .into_iter()
        .chain(data_dir.map(Path::to_path_buf))
        .collect()
}

/// A fresh random id: 128 bits, 32 lowercase hex digits.
fn new_id() -> String {
    format!("{:032x}", rand::random::<u128>())
}

fn is_valid_id(candidate: &str) -> bool {
    candidate.len() == 32
        && candidate
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn read_id(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let id = content.trim();
    is_valid_id(id).then(|| id.to_string())
}

/// How long a start waits for another process that is writing the id file,
/// and how often it looks.
const LOCK_WAIT: Duration = Duration::from_secs(2);
const LOCK_POLL: Duration = Duration::from_millis(5);

/// The file locked while the id file is written. It stays on disk, empty:
/// deleting a lock file other processes may be waiting on lets two of them
/// hold "the" lock at once.
const LOCK_NAME: &str = ".install-id.lock";

/// The id stored in `dir`, generated and written there when absent or
/// unreadable as an id. `None` when `dir` cannot hold it.
///
/// A start that finds the file readable returns at once. Otherwise it takes
/// an exclusive advisory lock on `.install-id.lock`, reads the file again,
/// and only if it is still missing or damaged writes a complete, synced
/// temporary file and renames it into place. The kernel drops the lock when
/// its holder exits or dies, so there is nothing to time out, take over or
/// clean up, and concurrent first starts all return the one id that ended up
/// in the file. When the lock cannot be had (another process holds it for
/// longer than [`LOCK_WAIT`], or the filesystem does not support locking)
/// the directory counts as unusable and the next candidate is tried: at
/// worst that start reports an id of its own for one run. On a filesystem
/// without locking (a network home, some FUSE mounts) the next candidate is
/// tried, and a start gets an ephemeral id, counting as a new install, only
/// when no candidate directory supports locking (always the case for the CLI,
/// which has no data directory, if its config directory cannot lock).
pub(super) fn load_or_create(dir: &Path) -> Option<String> {
    load_or_create_within(dir, LOCK_WAIT)
}

fn load_or_create_within(dir: &Path, wait: Duration) -> Option<String> {
    let path = dir.join(FILE_NAME);
    if let Some(id) = read_id(&path) {
        return Some(id);
    }
    std::fs::create_dir_all(dir).ok()?;
    let _lock = lock_exclusive(&dir.join(LOCK_NAME), wait)?;
    read_id(&path).or_else(|| write_new(dir, &path))
}

/// What a start does after one attempt to take the lock.
#[derive(Debug, PartialEq, Eq)]
enum LockStep {
    /// The lock is ours.
    Acquired,
    /// Held by someone else and still within the wait: sleep and try again.
    Retry,
    /// Held for longer than the wait, or the filesystem cannot lock at all:
    /// this directory is unusable.
    GiveUp,
}

/// Maps the outcome of [`std::fs::File::try_lock`] to the next step. A lock
/// error that is not contention (`TryLockError::Error`, for instance a
/// filesystem without locking) is never retried and never ignored: writing
/// without the lock would let concurrent first starts disagree.
fn lock_step(
    attempt: &Result<(), std::fs::TryLockError>,
    waited: Duration,
    wait: Duration,
) -> LockStep {
    match attempt {
        Ok(()) => LockStep::Acquired,
        Err(std::fs::TryLockError::WouldBlock) if waited < wait => LockStep::Retry,
        Err(_) => LockStep::GiveUp,
    }
}

/// Opens `path` and takes an exclusive advisory lock on it, polling for at
/// most `wait`. The lock lasts as long as the returned file is open.
fn lock_exclusive(path: &Path, wait: Duration) -> Option<std::fs::File> {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .ok()?;
    let started = std::time::Instant::now();
    loop {
        match lock_step(&file.try_lock(), started.elapsed(), wait) {
            LockStep::Acquired => return Some(file),
            LockStep::Retry => std::thread::sleep(LOCK_POLL),
            LockStep::GiveUp => return None,
        }
    }
}

/// Writes a fresh id to a temporary file, flushes it to disk, and renames it
/// over `path`, so a reader sees the old file or the complete new one.
fn write_new(dir: &Path, path: &Path) -> Option<String> {
    let id = new_id();
    let temporary = dir.join(format!(".{FILE_NAME}.{id}.tmp"));
    write_synced(&temporary, &format!("{id}\n")).ok()?;
    if std::fs::rename(&temporary, path).is_err() {
        let _ = std::fs::remove_file(&temporary);
        return None;
    }
    Some(id)
}

fn write_synced(path: &Path, content: &str) -> std::io::Result<()> {
    let mut file = std::fs::File::create(path)?;
    file.write_all(content.as_bytes())?;
    file.sync_all()
}

/// The id of the first directory that can hold one, else a fresh id that
/// lives for this process only.
pub(super) fn resolve_install_id(dirs: &[PathBuf]) -> String {
    dirs.iter()
        .find_map(|dir| load_or_create(dir))
        .unwrap_or_else(new_id)
}

/// The stored id when a directory holds one, else a fresh id; nothing is
/// ever written.
pub(super) fn peek_install_id(dirs: &[PathBuf]) -> String {
    dirs.iter()
        .find_map(|dir| read_id(&dir.join(FILE_NAME)))
        .unwrap_or_else(new_id)
}

/// SHA-256 hex digest sent to the server in place of the id itself.
pub(super) fn hash_install_id(id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(HASH_PREFIX);
    hasher.update(id.as_bytes());
    hex::encode(hasher.finalize())
}

/// Hash identifying this installation, resolving its id on the real
/// filesystem and environment.
pub(super) fn install_hash(data_dir: Option<&Path>) -> String {
    let dirs = candidate_dirs(Platform::CURRENT, |key| std::env::var_os(key), data_dir);
    hash_install_id(&resolve_install_id(&dirs))
}

/// Compute the hash identifying this installation.
///
/// Deprecated: the hash no longer depends on the machine or on `data_dir`.
/// It is derived from a random id the update check stores once per user, and
/// `data_dir` is only a fallback place to look for it. This function reads
/// that id and **never creates it** — an install that opted out of the
/// update check must not gain a file by calling it — so with no stored id it
/// returns the hash of a fresh random id that is not kept, so two calls
/// then return two different values.
#[deprecated(
    since = "7.1.0",
    note = "the update check derives its install hash itself; this reads the stored id, never creates one (a fresh random hash on every call when none is stored), and is no longer machine- or data-dir-bound"
)]
#[must_use]
pub fn compute_instance_hash(data_dir: &Path) -> String {
    stored_install_hash(Platform::CURRENT, |key| std::env::var_os(key), data_dir)
}

/// [`compute_instance_hash`] with the platform and the environment given, so
/// a test can point every candidate directory at a temporary one.
fn stored_install_hash(
    platform: Platform,
    lookup: impl Fn(&str) -> Option<OsString>,
    data_dir: &Path,
) -> String {
    let dirs = candidate_dirs(platform, lookup, Some(data_dir));
    hash_install_id(&peek_install_id(&dirs))
}

#[cfg(test)]
#[path = "instance_id_tests.rs"]
mod tests;
