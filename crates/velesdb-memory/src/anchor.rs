//! File anchors: tie a lesson to the files it is about, so recall can tell
//! when the code under it has moved on.
//!
//! A lesson remembered with anchors carries, for each file, its
//! repository-relative [`RepoPath`](crate::anchor::RepoPath) and the
//! [`ContentDigest`](crate::anchor::ContentDigest) of the bytes the lesson was
//! written against. At recall time an
//! [`AnchorProbe`](crate::anchor::AnchorProbe) observes each file again and
//! [`assess_freshness`](crate::anchor::assess_freshness) turns those
//! observations into a [`Freshness`](crate::anchor::Freshness) verdict: a
//! lesson whose file changed or disappeared is
//! [`Freshness::Stale`](crate::anchor::Freshness::Stale), and
//! [`Freshness::rank_factor`](crate::anchor::Freshness::rank_factor)
//! down-weights it in the ranking without hiding it.
//!
//! # Where the current digest comes from
//!
//! The server computes it, never the caller. The only production probe is
//! [`AnchorRoot`](crate::anchor::AnchorRoot), which reads files under a
//! checkout root that must itself lie inside the operator's allowlist (the ingest roots,
//! `VELESDB_MEMORY_INGEST_ROOTS`). A digest supplied by the agent would be the
//! agent's own claim about the code, which is exactly what this signal exists
//! to replace; and a hook cannot reach the ranking at all, since it never
//! opens the store. The cost is bounded per recall (see
//! [`crate::limits::MAX_ANCHOR_BYTES_PER_RECALL`]) and paid only when the
//! caller names a root.
//!
//! # Staleness is a ranking signal, not a reward
//!
//! A changed file says the lesson may be out of date, not that it was wrong
//! or unhelpful. So freshness is recomputed on every recall and never
//! written back: it bends the order of one result set and leaves the learned
//! confidence ([`MemoryService::feedback`](crate::MemoryService::feedback))
//! untouched. Reverting the file makes the lesson fresh again.
//!
//! # One checkout, one verdict
//!
//! An [`AnchorSet`](crate::anchor::AnchorSet) records the canonical checkout
//! root its files were read under. A recall judges a lesson only against that
//! same root: a probe of another checkout (another clone, another worktree, a
//! moved directory) has nothing to compare, so the lesson reads
//! [`Freshness::Unverified`](crate::anchor::Freshness::Unverified) there and
//! keeps its rank. Two repositories that happen to hold `src/lib.rs` never
//! make each other's lessons stale. A moved checkout reads as unverified
//! until the lesson is anchored again from its new place.
//!
//! # Storage
//!
//! Anchors live in the reserved `_veles_anchors` payload key, so they never
//! appear in caller metadata, survive a restart, and are carried forward when
//! the same fact is remembered again without anchors. Remembering it again
//! WITH anchors replaces them, which is how a lesson is re-anchored after it
//! has been re-verified.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::service::Metadata;

#[path = "anchor_root.rs"]
mod root;
pub use root::AnchorRoot;

pub use crate::service::AnchoredCandidates;

/// Reserved payload key holding a fact's anchors.
pub(crate) const ANCHORS_KEY: &str = "_veles_anchors";

/// Most anchors one lesson may carry. A lesson about more files than this is
/// a summary, and its staleness would say nothing precise.
pub const MAX_ANCHORS_PER_FACT: usize = 8;

/// Longest accepted repository-relative path, in bytes.
pub const MAX_ANCHOR_PATH_BYTES: usize = 512;

/// The factor a stale lesson's ranking key is multiplied by. Half: a stale
/// lesson stays in the result, flagged, but yields its place to a fresh one
/// of comparable relevance.
pub const STALE_RANK_FACTOR: f32 = 0.5;

/// Why an anchor request was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum AnchorError {
    /// The path is not a plain repository-relative path.
    #[error("anchor path '{path}' is not a repository-relative path: {reason}")]
    InvalidPath {
        /// The path as the caller wrote it.
        path: String,
        /// Which rule it broke.
        reason: &'static str,
    },
    /// More anchors than [`MAX_ANCHORS_PER_FACT`].
    #[error("{count} anchors requested, at most {max} are accepted")]
    TooMany {
        /// Anchors in the request.
        count: usize,
        /// The cap.
        max: usize,
    },
    /// Anchor paths were given without the checkout root they are relative to.
    #[error("anchor paths need the checkout root they are relative to (`anchor_root`)")]
    MissingRoot,
    /// No allowlisted root is configured, so no file may be read.
    #[error(
        "file anchoring is disabled: set VELESDB_MEMORY_INGEST_ROOTS to allow the server to read anchored files"
    )]
    Disabled,
    /// The checkout root is relative, unresolvable, not a directory, or
    /// outside every allowlisted root. Cites the root as requested, never
    /// the resolved location.
    #[error("anchor root '{root}' refused: {reason}")]
    RootRefused {
        /// The root as the caller wrote it.
        root: String,
        /// Which rule it broke.
        reason: &'static str,
    },
    /// An anchored file could not be read at remember time.
    #[error("anchored file '{path}' cannot be read: {reason}")]
    Unreadable {
        /// The repository-relative path.
        path: String,
        /// Why.
        reason: &'static str,
    },
}

