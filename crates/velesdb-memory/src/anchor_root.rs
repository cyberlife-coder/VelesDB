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
//! 5. the target is a regular file of at most
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
//! TOCTOU between the check and the read is an accepted non-goal, as for the
//! ingest pipeline: a local single-user server, and a verdict about the bytes
//! actually read.

use std::collections::HashMap;
use std::fs;
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};

use super::{
    check_anchor_count, AnchorError, AnchorObservation, AnchorProbe, ContentDigest, FileAnchor,
    RepoPath,
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
    root: PathBuf,
    bytes_left: usize,
    files_left: usize,
    seen: HashMap<RepoPath, Look>,
}

impl AnchorRoot {
    /// Admit `requested` as a checkout root if it lies inside one of the
    /// already-canonical `allowlist` roots.
    ///
    /// # Errors
    /// [`AnchorError::Disabled`] when `allowlist` is empty, and
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
        Ok(Self {
            root,
            bytes_left: MAX_ANCHOR_BYTES_PER_RECALL,
            files_left: MAX_ANCHOR_FILES_PER_RECALL,
            seen: HashMap::new(),
        })
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

    /// Resolve an adapter's anchor request — a checkout root and the
    /// repository-relative paths below it — into anchors, reading each file
    /// once. No path means no anchor, whatever the root. The count is checked
    /// before any file is read.
    ///
    /// # Errors
    /// [`AnchorError::MissingRoot`] for paths without a root,
    /// [`AnchorError::TooMany`], and the errors of [`Self::open`] and
    /// [`Self::anchor`].
    pub fn resolve_request(
        requested_root: Option<&str>,
        paths: &[String],
        allowlist: &[PathBuf],
    ) -> Result<Vec<FileAnchor>, AnchorError> {
        if paths.is_empty() {
            return Ok(Vec::new());
        }
        check_anchor_count(paths.len())?;
        let requested_root = requested_root.ok_or(AnchorError::MissingRoot)?;
        let mut root = Self::open(requested_root, allowlist)?;
        paths.iter().map(|path| root.anchor(path)).collect()
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
        let canonical = match fs::canonicalize(self.root.join(path.as_str())) {
            Ok(canonical) => canonical,
            Err(err) if err.kind() == ErrorKind::NotFound => return Look::Missing,
            Err(_) => return Look::Refused("it cannot be resolved"),
        };
        if !canonical.starts_with(&self.root) {
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
fn read_bounded(path: &Path, cap: usize) -> Result<Vec<u8>, &'static str> {
    let metadata = fs::metadata(path).map_err(|_| "it cannot be inspected")?;
    if !metadata.is_file() {
        return Err("it is not a regular file");
    }
    let too_large = "it is larger than the anchor size budget (1 MiB per file)";
    if !matches!(usize::try_from(metadata.len()), Ok(len) if len <= cap) {
        return Err(too_large);
    }
    let file = fs::File::open(path).map_err(|_| "it cannot be opened")?;
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

#[cfg(test)]
#[path = "anchor_root_tests.rs"]
mod anchor_root_tests;
