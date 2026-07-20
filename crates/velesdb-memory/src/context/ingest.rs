//! Adapter-side I/O pre-pass for `path`-referenced context fragments
//! (V2b-1, see the crate's `PLAN.md`).
//!
//! [`resolve_fragments`] turns every [`ContextFragment::path`] into ordinary
//! `content` — read from disk under a strict, short-circuiting security
//! pipeline — BEFORE the request reaches [`super::ContextCompiler`], which
//! never performs I/O itself (mirrors the [`super::media`] pre-pass: decode
//! once at the boundary, keep the pipeline core pure). Called from the MCP
//! adapter (`crate::mcp::context_tools`) ahead of `compile_context` and
//! `explain_compilation`; a `path` fragment that reaches the compiler
//! unresolved is rejected with [`MemoryError::IngestDisabled`] (see
//! `context::validate`) rather than silently compiling empty content.
//!
//! # Security model
//!
//! Path ingestion is opt-in and allowlisted: nothing is readable unless
//! [`IngestRoots::parse`] was given at least one root
//! (`VELESDB_MEMORY_INGEST_ROOTS`, platform `PATH`-list syntax). Every
//! `path` fragment then runs this ordered, short-circuiting pipeline:
//!
//! 1. Shape: a fragment may set exactly one of `path`, non-empty `content`,
//!    or `media` — checked first, independent of whether ingestion is even
//!    enabled (a malformed request is a malformed request regardless of
//!    server configuration).
//! 2. At least one configured root, else [`MemoryError::IngestDisabled`].
//! 3. The number of `path` fragments in the request is bounded
//!    ([`crate::limits::MAX_INGEST_FILES`]).
//! 4. The path must be absolute — an MCP server's working directory is not
//!    something a caller can rely on.
//! 5. [`std::fs::canonicalize`] resolves *every* symlink in one call.
//! 6. The canonical path is checked against the canonical roots
//!    **component-wise** (never a string prefix, which a sibling directory
//!    name like `/root-evil` next to `/root` would defeat). On failure the
//!    error cites the path the caller ASKED for, never the resolved
//!    target — the target itself can be sensitive (e.g. that a symlink
//!    escapes to a specific system path).
//! 7. `fs::metadata` must report a plain file, within
//!    [`crate::limits::MAX_INGEST_FILE_BYTES`] and the request's running
//!    [`crate::limits::MAX_TOTAL_INGEST_BYTES`] — checked BEFORE any read.
//! 8. `fs::read`, a re-check of the length actually read, then
//!    `String::from_utf8`; non-UTF-8 content is rejected (never lossily
//!    decoded), with a short hint when the leading bytes look like a known
//!    image format.
//!
//! Symlinks are allowed as long as their canonical target lands under a
//! root — no special-casing needed, step 5 already resolves them. TOCTOU
//! (a file changing between steps 7 and 8, or after) is an accepted,
//! documented non-goal: this is a local, single-user server, and the
//! deterministic contract is on the bytes actually read, not on the file as
//! it exists at any other instant.

use std::fs;
use std::path::{Path, PathBuf};

use crate::context::model::ContextFragment;
use crate::error::MemoryError;
use crate::limits::{
    MAX_INGEST_FILES, MAX_INGEST_FILE_BYTES, MAX_TOTAL_INGEST_BYTES, MAX_TRANSCRIPT_BYTES,
};

/// A parsed, canonicalized allowlist of filesystem roots a `path` fragment
/// may resolve under. The only way to construct one is [`Self::parse`] —
/// there is no "allow everything" escape hatch.
#[derive(Debug, Clone, Default)]
pub struct IngestRoots {
    roots: Vec<PathBuf>,
}

impl IngestRoots {
    /// Parse `VELESDB_MEMORY_INGEST_ROOTS`'s value: a platform `PATH`-list
    /// (`:`-separated on Unix, `;`-separated on Windows, via
    /// [`std::env::split_paths`]) of directories, each canonicalized
    /// immediately. Fails fast — a root that does not exist or is not a
    /// directory is a startup configuration error, not something to
    /// discover later on a caller's first `path` fragment. An empty or
    /// unset value parses to an empty (disabled) allowlist, not an error.
    ///
    /// # Errors
    /// A human-readable message naming the offending entry.
    pub fn parse(value: &str) -> Result<Self, String> {
        let mut roots = Vec::new();
        for raw in std::env::split_paths(value) {
            if raw.as_os_str().is_empty() {
                continue;
            }
            let canonical = fs::canonicalize(&raw).map_err(|err| {
                format!(
                    "VELESDB_MEMORY_INGEST_ROOTS entry '{}' could not be resolved: {err}",
                    raw.display()
                )
            })?;
            if !canonical.is_dir() {
                return Err(format!(
                    "VELESDB_MEMORY_INGEST_ROOTS entry '{}' is not a directory",
                    raw.display()
                ));
            }
            roots.push(canonical);
        }
        Ok(Self { roots })
    }

