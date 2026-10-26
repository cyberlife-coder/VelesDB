//! Install identity.
//!
//! One random id per installation, generated once and kept in a per-user
//! configuration directory. Nothing about the machine, the user or the data
//! directory goes into it, so it identifies an install and nothing else.

use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::io::{ErrorKind, Write};
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

/// How many times, and how far apart, an id file that exists but does not
/// read as an id is looked at again before it is called corrupt. Files this
/// module creates are complete the moment they appear; the wait is for a
/// writer that is not this module.
const REREAD_ATTEMPTS: u32 = 3;
const REREAD_DELAY: Duration = Duration::from_millis(5);

/// How long a racer waits for the holder of the lock before concluding the
/// holder died while replacing a corrupt id file, and taking the lock over.
const LOCK_PATIENCE: Duration = Duration::from_secs(30);

/// Name of the lock file taken while a corrupt id file is replaced.
const LOCK_NAME: &str = ".install-id.lock";

/// The id stored in `dir`, generated and written there when absent or
/// unreadable as an id. `None` when `dir` cannot hold it.
///
/// The id is written to a private temporary file, flushed to disk, and
/// hard-linked into place: the link fails if the file already exists, so of
/// several first starts racing for one directory exactly one wins and the
/// others read its id, and no reader ever sees a half-written file. A file
/// that is still not an id after a short wait is corrupt; replacing it is
/// done under a lock so that racers still agree on one id (see
/// [`replace_corrupt`]). Needs a filesystem that supports hard links;
/// elsewhere the directory counts as unusable and the next candidate is
/// tried.
pub(super) fn load_or_create(dir: &Path) -> Option<String> {
    let path = dir.join(FILE_NAME);
    if let Some(id) = read_id(&path) {
        return Some(id);
    }
    std::fs::create_dir_all(dir).ok()?;
    let id = new_id();
    let temporary = dir.join(format!(".{FILE_NAME}.{id}.tmp"));
    write_synced(&temporary, &format!("{id}\n")).ok()?;
    let stored = publish(&temporary, &path, &id);
    let _ = std::fs::remove_file(&temporary);
    stored
}

fn write_synced(path: &Path, content: &str) -> std::io::Result<()> {
    let mut file = std::fs::File::create(path)?;
    file.write_all(content.as_bytes())?;
    file.sync_all()
}

/// Puts the complete file `temporary` at `path` without replacing an id that
/// is already there; returns the id `path` ends up holding.
fn publish(temporary: &Path, path: &Path, id: &str) -> Option<String> {
    match std::fs::hard_link(temporary, path) {
        Ok(()) => Some(id.to_string()),
        Err(e) if e.kind() == ErrorKind::AlreadyExists => {
            for _ in 0..REREAD_ATTEMPTS {
                if let Some(existing) = read_id(path) {
                    return Some(existing);
                }
                std::thread::sleep(REREAD_DELAY);
            }
            replace_corrupt(temporary, path, id, LOCK_PATIENCE)
        }
        Err(_) => None,
    }
}

/// Replaces an id file that stayed unreadable, under a lock: the holder
/// re-checks the file, and only if it is still corrupt renames its complete
/// temporary file over it. Everyone else waits for the holder and returns
/// what the file then holds, so a crash that left a damaged file cannot make
/// concurrent starts disagree. The lock is taken with `create_new`, so one
/// racer holds it; one that outlasts `patience` is taken over.
fn replace_corrupt(temporary: &Path, path: &Path, id: &str, patience: Duration) -> Option<String> {
    let lock = path.with_file_name(LOCK_NAME);
    for _ in 0..2 {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock)
        {
            Ok(_) => {
                let stored = read_id(path).or_else(|| {
                    std::fs::rename(temporary, path).ok()?;
                    Some(id.to_string())
                });
                let _ = std::fs::remove_file(&lock);
                return stored;
            }
            Err(e) if e.kind() == ErrorKind::AlreadyExists => {
                if let Some(existing) = wait_for_holder(path, &lock, patience) {
                    return Some(existing);
                }
                let _ = std::fs::remove_file(&lock); // stale: its holder is gone
            }
            Err(_) => return None,
        }
    }
    None
}

/// Waits until the lock holder has stored an id, the lock disappears, or
/// `patience` runs out.
fn wait_for_holder(path: &Path, lock: &Path, patience: Duration) -> Option<String> {
    let started = std::time::Instant::now();
    while started.elapsed() < patience && lock.exists() {
        if let Some(existing) = read_id(path) {
            return Some(existing);
        }
        std::thread::sleep(REREAD_DELAY);
    }
    read_id(path)
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
    since = "6.1.0",
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