/// A validated repository-relative path: `/`-separated, every component a
/// plain name. No root, no `.` or `..`, no backslash, no control character —
/// joined to a checkout root it can only name something below that root, by
/// construction rather than by a later check.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct RepoPath(String);

impl RepoPath {
    /// Validate `path`.
    ///
    /// # Errors
    /// [`AnchorError::InvalidPath`] naming the broken rule.
    pub fn parse(path: &str) -> Result<Self, AnchorError> {
        let refuse = |reason| AnchorError::InvalidPath {
            path: path.to_owned(),
            reason,
        };
        if path.is_empty() || path.len() > MAX_ANCHOR_PATH_BYTES {
            return Err(refuse("empty or longer than 512 bytes"));
        }
        if path.chars().any(|c| c.is_control() || c == '\\') {
            return Err(refuse("control characters and backslashes are refused"));
        }
        if path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        {
            return Err(refuse(
                "every component must be a plain name (no root, `.` or `..`)",
            ));
        }
        Ok(Self(path.to_owned()))
    }

    /// The path as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for RepoPath {
    type Error = AnchorError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<RepoPath> for String {
    fn from(path: RepoPath) -> Self {
        path.0
    }
}

impl fmt::Display for RepoPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The SHA-256 of a file's bytes, written `sha256:<64 lowercase hex>`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ContentDigest(String);

impl ContentDigest {
    const PREFIX: &'static str = "sha256:";

    /// The digest of `bytes`.
    #[must_use]
    pub fn of(bytes: &[u8]) -> Self {
        let hash = Sha256::digest(bytes);
        let mut text = String::with_capacity(Self::PREFIX.len() + 64);
        text.push_str(Self::PREFIX);
        for byte in hash {
            text.push(hex_digit(byte >> 4));
            text.push(hex_digit(byte & 0x0f));
        }
        Self(text)
    }

    /// The digest as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ContentDigest {
    type Error = &'static str;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        let hex = value
            .strip_prefix(Self::PREFIX)
            .ok_or("a digest starts with `sha256:`")?;
        if hex.len() != 64 || !hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            return Err("a digest carries 64 lowercase hex digits");
        }
        Ok(Self(value))
    }
}

impl From<ContentDigest> for String {
    fn from(digest: ContentDigest) -> Self {
        digest.0
    }
}

/// One lowercase hex digit for a nibble.
fn hex_digit(nibble: u8) -> char {
    char::from(b"0123456789abcdef"[usize::from(nibble & 0x0f)])
}

/// A lesson's claim about one file: these bytes, at this path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileAnchor {
    /// Where the file lives, relative to the checkout root.
    pub path: RepoPath,
    /// What it contained when the lesson was anchored.
    pub digest: ContentDigest,
}

/// A lesson's anchors: the checkout they were read in, and the files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnchorSet {
    /// The canonical checkout root the files were read under. A probe of any
    /// other root cannot judge them.
    pub root: String,
    /// The anchored files, relative to `root`.
    pub files: Vec<FileAnchor>,
}

/// What a probe saw when it looked at an anchored file again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum AnchorObservation {
    /// Same bytes as when the lesson was anchored.
    Unchanged,
    /// The file is there, with different bytes.
    Changed,
    /// No file at that path any more.
    Missing,
    /// The probe could not look: refused (outside the root, not a regular
    /// file, too large) or out of budget. Says nothing about the lesson.
    Unverifiable,
}

/// Something that can look at an anchored file now, below one checkout.
pub trait AnchorProbe {
    /// The canonical checkout root this probe reads under; an
    /// [`AnchorSet`] recorded under another root is not judged by it.
    fn root(&self) -> &str;
    /// Observe `anchor`'s file and compare it with the anchored digest.
    fn observe(&mut self, anchor: &FileAnchor) -> AnchorObservation;
}