    /// Whether path ingestion is enabled at all (at least one root
    /// configured). `false` short-circuits every `path` fragment with
    /// [`MemoryError::IngestDisabled`] before any filesystem access.
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        !self.roots.is_empty()
    }

    /// Whether the already-canonical `candidate` sits under one of the
    /// roots, checked path-COMPONENT-wise via [`Path::starts_with`] — never
    /// a string prefix, which would wrongly accept a sibling directory that
    /// merely shares a prefix (`/root-evil` under a root of `/root`).
    fn contains(&self, candidate: &Path) -> bool {
        self.roots.iter().any(|root| candidate.starts_with(root))
    }
}

/// Resolve every `path`-carrying fragment of `fragments` into `content`, in
/// place, in one pre-pass — see the module docs for the full ordered
/// pipeline. A no-op (and roots-independent) when no fragment carries a
/// `path`. The first fragment to fail short-circuits the whole request:
/// never a partial resolution, so a caller never has to guess which
/// fragments actually got read.
///
/// # Errors
/// [`MemoryError::IngestPath`] for a malformed request shape or an
/// unreadable/non-UTF-8 path, [`MemoryError::IngestDisabled`] when no root
/// is configured, [`MemoryError::IngestOutsideRoots`] when a path (after
/// resolving symlinks) escapes every root, and [`MemoryError::ContextOverLimit`]
/// for the file-count and byte caps.
pub fn resolve_fragments(
    fragments: &mut [ContextFragment],
    roots: Option<&IngestRoots>,
) -> Result<(), MemoryError> {
    // Step 1 (shape): exactly one of `path`, non-empty `content`, `media` —
    // checked for every fragment up front, independent of whether
    // ingestion is enabled at all.
    for fragment in fragments.iter() {
        if fragment.path.is_some() && (!fragment.content.is_empty() || fragment.media.is_some()) {
            return Err(MemoryError::IngestPath(
                "a fragment may set `path`, or `content`, or `media` — never more than one"
                    .to_owned(),
            ));
        }
    }

    let indices: Vec<usize> = fragments
        .iter()
        .enumerate()
        .filter(|(_, fragment)| fragment.path.is_some())
        .map(|(index, _)| index)
        .collect();
    if indices.is_empty() {
        return Ok(());
    }

    // Step 2: ingestion must be enabled at all.
    let Some(roots) = roots.filter(|roots| roots.is_enabled()) else {
        return Err(MemoryError::IngestDisabled);
    };

    // Step 3: bound the number of files one request may reference.
    if indices.len() > MAX_INGEST_FILES {
        return Err(MemoryError::ContextOverLimit(format!(
            "request references {} files via `path`, exceeding the cap of {MAX_INGEST_FILES}",
            indices.len()
        )));
    }

    let mut total_bytes: usize = 0;
    for index in indices {
        // Indices were filtered on `path.is_some()` above; `take` both fetches
        // the path and clears it without a panicking unwrap. On error the whole
        // request is rejected, so the cleared field is never observed.
        let Some(requested) = fragments[index].path.take() else {
            continue;
        };
        let content = resolve_one(&requested, roots, &mut total_bytes, MAX_INGEST_FILE_BYTES)?;
        fragments[index].content = content;
    }
    Ok(())
}

/// Resolve a `compile_transcript` transcript's `path` field (V2b-2): the same
/// ordered security pipeline as [`resolve_fragments`]'s `path` handling
/// (steps 4 through 8 below), but with [`MAX_TRANSCRIPT_BYTES`] in place of
/// [`MAX_INGEST_FILE_BYTES`] — a transcript is the one caller-facing shape
/// allowed to read past the ordinary 1 MiB fragment ceiling, because
/// [`super::segment::segment_transcript`] segments it into sub-1-MiB pieces
/// immediately after this read, never compiling it as one oversized
/// fragment. `roots` must already be checked enabled by the caller (the MCP
/// adapter mirrors [`resolve_fragments`]'s step 2 before calling this).
///
/// # Errors
/// [`MemoryError::IngestPath`] for a relative path, an unreadable path, a
/// non-plain-file, or non-UTF-8 content; [`MemoryError::IngestOutsideRoots`]
/// when the canonicalized path escapes every root; [`MemoryError::ContextOverLimit`]
/// when the file exceeds [`MAX_TRANSCRIPT_BYTES`].
///
/// `pub`, not `pub(crate)` — like [`resolve_fragments`], part of the public
/// `context::ingest` surface (the `mcp` adapter is its first caller, but the
/// module itself only requires `context`; a build with `context` and no
/// `mcp` would otherwise flag this crate-internal-only function dead code).
pub fn resolve_transcript_path(
    requested: &str,
    roots: &IngestRoots,
) -> Result<String, MemoryError> {
    let mut total_bytes: usize = 0;
    resolve_one(requested, roots, &mut total_bytes, MAX_TRANSCRIPT_BYTES)
}

