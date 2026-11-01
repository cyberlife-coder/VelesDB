//! [`AnchorRoot`]: the one production [`AnchorProbe`] — reads anchored files
//! below a checkout root, and nowhere else.
//!
//! # Security model
//!
//! The server reads a file only when every step below holds, in order:
//!
//! 1. at least one allowlisted root is configured (the ingest roots,
//!    `VELESDB_MEMORY_INGEST_ROOTS`); otherwise anchoring is
//!    [`AnchorError::Disabled`];
//! 2. the checkout root is absolute, resolves (every symlink followed by
//!    [`std::fs::canonicalize`]) to a directory, and that directory lies
//!    inside an allowlisted root, compared component-wise;
//! 3. the anchored path is a [`RepoPath`] — no root, `.` or `..` component —
//!    so joining it can only name something below the checkout root;
//! 4. the joined path, canonicalized again, still lies below the canonical
//!    checkout root: a symlink inside the checkout that points out of it is
//!    refused, not followed;
//! 5. the OPENED HANDLE is a regular file of at most
//!    [`crate::limits::MAX_ANCHOR_FILE_BYTES`], and the read itself is
//!    bounded to that size plus one byte, so a file growing between the
//!    `stat` and the read is refused rather than read whole.
//!
//! Errors and refusals cite the path as requested, never a resolved target.
//! A refusal at recall time makes the anchor [`AnchorObservation::Unverifiable`],
//! never stale: being unable to look is not evidence.
//!
//! # Cost
//!
//! One root serves one request. Each distinct file is read at most once per
//! request (memoized), at most [`crate::limits::MAX_ANCHOR_FILES_PER_RECALL`]
//! files and [`crate::limits::MAX_ANCHOR_BYTES_PER_RECALL`] bytes in total;
//! past either budget the remaining anchors are unverifiable.
//!
//! The open itself is hardened against swaps between canonicalization and
//! open, asymmetrically by platform. On unix it carries `O_NONBLOCK` and
//! `O_NOFOLLOW`, so a FIFO swapped in under an anchor reports unverifiable
//! instead of parking the recall (and, behind a queued writer, the daemon)
//! on a writer that never comes, and a final-component symlink swapped in
//! is refused rather than followed. `O_NOFOLLOW` guards the FINAL component
//! only: an intermediate directory component replaced by a symlink in the
//! gap still redirects the open, as does a regular file swapped for
//! another — and on non-unix builds, where [`open_for_anchor`] is a plain
//! `File::open`, the final-component swap is followed too and a swapped-in
//! FIFO can block. Those residual windows are accepted, as for the ingest
//! pipeline: a local single-user server, and a verdict about the bytes
//! actually read.

use std::collections::HashMap;
use std::fs;
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};

use super::{
    check_anchor_count, AnchorError, AnchorObservation, AnchorProbe, AnchorSet, ContentDigest,
    FileAnchor, RepoPath,
};
use crate::limits::{
    MAX_ANCHOR_BYTES_PER_RECALL, MAX_ANCHOR_FILES_PER_RECALL, MAX_ANCHOR_FILE_BYTES,
};

/// What one look at an anchored path found.
#[derive(Debug, Clone)]
enum Look {
    /// The file's current digest.
    Digest(ContentDigest),
    /// Nothing at that path.
    Missing,
    /// The file may not or could not be read, and why.
    Refused(&'static str),
}

/// A checkout root inside the allowlist, with this request's read budget.
#[derive(Debug)]
pub struct AnchorRoot {
    root: String,
    bytes_left: usize,
    files_left: usize,
    seen: HashMap<RepoPath, Look>,
}

impl AnchorRoot {
    /// Admit `requested` as a checkout root if it lies inside one of the
    /// already-canonical `allowlist` roots.
    ///
    /// # Errors
    /// [`AnchorError::Disabled`] when `allowlist` is empty,
    /// [`AnchorError::NonUtf8Root`] when the canonical root is not UTF-8 —
    /// it could not be recorded with the anchors — and
    /// [`AnchorError::RootRefused`] for a relative, unresolvable,
    /// non-directory or out-of-allowlist root.
    pub fn open(requested: &str, allowlist: &[PathBuf]) -> Result<Self, AnchorError> {
        if allowlist.is_empty() {
            return Err(AnchorError::Disabled);
        }
        let refuse = |reason| AnchorError::RootRefused {
            root: requested.to_owned(),
            reason,
        };
        let path = Path::new(requested);
        if !path.is_absolute() {
            return Err(refuse("only an absolute root is accepted"));
        }
        let root = fs::canonicalize(path).map_err(|_| refuse("it cannot be resolved"))?;
        if !root.is_dir() {
            return Err(refuse("it is not a directory"));
        }
        if !allowlist.iter().any(|allowed| root.starts_with(allowed)) {
            return Err(refuse("it is outside the configured ingest roots"));
        }
        // The canonical root is recorded with the anchors and later compared
        // to the probe's name for the checkout — as a string. A root that
        // cannot be a string cannot be recorded, so it is its own error,
        // never an empty one silently unverifying every lesson anchored here.
        let root = root
            .into_os_string()
            .into_string()
            .map_err(|_| AnchorError::NonUtf8Root {
                root: requested.to_owned(),
            })?;
        Ok(Self {
            root,
            bytes_left: MAX_ANCHOR_BYTES_PER_RECALL,
            files_left: MAX_ANCHOR_FILES_PER_RECALL,
            seen: HashMap::new(),
        })
    }

