//! Stage two: pack the analysed fragments under the token budget into emissions and sections.

use super::analysis::{media_fragment_tokens, Analysis};
use super::budget::{self, PackItem};
use super::{
    chunk_text, ChunkPolicy, CompilePolicy, CompiledSection, ContextAction, SectionKind,
    TokenEstimator,
};
use std::collections::BTreeMap;

/// What actually got emitted for one packed fragment.
pub(super) struct Emission {
    /// The emitted text (a prefix of the fragment's pieces, concatenated).
    pub(super) text: String,
    /// Pieces taken / pieces available.
    pub(super) taken: usize,
    /// Total pieces the fragment was cut into.
    pub(super) total: usize,
}

impl Emission {
    /// Whether the fragment's pieces were all emitted.
    pub(super) fn is_full(&self) -> bool {
        self.taken == self.total
    }
}
/// Build the packing input for every non-duplicate fragment: abstracted
/// fragments emit their collapsed form as one piece, everything else is cut
/// into budget-sized chunks.
pub(super) fn pack_items(
    analyses: &[Analysis],
    policy: &CompilePolicy,
    usable: u64,
    estimator: &dyn TokenEstimator,
) -> Vec<PackItem> {
    let chunk_policy = effective_chunk_policy(policy, usable, estimator);
    analyses
        .iter()
        // A superseded screenshot (US-009, PR2) is never attempted, budget
        // or no budget — see `Analysis::superseded`.
        .filter(|analysis| analysis.dup.is_none() && !analysis.superseded)
        .map(|analysis| PackItem {
            seq: analysis.seq,
            critical: analysis.rule.critical,
            priority: analysis.priority,
            relevance: analysis.relevance,
            // Query-independent selection tier (issue #1455): see
            // `budget::selection_order`.
            cache: analysis.rule.action == ContextAction::Cache,
            pieces: pieces(analysis, &chunk_policy, estimator),
        })
        .collect()
}

/// The emission pieces of one fragment.
///
/// A media fragment (US-009, PR1) is always exactly one atomic, all-or-
/// nothing piece — never passed to [`chunk_text`], mirroring the
/// `abstract.log_dedup` case below: packing can take it whole or not at all,
/// never a byte-range prefix, so an image can never be cut mid-stream. Its
/// text is only the caption (raw media bytes never become packable "piece"
/// text); its cost is the precomputed [`media_fragment_tokens`] total, so
/// packing never re-derives a media fragment's cost from `estimator.estimate`
/// over an empty or near-empty caption.
pub(super) fn pieces(
    analysis: &Analysis,
    chunk_policy: &ChunkPolicy,
    estimator: &dyn TokenEstimator,
) -> Vec<budget::Piece> {
    if let Some(media) = &analysis.media {
        let cost = media_fragment_tokens(media, analysis.original, estimator);
        return vec![budget::Piece {
            text: analysis.original.to_owned(),
            cost: Some(cost),
        }];
    }
    if let Some((collapsed, _normalized)) = &analysis.abstract_collapse {
        return vec![budget::Piece {
            text: collapsed.clone(),
            cost: None,
        }];
    }
    chunk_text(analysis.original, chunk_policy)
        .into_iter()
        .map(|chunk| budget::Piece {
            text: chunk.text,
            cost: None,
        })
        .collect()
}

/// Lower bound on the pipeline's effective chunk size, regardless of budget
/// or caller policy. Guards against memory-amplification: without a floor, a
/// tiny `token_budget` (or a tiny caller-supplied `max_chunk_bytes`) would
/// drive the ceiling toward one byte and explode a large fragment into one
/// heap `String` per byte. At 256 bytes the per-piece `String` overhead is
/// under a tenth, so pieces stay bounded by ~`input_bytes / 256` — no
/// amplification beyond the already-capped input size ([`crate::limits`]).
const MIN_CHUNK_BYTES: usize = 256;

/// The chunk policy the pipeline actually cuts with: the ceiling tracks the
/// usable budget (sized via the estimator's bytes-per-token hint) but is
/// **floored at [`MIN_CHUNK_BYTES`]** so neither a tiny budget nor a tiny
/// caller-supplied `max_chunk_bytes` can drive it toward a byte (a
/// memory-amplification `DoS`). A budget too small to hold a floored piece
/// simply externalizes everything, which is the correct outcome anyway.
/// **Overlap is forced to zero** — pipeline pieces are emitted by
/// concatenation, and an overlap prefix would duplicate every seam in
/// content reported as verbatim; overlap stays meaningful only for the
/// standalone [`chunk_text`] API. The byte ceiling is a *hint*: every piece
/// is still measured by the injected estimator during packing.
fn effective_chunk_policy(
    policy: &CompilePolicy,
    usable: u64,
    estimator: &dyn TokenEstimator,
) -> ChunkPolicy {
    let budget_bytes = usize::try_from(usable.saturating_mul(estimator.bytes_per_token_hint()))
        .unwrap_or(usize::MAX);
    ChunkPolicy {
        max_chunk_bytes: policy
            .chunk
            .max_chunk_bytes
            .min(budget_bytes)
            .max(MIN_CHUNK_BYTES),
        overlap_bytes: 0,
        boundary: policy.chunk.boundary,
    }
}

/// Materialize what each packed fragment emits, keyed by `seq`. A fragment
/// with no pieces at all (empty content) is kept here with `taken == total
/// == 0` — trivially fully emitted, since there is nothing to lose — rather
/// than dropped as "took none of what was offered", which is reserved for a
/// fragment that had pieces and the budget could not fit any of them.
pub(super) fn emissions(items: &[PackItem], taken: &[usize]) -> BTreeMap<usize, Emission> {
    items
        .iter()
        .zip(taken.iter().copied())
        .filter(|&(item, count)| count > 0 || item.pieces.is_empty())
        .map(|(item, count)| {
            (
                item.seq,
                Emission {
                    text: item.pieces[..count]
                        .iter()
                        .map(|piece| piece.text.as_str())
                        .collect(),
                    taken: count,
                    total: item.pieces.len(),
                },
            )
        })
        .collect()
}

/// The output blocks: the cache-marked prefix first, then the body, both in
/// input order.
pub(super) fn sections(
    analyses: &[Analysis],
    emissions: &BTreeMap<usize, Emission>,
) -> Vec<CompiledSection> {
    let mut result = Vec::new();
    for kind in [SectionKind::Cache, SectionKind::Body] {
        let mut blocks: Vec<&str> = Vec::new();
        let mut ids: Vec<u64> = Vec::new();
        for analysis in analyses {
            let cache = analysis.rule.action == ContextAction::Cache;
            let wanted = (kind == SectionKind::Cache) == cache;
            // Skip empty emissions: a trivially-emitted empty fragment
            // (taken == total == 0) still gets its own decision, but must
            // contribute no block — otherwise `join(JOINER)` would wrap it in
            // joiners the packer never accounted for, breaking the budget
            // invariant once more than one empty fragment is present.
            if let Some(emission) = emissions
                .get(&analysis.seq)
                .filter(|emission| wanted && !emission.text.is_empty())
            {
                blocks.push(&emission.text);
                ids.push(analysis.fragment_id);
            }
        }
        if !blocks.is_empty() {
            result.push(CompiledSection {
                kind,
                content: blocks.join(budget::JOINER),
                fragment_ids: ids,
            });
        }
    }
    result
}

#[cfg(test)]
#[path = "packing_tests.rs"]
mod tests;
