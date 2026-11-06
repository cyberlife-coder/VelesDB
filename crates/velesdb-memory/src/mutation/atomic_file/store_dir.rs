//! A store's workspace directory, pinned by an open descriptor so every
//! later file operation resolves a bare name against the directory *inode*
//! the store opened once — never the path again (#2408).
//!
//! On unix [`StoreDir::open`] keeps the directory itself open
//! (`O_NOFOLLOW`, refused when it is not a real directory) and every method
//! resolves `name` through the `*at` family against that descriptor
//! (`openat`, `renameat`, `unlinkat`, `fstatat`, `Dir`) — `rustix` safe
//! wrappers, so this crate's `deny(unsafe_code)` holds: a workspace renamed
//! away or swapped for a symlink afterwards cannot be followed, because the
//! descriptor pins the inode. `path` is retained for error messages and
//! path-equality checks only — never for resolution.
//!
//! Non-unix platforms expose no `openat` through `std` (Windows
//! handle-relative opens need raw `NtCreateFile`), so each method falls back
//! to joining `path` and the swap-after-open window remains open there —
//! named here rather than hidden. `StoreDir` is still the right boundary:
//! the sites that stay racy are exactly these methods.

use std::fs::File;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use super::not_a_regular_file;
#[cfg(not(unix))]
use super::{
    open_regular_file, open_regular_file_allow_hard_links, path_exists, promote, sync_directory,
    validate_regular_file, validate_workspace,
};
#[cfg(unix)]
use crate::migration::capture;
use crate::MemoryError;
#[cfg(unix)]
use rustix::fs::{AtFlags, FileType, Mode, OFlags};
#[cfg(unix)]
use rustix::io::Errno;

/// The flags every `openat` in this module carries on top of the caller's
/// access mode: refuse a symlinked final component, never block on a FIFO,
/// and close on exec.
#[cfg(unix)]
const OPENAT_FLAGS: OFlags = OFlags::NOFOLLOW
    .union(OFlags::NONBLOCK)
    .union(OFlags::CLOEXEC);

/// A workspace directory a store holds open for its whole lifetime. See the
/// module doc for the threat model and the non-unix fallback's residual.
#[derive(Debug)]
pub(crate) struct StoreDir {
    path: PathBuf,
    #[cfg(unix)]
    dir: File,
}

impl StoreDir {
    /// The path the workspace was opened at — for messages and equality
    /// checks only; resolution never goes through it.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// Opens `path` read-only with `O_NOFOLLOW` (a symlinked workspace is
    /// refused at the open itself, like `open_checked`'s final-component
    /// rule) and keeps the descriptor. `O_NONBLOCK` so a planted FIFO
    /// refuses instead of blocking the open.
    #[cfg(unix)]
    pub(crate) fn open(path: &Path, entity: &str) -> Result<Self, MemoryError> {
        let dir = rustix::fs::open(path, OFlags::RDONLY.union(OPENAT_FLAGS), Mode::empty())
            .map(File::from)
            .map_err(|err| open_dir_error(err, path, entity))?;
        if !dir
            .metadata()
            .map_err(|err| capture(format!("cannot inspect {entity} workspace: {err}")))?
            .is_dir()
        {
            return Err(capture(format!(
                "{entity} workspace must be a real directory"
            )));
        }
        Ok(Self {
            path: path.to_owned(),
            dir,
        })
    }

    #[cfg(not(unix))]
    pub(crate) fn open(path: &Path, entity: &str) -> Result<Self, MemoryError> {
        validate_workspace(path, entity)?;
        Ok(Self {
            path: path.to_owned(),
        })
    }

    /// Opens `name` read-write inside this directory, refusing a symlink or
    /// any non-regular type — and a hard link (`nlink > 1`), the same rule
    /// `open_regular_file` applies to a file this store reopens to write
    /// into (the journal's load path is the only caller).
    #[cfg(unix)]
    pub(crate) fn open_regular_file(&self, name: &str, entity: &str) -> Result<File, MemoryError> {
        use std::os::unix::fs::MetadataExt;
        let (file, metadata) = self.open_at(name, entity, OFlags::RDWR)?;
        if metadata.nlink() > 1 {
            return Err(not_a_regular_file(entity));
        }
        Ok(file)
    }