    /// The canonical checkout root this probe reads below, in the same form
    /// [`AnchorSet::root`] records it. Never empty: [`Self::open`] admits
    /// only a canonical, UTF-8 root, and a root the probe cannot name cannot
    /// record itself.
    #[must_use]
    pub fn root(&self) -> &str {
        &self.root
    }

    /// Anchor a lesson to `path` as it is now: validate the path and read
    /// the file's current digest.
    ///
    /// # Errors
    /// [`AnchorError::InvalidPath`] for a path that is not repository-relative,
    /// and the errors of [`Self::digest`].
    pub fn anchor(&mut self, path: &str) -> Result<FileAnchor, AnchorError> {
        let path = RepoPath::parse(path)?;
        let digest = self.digest(&path)?;
        Ok(FileAnchor { path, digest })
    }

    /// Anchor a lesson to `paths` as they are now, recording this probe's
    /// canonical root with the files. The count is checked before any file
    /// is read.
    ///
    /// # Errors
    /// [`AnchorError::TooMany`], and the errors of [`Self::anchor`].
    pub fn anchors(&mut self, paths: &[&str]) -> Result<AnchorSet, AnchorError> {
        check_anchor_count(paths.len())?;
        let files = paths
            .iter()
            .map(|path| self.anchor(path))
            .collect::<Result<_, _>>()?;
        Ok(AnchorSet {
            root: self.root().to_owned(),
            files,
        })
    }

    /// Resolve an adapter's anchor request — a checkout root and the
    /// repository-relative paths below it — into an [`AnchorSet`] recording
    /// the canonical root and each file's digest. `None` means a plain
    /// remember: no paths at all. A root sent WITHOUT paths is still
    /// validated, so a mistyped or refused root is reported rather than
    /// silently ignored.
    ///
    /// # Errors
    /// [`AnchorError::MissingRoot`] for paths without a root,
    /// [`AnchorError::TooMany`], and the errors of [`Self::open`] and
    /// [`Self::anchor`].
    pub fn resolve_request(
        requested_root: Option<&str>,
        paths: &[String],
        allowlist: &[PathBuf],
    ) -> Result<Option<AnchorSet>, AnchorError> {
        if paths.is_empty() {
            return match requested_root {
                Some(root) => Self::open(root, allowlist).map(|_| None),
                None => Ok(None),
            };
        }
        check_anchor_count(paths.len())?;
        let requested_root = requested_root.ok_or(AnchorError::MissingRoot)?;
        let mut root = Self::open(requested_root, allowlist)?;
        let files = paths
            .iter()
            .map(|path| root.anchor(path))
            .collect::<Result<_, _>>()?;
        Ok(Some(AnchorSet {
            root: root.root().to_owned(),
            files,
        }))
    }

    /// The current digest of `path`, for anchoring a lesson to it now.
    ///
    /// # Errors
    /// [`AnchorError::Unreadable`] when the file is missing, refused by the
    /// security model, or past the request's budget.
    pub fn digest(&mut self, path: &RepoPath) -> Result<ContentDigest, AnchorError> {
        match self.look(path) {
            Look::Digest(digest) => Ok(digest),
            Look::Missing => Err(AnchorError::Unreadable {
                path: path.to_string(),
                reason: "no file exists at that path below the root",
            }),
            Look::Refused(reason) => Err(AnchorError::Unreadable {
                path: path.to_string(),
                reason,
            }),
        }
    }

