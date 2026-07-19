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
//!
//! **Media re-anchoring** (US-009, PR3): the media namespace anchors on the
//! *first* occurrence by default, same as text — except when that anchor is
//! itself superseded (see [`super::classify::screenshot_supersession`]) and a
//! later, byte-identical occurrence is not. Anchoring on a superseded fragment
//! would make every byte-identical twin "dup of a fragment excluded from
//! packing," which drops the whole image chain from the compiled output.
//! Re-anchoring on the later, non-superseded occurrence guarantees the
//! freshest surviving copy is exactly the one every duplicate resolves to.

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
///
/// `media_superseded[i]` is `true` when fragment `i` was flagged by
/// [`super::classify::screenshot_supersession`] (ignored for a `None` media
/// hash); it drives the re-anchoring described in the module doc and must
/// also be the same length as `contents`.
pub(crate) fn find_duplicates(
    contents: &[&str],
    near: bool,
    media_hashes: &[Option<u64>],
    media_superseded: &[bool],
) -> Vec<Option<Duplicate>> {
    let mut exact_seen: BTreeMap<u64, usize> = BTreeMap::new();
    let mut near_seen: BTreeMap<u64, usize> = BTreeMap::new();
    // Media anchor per raw hash, paired with whether that anchor fragment is
    // itself superseded — see "Media re-anchoring" in the module doc.
    let mut media_seen: BTreeMap<u64, (usize, bool)> = BTreeMap::new();
    let mut results: Vec<Option<Duplicate>> = Vec::with_capacity(contents.len());
    for (seq, (content, media_hash)) in contents.iter().zip(media_hashes).enumerate() {
        if let Some(&hash) = media_hash.as_ref() {
            // Media's own namespace: Exact-only, never reads or writes
            // the text-content tables below (see the module doc).
            let current_superseded = media_superseded.get(seq).copied().unwrap_or(false);
            let verdict =
                media_verdict(&mut media_seen, &mut results, hash, seq, current_superseded);
            results.push(verdict);
            continue;
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
        results.push(verdict);
    }
    results
}

/// Classify one media fragment against the media namespace's current anchor
/// for `hash`, re-anchoring on `seq` when the existing anchor is superseded
/// and `seq` is not (see "Media re-anchoring" in the module doc). When a
/// re-anchor happens, the stale anchor's own `results` entry — already
/// pushed as `None` when it was first seen — is rewritten in place to point
/// at the new anchor.
fn media_verdict(
    media_seen: &mut BTreeMap<u64, (usize, bool)>,
    results: &mut [Option<Duplicate>],
    hash: u64,
    seq: usize,
    current_superseded: bool,
) -> Option<Duplicate> {
    match media_seen.get(&hash) {
        Some(&(anchor_seq, anchor_superseded)) if anchor_superseded && !current_superseded => {
            results[anchor_seq] = Some(Duplicate {
                kind: DupKind::Exact,
                kept_seq: seq,
            });
            media_seen.insert(hash, (seq, current_superseded));
            None
        }
        Some(&(anchor_seq, _)) => Some(Duplicate {
            kind: DupKind::Exact,
            kept_seq: anchor_seq,
        }),
        None => {
            media_seen.insert(hash, (seq, current_superseded));
            None
        }
    }
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