    /// Opens `name` read-only inside this directory, refusing a symlink or
    /// any non-regular type but tolerating a hard link — the
    /// dirfd-relative analogue of `open_regular_file_allow_hard_links`,
    /// for records the store only ever replaces wholesale.
    #[cfg(unix)]
    pub(crate) fn open_regular_file_allow_hard_links(
        &self,
        name: &str,
        entity: &str,
    ) -> Result<(File, std::fs::Metadata), MemoryError> {
        self.open_at(name, entity, OFlags::RDONLY)
    }

    #[cfg(not(unix))]
    pub(crate) fn open_regular_file(&self, name: &str, entity: &str) -> Result<File, MemoryError> {
        let mut options = std::fs::OpenOptions::new();
        options.read(true).write(true);
        open_regular_file(&self.path.join(name), entity, options)
    }

    #[cfg(not(unix))]
    pub(crate) fn open_regular_file_allow_hard_links(
        &self,
        name: &str,
        entity: &str,
    ) -> Result<(File, std::fs::Metadata), MemoryError> {
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        open_regular_file_allow_hard_links(&self.path.join(name), entity, options)
    }

    /// Creates `name` fresh inside this directory (`O_EXCL` — an existing
    /// entry of any kind is refused) and returns the read-write handle.
    /// `mode` is the unix permission bits, as `std`'s `0o666` default; on
    /// non-unix it is ignored.
    #[cfg(unix)]
    pub(crate) fn create_new(&self, name: &str, mode: libc::mode_t) -> std::io::Result<File> {
        rustix::fs::openat(
            &self.dir,
            name,
            OFlags::RDWR
                .union(OFlags::CREATE)
                .union(OFlags::EXCL)
                .union(OPENAT_FLAGS),
            Mode::from_bits_truncate(mode),
        )
        .map(File::from)
        .map_err(std::io::Error::from)
    }

    #[cfg(not(unix))]
    pub(crate) fn create_new(&self, name: &str, _mode: u32) -> std::io::Result<File> {
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(self.path.join(name))
    }

    /// Renames `staging` to `final_name`, both inside this directory —
    /// `renameat` against the pinned descriptor on unix.
    #[cfg(unix)]
    pub(crate) fn promote(&self, staging: &str, final_name: &str) -> std::io::Result<()> {
        rustix::fs::renameat(&self.dir, staging, &self.dir, final_name)
            .map_err(std::io::Error::from)
    }

    #[cfg(not(unix))]
    pub(crate) fn promote(&self, staging: &str, final_name: &str) -> std::io::Result<()> {
        promote(&self.path.join(staging), &self.path.join(final_name))
    }

    /// Removes `name` inside this directory (`unlinkat` on unix).
    #[cfg(unix)]
    pub(crate) fn remove(&self, name: &str) -> std::io::Result<()> {
        rustix::fs::unlinkat(&self.dir, name, AtFlags::empty()).map_err(std::io::Error::from)
    }

    #[cfg(not(unix))]
    pub(crate) fn remove(&self, name: &str) -> std::io::Result<()> {
        std::fs::remove_file(self.path.join(name))
    }

    /// Whether `name` exists inside this directory — `fstatat` with
    /// `AT_SYMLINK_NOFOLLOW`, so a dangling link still counts, like
    /// `path_exists`.
    #[cfg(unix)]
    pub(crate) fn exists(&self, name: &str) -> Result<bool, MemoryError> {
        match lstat_at(&self.dir, name) {
            Ok(_) => Ok(true),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(err) => Err(capture(format!(
                "cannot inspect {}: {err}",
                self.path.join(name).display()
            ))),
        }
    }

    #[cfg(not(unix))]
    pub(crate) fn exists(&self, name: &str) -> Result<bool, MemoryError> {
        path_exists(&self.path.join(name))
    }

    /// Refuses `name` unless it is a lone regular file, without following a
    /// symlink — `validate_regular_file` resolved through the descriptor.
    #[cfg(unix)]
    pub(crate) fn validate_regular(&self, name: &str, entity: &str) -> Result<(), MemoryError> {
        let stat = lstat_at(&self.dir, name)
            .map_err(|err| capture(format!("cannot inspect {entity} file: {err}")))?;
        if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile {
            return Err(capture(format!("{entity} path must be a regular file")));
        }
        Ok(())
    }

    #[cfg(not(unix))]
    pub(crate) fn validate_regular(&self, name: &str, entity: &str) -> Result<(), MemoryError> {
        validate_regular_file(&self.path.join(name), entity)
    }