/// Resolve a single `path` fragment's content — steps 4 through 8 of the
/// module-level pipeline, each short-circuiting on failure. `total_bytes`
/// accumulates across the whole request (the caller threads the same
/// counter through every call) so the aggregate cap
/// ([`MAX_TOTAL_INGEST_BYTES`]) is enforced across files, not just within
/// one. `file_cap` is the per-file ceiling: [`MAX_INGEST_FILE_BYTES`] for an
/// ordinary `path` fragment, [`MAX_TRANSCRIPT_BYTES`] for
/// [`resolve_transcript_path`] (V2b-2) — parameterized rather than a second
/// copy of this function, so the ordered pipeline (steps 4-8) can never drift
/// between the two callers.
fn resolve_one(
    requested: &str,
    roots: &IngestRoots,
    total_bytes: &mut usize,
    file_cap: usize,
) -> Result<String, MemoryError> {
    // Step 4: relative paths are refused outright.
    let requested_path = Path::new(requested);
    if requested_path.is_relative() {
        return Err(MemoryError::IngestPath(format!(
            "path '{requested}' is relative; only absolute paths are accepted"
        )));
    }

    // Step 5: canonicalize resolves every symlink in one call, so the
    // prefix check in step 6 can never be fooled by an intermediate one.
    let canonical = fs::canonicalize(requested_path).map_err(|err| {
        MemoryError::IngestPath(format!("cannot resolve path '{requested}': {err}"))
    })?;

    // Step 6: prefix check by path components against the canonical roots.
    // Cites `requested`, never `canonical` — see the module docs.
    if !roots.contains(&canonical) {
        return Err(MemoryError::IngestOutsideRoots(requested.to_owned()));
    }

    // Step 7: metadata checks BEFORE any read.
    let file_metadata = fs::metadata(&canonical)
        .map_err(|err| MemoryError::IngestPath(format!("cannot stat path '{requested}': {err}")))?;
    if !file_metadata.is_file() {
        return Err(MemoryError::IngestPath(format!(
            "path '{requested}' is not a regular file"
        )));
    }
    let declared_len = usize::try_from(file_metadata.len()).unwrap_or(usize::MAX);
    if declared_len > file_cap {
        return Err(MemoryError::ContextOverLimit(format!(
            "file '{requested}' is {declared_len} bytes, exceeding the cap of {file_cap} bytes"
        )));
    }
    let running_total = total_bytes.saturating_add(declared_len);
    if running_total > MAX_TOTAL_INGEST_BYTES {
        return Err(MemoryError::ContextOverLimit(format!(
            "ingesting '{requested}' would bring the request total to {running_total} bytes, \
             exceeding the cap of {MAX_TOTAL_INGEST_BYTES} bytes"
        )));
    }

    // Step 8: read, re-check the length actually read (the file may have
    // changed since `metadata` — documented TOCTOU non-goal, but a race
    // that would silently blow the byte cap is still caught), then decode.
    let bytes = fs::read(&canonical)
        .map_err(|err| MemoryError::IngestPath(format!("cannot read path '{requested}': {err}")))?;
    if bytes.len() > file_cap {
        return Err(MemoryError::ContextOverLimit(format!(
            "file '{requested}' grew to {} bytes while being read, exceeding the cap of \
             {file_cap} bytes",
            bytes.len()
        )));
    }
    *total_bytes = total_bytes.saturating_add(bytes.len());

    String::from_utf8(bytes).map_err(|err| {
        let hint = magic_bytes_hint(err.as_bytes());
        MemoryError::IngestPath(format!("path '{requested}' is not valid UTF-8{hint}"))
    })
}

/// A short, actionable suffix appended to the non-UTF-8 error when the
/// file's leading bytes match a recognized image format's magic number —
/// steering a caller who ingested a screenshot by mistake toward
/// [`super::model::MediaRef`] instead of leaving them to guess why a
/// `path` fragment failed. Recognizes exactly PNG and JPEG (the two
/// formats [`super::estimator::ImageTokenEstimator`] special-cases);
/// unrecognized binary content gets no hint, never a wrong guess.
fn magic_bytes_hint(bytes: &[u8]) -> &'static str {
    const PNG_MAGIC: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    const JPEG_MAGIC: [u8; 3] = [0xFF, 0xD8, 0xFF];
    if bytes.starts_with(&PNG_MAGIC) || bytes.starts_with(&JPEG_MAGIC) {
        " (looks like an image — use a media fragment instead of `path`)"
    } else {
        ""
    }
}

#[cfg(test)]
#[path = "ingest_tests.rs"]
mod tests;
