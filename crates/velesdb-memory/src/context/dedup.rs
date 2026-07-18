//! Exact and near-duplicate detection over the input fragments.
//!
//! Identity is content-addressed through the crate's one id scheme
//! ([`crate::id::stable_id`] — FNV-1a 64): exact duplicates hash the raw
//! content, near-duplicates hash a normalized form (lowercased, whitespace
//! runs collapsed). The **first** occurrence survives; later ones are marked
//! duplicates of it. Deterministic by construction: input order decides.
//!
//! **Media fragments** (US-009, PR1) opt out of this entirely: a media
//! fragment's identity is its *raw decoded media bytes*
//! ([`crate::context::media::MediaAnalysis::raw_hash`]), never its caption
//! text — captions are often empty, and two distinct screenshots with blank
//! captions would otherwise collide under the text-content check. Media
//! identity lives in its own namespace (`media_seen`, keyed on the caller-
//! supplied `media_hashes` slice) and is Exact-only: near-duplication (case/
//! whitespace normalization) is a *text* notion and never applies to media.
//! A media fragment neither reads nor writes the text-content tables, so it
//! can never be mistaken for — or mistakenly anchor — a plain-text
//! fragment's dedup chain, even when both have identical (often empty)
//! content strings.

use std::collections::BTreeMap;

use crate::id::stable_id;

/// How a fragment duplicates an earlier one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DupKind {
    /// Byte-identical content.
    Exact,
    /// Identical after case folding and whitespace collapsing.
    Near,
}

/// A later fragment's link to the earlier fragment it duplicates.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Duplicate {
    /// Exact or near.
    pub kind: DupKind,
    /// Index (in the input order) of the surviving first occurrence.
    pub kept_seq: usize,
}

/// For each content (in input order): `None` if it is a first occurrence,
/// `Some(duplicate)` if an earlier fragment already covers it. Near-duplicate
/// detection is skipped when `near` is `false`.
///
/// `media_hashes[i]` is `Some(raw_hash)` when fragment `i` carries media
/// (its identity — see the module doc), `None` for an ordinary text
/// fragment; it must be the same length as `contents` (one entry per
/// fragment, same input order).
pub(crate) fn find_duplicates(
    contents: &[&str],
    near: bool,
    media_hashes: &[Option<u64>],
) -> Vec<Option<Duplicate>> {
    let mut exact_seen: BTreeMap<u64, usize> = BTreeMap::new();
    let mut near_seen: BTreeMap<u64, usize> = BTreeMap::new();
    let mut media_seen: BTreeMap<u64, usize> = BTreeMap::new();
    contents
        .iter()
        .zip(media_hashes)
        .enumerate()
        .map(|(seq, (content, media_hash))| {
            if let Some(&hash) = media_hash.as_ref() {
                // Media's own namespace: Exact-only, never reads or writes
                // the text-content tables below (see the module doc).
                let verdict = media_seen.get(&hash).map(|&kept_seq| Duplicate {
                    kind: DupKind::Exact,
                    kept_seq,
                });
                media_seen.entry(hash).or_insert(seq);
                return verdict;
            }
            let exact_hash = stable_id(content);
            // Skip the normalize+hash pass entirely when near-dup detection
            // is off — the value would never be read.
            let near_hash = near.then(|| stable_id(&normalize(content)));
            let verdict = check(exact_hash, near_hash, &exact_seen, &near_seen);
            // Anchor differently per hash: `near_seen` must always chain to
            // the true root (only the root's bytes are ever emitted, so
            // that is the only fragment whose survival matters), but
            // `exact_seen` must anchor at THIS fragment whenever it is only
            // a *near* match of its twin — its own bytes differ from the
            // root's, so a later byte-identical copy is exact-duplicating
            // *this* fragment, not the root it merely resembles. Chaining an
            // exact match to the root there would let downstream code
            // assume the copy's exact bytes survive whenever the root is
            // emitted verbatim, which is false whenever root and twin
            // differ (exactly why they were only a near match).
            let near_root = verdict.map_or(seq, |dup| dup.kept_seq);
            let exact_anchor = match verdict {
                Some(dup) if dup.kind == DupKind::Exact => dup.kept_seq,
                _ => seq,
            };
            exact_seen.entry(exact_hash).or_insert(exact_anchor);
            if let Some(near_hash) = near_hash {
                near_seen.entry(near_hash).or_insert(near_root);
            }
            verdict
        })
        .collect()
}

/// Classify one content (by its precomputed hashes) against what was seen.
/// `near_hash` is `None` exactly when near-duplicate detection is off.
fn check(
    exact_hash: u64,
    near_hash: Option<u64>,
    exact_seen: &BTreeMap<u64, usize>,
    near_seen: &BTreeMap<u64, usize>,
) -> Option<Duplicate> {
    if let Some(&kept_seq) = exact_seen.get(&exact_hash) {
        return Some(Duplicate {
            kind: DupKind::Exact,
            kept_seq,
        });
    }
    near_seen.get(&near_hash?).map(|&kept_seq| Duplicate {
        kind: DupKind::Near,
        kept_seq,
    })
}

/// The near-duplicate normal form: lowercase, single spaces, trimmed.
fn normalize(content: &str) -> String {
    content
        .split_whitespace()
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
#[path = "dedup_tests.rs"]
mod tests;