/// A lesson's freshness against the files it is anchored to.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Freshness {
    /// The lesson carries no anchor.
    Unanchored,
    /// Every anchored file still holds the anchored bytes.
    Fresh,
    /// At least one anchored file changed or disappeared.
    Stale {
        /// The anchored paths whose file changed or disappeared.
        changed: Vec<RepoPath>,
    },
    /// No anchored file changed, but at least one could not be checked, the
    /// stored anchors are unreadable, or they were recorded under another
    /// checkout root than the one probed.
    Unverified,
}

impl Freshness {
    /// The factor this verdict applies to a lesson's ranking key:
    /// [`STALE_RANK_FACTOR`] when stale, `1.0` otherwise.
    #[must_use]
    pub fn rank_factor(&self) -> f32 {
        match self {
            Self::Stale { .. } => STALE_RANK_FACTOR,
            _ => 1.0,
        }
    }
}

/// Judge `anchors` through `probe`. A change or a disappearance anywhere
/// makes the lesson stale, and only a change can: an unverifiable file never
/// does, since not being able to look is not evidence. Anchors recorded under
/// another checkout root than the probe's are not looked at at all: they are
/// unverified, never stale.
pub fn assess_freshness(anchors: &AnchorSet, probe: &mut dyn AnchorProbe) -> Freshness {
    if anchors.files.is_empty() {
        return Freshness::Unanchored;
    }
    if anchors.root != probe.root() {
        return Freshness::Unverified;
    }
    let mut changed = Vec::new();
    let mut unverified = false;
    for anchor in &anchors.files {
        match probe.observe(anchor) {
            AnchorObservation::Unchanged => {}
            AnchorObservation::Changed | AnchorObservation::Missing => {
                changed.push(anchor.path.clone());
            }
            AnchorObservation::Unverifiable => unverified = true,
        }
    }
    if !changed.is_empty() {
        Freshness::Stale { changed }
    } else if unverified {
        Freshness::Unverified
    } else {
        Freshness::Fresh
    }
}

/// A recalled lesson with its freshness verdict.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct AnchoredRecollection {
    /// The recalled memory, exactly as [`MemoryService::recall`](crate::MemoryService::recall) returns it.
    pub recollection: crate::model::Recollection,
    /// Its freshness against the probed checkout.
    pub freshness: Freshness,
}

/// Refuse more than [`MAX_ANCHORS_PER_FACT`] anchors.
///
/// # Errors
/// [`AnchorError::TooMany`].
pub fn check_anchor_count(count: usize) -> Result<(), AnchorError> {
    if count > MAX_ANCHORS_PER_FACT {
        return Err(AnchorError::TooMany {
            count,
            max: MAX_ANCHORS_PER_FACT,
        });
    }
    Ok(())
}

/// The reserved payload entry recording `anchors`.
pub(crate) fn anchors_entry(anchors: &AnchorSet) -> Metadata {
    let mut entry = Metadata::new();
    entry.insert(
        ANCHORS_KEY.to_owned(),
        serde_json::to_value(anchors).unwrap_or(Value::Null),
    );
    entry
}

/// What a raw payload records about anchors.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum StoredAnchors {
    /// No anchor entry: an unanchored lesson.
    Unanchored,
    /// A well-formed entry.
    Anchored(AnchorSet),
    /// An entry that does not parse, holds no file or too many: it must read
    /// as "cannot verify", never as "fresh".
    Unreadable,
}

/// The anchors a raw payload records.
pub(crate) fn stored_anchors(payload: Option<&Metadata>) -> StoredAnchors {
    let Some(value) = payload.and_then(|payload| payload.get(ANCHORS_KEY)) else {
        return StoredAnchors::Unanchored;
    };
    match serde_json::from_value::<AnchorSet>(value.clone()) {
        Ok(set) if (1..=MAX_ANCHORS_PER_FACT).contains(&set.files.len()) => {
            StoredAnchors::Anchored(set)
        }
        _ => StoredAnchors::Unreadable,
    }
}

/// The freshness of a lesson whose raw payload is `payload`.
pub(crate) fn payload_freshness(
    payload: Option<&Metadata>,
    probe: &mut dyn AnchorProbe,
) -> Freshness {
    match stored_anchors(payload) {
        StoredAnchors::Unanchored => Freshness::Unanchored,
        StoredAnchors::Anchored(anchors) => assess_freshness(&anchors, probe),
        StoredAnchors::Unreadable => Freshness::Unverified,
    }
}

#[cfg(test)]
#[path = "anchor_tests.rs"]
mod anchor_tests;