    /// Durability-barriers the directory — `fsync` on the pinned descriptor
    /// itself, so nothing reopens the workspace by path.
    #[cfg(unix)]
    pub(crate) fn sync(&self) -> std::io::Result<()> {
        self.dir.sync_all()
    }

    #[cfg(not(unix))]
    pub(crate) fn sync(&self) -> std::io::Result<()> {
        sync_directory(&self.path)
    }

    /// Names present in the pinned directory — `Dir::read_from` on a clone
    /// of the descriptor, so a swapped-in directory is never listed. Only the
    /// extraction job store's `pending()` scan lists a directory today.
    #[cfg(all(unix, any(feature = "mcp", test)))]
    pub(crate) fn entries(&self) -> std::io::Result<Vec<String>> {
        let cloned = self.dir.try_clone()?;
        let mut dir = rustix::fs::Dir::read_from(cloned)?;
        let mut names = Vec::new();
        while let Some(entry) = dir.read() {
            let name = entry?.file_name().to_bytes().to_vec();
            if name == b"." || name == b".." {
                continue;
            }
            names.push(String::from_utf8_lossy(&name).into_owned());
        }
        Ok(names)
    }

    #[cfg(all(not(unix), any(feature = "mcp", test)))]
    pub(crate) fn entries(&self) -> std::io::Result<Vec<String>> {
        std::fs::read_dir(&self.path)?
            .map(|entry| entry.map(|e| e.file_name().to_string_lossy().into_owned()))
            .collect()
    }

    /// Opens `name` relative to the pinned descriptor with `access` plus
    /// [`OPENAT_FLAGS`], then checks the *handle* — the same
    /// open-then-`fstat` shape as `open_checked`, resolved through the
    /// descriptor so the workspace path is never consulted.
    #[cfg(unix)]
    fn open_at(
        &self,
        name: &str,
        entity: &str,
        access: OFlags,
    ) -> Result<(File, std::fs::Metadata), MemoryError> {
        let file = rustix::fs::openat(&self.dir, name, access.union(OPENAT_FLAGS), Mode::empty())
            .map(File::from)
            .map_err(|err| open_file_error(err, &self.dir, &self.path, name, entity))?;
        let metadata = file.metadata().map_err(|err| {
            capture(format!(
                "cannot inspect {entity} file {}: {err}",
                self.path.join(name).display()
            ))
        })?;
        if !metadata.is_file() {
            return Err(not_a_regular_file(entity));
        }
        Ok((file, metadata))
    }
}

/// Message selection for a failed `StoreDir::open`, mirroring `open_error`:
/// the open already refused, so the fallback `lstat` only chooses the
/// message — `ELOOP` for a symlinked workspace, a non-directory entry for
/// other wrong kinds, the raw error otherwise.
#[cfg(unix)]
fn open_dir_error(err: Errno, path: &Path, entity: &str) -> MemoryError {
    let wrong_kind = err == Errno::LOOP
        || std::fs::symlink_metadata(path).is_ok_and(|metadata| !metadata.is_dir());
    if wrong_kind {
        capture(format!("{entity} workspace must be a real directory"))
    } else {
        capture(format!("cannot inspect {entity} workspace: {err}"))
    }
}

/// Message selection for a failed `openat`, mirroring `open_error`: the open
/// already refused the path, so the `fstatat` below only chooses which
/// refusal to report — nothing security-relevant is decided here.
#[cfg(unix)]
fn open_file_error(
    err: Errno,
    dir: &File,
    dir_path: &Path,
    name: &str,
    entity: &str,
) -> MemoryError {
    if err == Errno::LOOP {
        return not_a_regular_file(entity);
    }
    if lstat_at(dir, name)
        .is_ok_and(|stat| FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile)
    {
        not_a_regular_file(entity)
    } else {
        capture(format!(
            "cannot open {entity} file {}: {err}",
            dir_path.join(name).display()
        ))
    }
}

/// `fstatat(AT_SYMLINK_NOFOLLOW)` against `dir`: the pinned-directory
/// analogue of `symlink_metadata`.
#[cfg(unix)]
fn lstat_at(dir: &File, name: &str) -> std::io::Result<rustix::fs::Stat> {
    rustix::fs::statat(dir, name, AtFlags::SYMLINK_NOFOLLOW).map_err(std::io::Error::from)
}