    /// Look at `path` once per request; later looks reuse the first answer.
    fn look(&mut self, path: &RepoPath) -> Look {
        if let Some(seen) = self.seen.get(path) {
            return seen.clone();
        }
        let look = self.read_digest(path);
        self.seen.insert(path.clone(), look.clone());
        look
    }

    /// Steps 4 and 5 of the module's security model, then the digest.
    fn read_digest(&mut self, path: &RepoPath) -> Look {
        if self.files_left == 0 {
            return Look::Refused("the request's anchor file budget is spent");
        }
        let canonical = match fs::canonicalize(Path::new(&self.root).join(path.as_str())) {
            Ok(canonical) => canonical,
            Err(err) if err.kind() == ErrorKind::NotFound => return Look::Missing,
            Err(_) => return Look::Refused("it cannot be resolved"),
        };
        if !canonical.starts_with(Path::new(&self.root)) {
            return Look::Refused("it resolves outside the anchor root");
        }
        let cap = MAX_ANCHOR_FILE_BYTES.min(self.bytes_left);
        match read_bounded(&canonical, cap) {
            Ok(bytes) => {
                self.files_left -= 1;
                self.bytes_left -= bytes.len();
                Look::Digest(ContentDigest::of(&bytes))
            }
            Err(reason) => Look::Refused(reason),
        }
    }
}

impl AnchorProbe for AnchorRoot {
    fn root(&self) -> &str {
        Self::root(self)
    }

    fn observe(&mut self, anchor: &FileAnchor) -> AnchorObservation {
        match self.look(&anchor.path) {
            Look::Digest(digest) if digest == anchor.digest => AnchorObservation::Unchanged,
            Look::Digest(_) => AnchorObservation::Changed,
            Look::Missing => AnchorObservation::Missing,
            Look::Refused(_) => AnchorObservation::Unverifiable,
        }
    }
}

/// Read a regular file of at most `cap` bytes, refusing anything else.
///
/// Type and size are checked on the OPENED HANDLE, never on the path: a path
/// check followed by a path open would leave room for a swap in between, and
/// a FIFO swapped in that gap opens for reading and then waits for a writer
/// forever — hanging the recall and, behind a queued writer, the daemon.
fn read_bounded(path: &Path, cap: usize) -> Result<Vec<u8>, &'static str> {
    let file = open_for_anchor(path)?;
    let metadata = file.metadata().map_err(|_| "it cannot be inspected")?;
    if !metadata.is_file() {
        return Err("it is not a regular file");
    }
    let too_large = "it is larger than the anchor size budget (1 MiB per file)";
    if !matches!(usize::try_from(metadata.len()), Ok(len) if len <= cap) {
        return Err(too_large);
    }
    let limit = u64::try_from(cap).map_or(u64::MAX, |cap| cap.saturating_add(1));
    let mut bytes = Vec::new();
    file.take(limit)
        .read_to_end(&mut bytes)
        .map_err(|_| "it cannot be read")?;
    if bytes.len() > cap {
        return Err(too_large);
    }
    Ok(bytes)
}

/// The open behind [`read_bounded`]. `O_NONBLOCK` keeps a FIFO from blocking
/// the probe (it has no effect on a regular file); `O_NOFOLLOW` refuses a
/// final-component symlink swapped in since the canonicalization instead of
/// reading it through.
#[cfg(unix)]
fn open_for_anchor(path: &Path) -> Result<fs::File, &'static str> {
    use std::os::unix::fs::OpenOptionsExt;
    fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| "it cannot be opened")
}

/// The non-unix open behind [`read_bounded`]. Neither `O_NOFOLLOW` nor
/// `O_NONBLOCK` exists portably here, so the residual windows the module doc
/// names stay OPEN on this build: a final-component symlink swapped in after
/// canonicalization is followed, and a swapped-in FIFO can block the open on
/// a writer that may never come. The handle-level type/size check still
/// applies, so what did open is verified before a byte is trusted.
#[cfg(not(unix))]
fn open_for_anchor(path: &Path) -> Result<fs::File, &'static str> {
    fs::File::open(path).map_err(|_| "it cannot be opened")
}

#[cfg(test)]
#[path = "anchor_root_tests.rs"]
mod anchor_root_tests;
