//! Stage one: classify, hash and deduplicate every fragment into an `Analysis`.

use super::classify::{self, RuleMatch};
use super::dedup::{self, DupKind, Duplicate};
use super::media;
use super::relevance;
use super::{CompilePolicy, CompileRequest, ContextAction, TokenEstimator};
use crate::id::stable_id;

/// Everything the pipeline derived about one input fragment. Borrows the
/// request (the pipeline never mutates fragments), so a compile at the size
/// caps does not double the corpus in memory.
pub(super) struct Analysis<'a> {
    /// Input position.
    pub(super) seq: usize,
    /// Caller id, or the content-derived stable id.
    pub(super) fragment_id: u64,
    /// FNV-1a hash of the original content (computed once, reused by ids,
    /// dedup, and handles).
    pub(super) content_hash: u64,
    /// The original text, borrowed from the request.
    pub(super) original: &'a str,
    /// Estimated tokens of the original (computed once, reused by insights,
    /// handles, and savings attribution).
    pub(super) tokens: u64,
    /// Classification outcome.
    pub(super) rule: RuleMatch,
    /// Lexical relevance to the query.
    pub(super) relevance: f32,
    /// Caller priority (default 0).
    pub(super) priority: u8,
    /// Set when this fragment duplicates an earlier one it may safely be
    /// dropped for (see [`retain_safe_duplicates`]).
    pub(super) dup: Option<Duplicate>,
    /// Only set for `abstract.log_dedup`-classified fragments: the
    /// collapsed single piece, and whether
    /// [`CompilePolicy::normalize_log_timestamps`] actually changed the
    /// grouping (ventilated into the decision `reason`). Computed once here
    /// so [`pieces`](super::packing::pieces) and [`decision`](super::verdict::decision) never redo the line-scan.
    pub(super) abstract_collapse: Option<(String, bool)>,
    /// Set when the fragment carries inline media (US-009, PR1): its
    /// dedup identity and precomputed image token cost, computed once here
    /// (decoding is not free) and reused by dedup, packing, and insights.
    pub(super) media: Option<media::MediaAnalysis>,
    /// Set when a LATER fragment in the same request shares this one's
    /// `kind == "screenshot"` and `metadata.target` value (US-009, PR2 —
    /// see [`classify::screenshot_supersession`]): excluded from packing
    /// entirely regardless of budget (see [`pack_items`](super::packing::pack_items)) and routed to
    /// `superseded_screenshot_verdict` (in `verdict`) instead of the ordinary
    /// pack-outcome verdicts. `analysis.rule` is left untouched (still
    /// whatever [`classify::classify`] returned) — only this flag steers
    /// packing and the decision; nothing else needs to know why.
    pub(super) superseded: bool,
}

impl Analysis<'_> {
    /// The hash every `ctx://source/<hash>` handle (and thus every bridge
    /// storage slot) for this fragment is minted from. **Media identity is
    /// the raw decoded BYTES** ([`media::MediaAnalysis::raw_hash`]), exactly
    /// like PR1's dedup — never the caption text: captions are typically
    /// blank, and keying on them would collide every captionless image onto
    /// one handle (serving arbitrary wrong bytes back). Two different
    /// images therefore always get two different handles; byte-identical
    /// images share one handle and resolve the same stored bytes (with the
    /// kept instance's caption — divergent duplicate captions do not
    /// survive, same as PR1's dedup semantics). Text fragments keep the
    /// content hash, byte-identical to every pre-PR2 handle.
    pub(super) fn handle_hash(&self) -> u64 {
        self.media
            .as_ref()
            .map_or(self.content_hash, |media| media.raw_hash)
    }
}

/// A media fragment's total precomputed token cost: the image alone (from
/// [`media::MediaAnalysis::image_tokens`]) plus its caption's own (usually
/// tiny, often zero for a blank caption) text cost. Shared by [`analyze`]
/// (feeds [`Analysis::tokens`]) and [`pieces`](super::packing::pieces) (feeds the packed piece's
/// cost) so the two can never drift apart — the same total is what gets
/// budgeted and what gets reported as "emitted" once packed.
pub(super) fn media_fragment_tokens(
    media: &media::MediaAnalysis,
    caption: &str,
    estimator: &dyn TokenEstimator,
) -> u64 {
    media
        .image_tokens
        .saturating_add(estimator.estimate(caption))
}
/// Run classification, relevance scoring, and duplicate detection over the
/// input order, hashing and estimating each fragment exactly once.
pub(super) fn analyze<'a>(
    request: &'a CompileRequest,
    policy: &CompilePolicy,
    estimator: &dyn TokenEstimator,
) -> Vec<Analysis<'a>> {
    let contents: Vec<&str> = request
        .fragments
        .iter()
        .map(|fragment| fragment.content.as_str())
        .collect();
    // Decode/analyze media exactly once per fragment (decoding is not
    // free), reused below both to feed dedup's media namespace and to build
    // each Analysis's own `media` field.
    let media_analyses: Vec<Option<media::MediaAnalysis>> = request
        .fragments
        .iter()
        .map(|fragment| fragment.media.as_ref().map(media::analyze))
        .collect();
    let media_hashes: Vec<Option<u64>> = media_analyses
        .iter()
        .map(|analysis| analysis.as_ref().map(|analysis| analysis.raw_hash))
        .collect();
    // Whole-batch pass, symmetric to `dedup::find_duplicates` below: needs
    // every fragment's `kind` + `metadata.target` at once, which a per-
    // fragment `classify::classify` call cannot see. Computed *before*
    // dedup so the media namespace can re-anchor off a superseded fragment
    // (see `dedup::find_duplicates`'s doc) instead of anchoring dedup on a
    // screenshot that supersession has already excluded from packing.
    let superseded_flags = classify::screenshot_supersession(&request.fragments);
    let duplicates = dedup::find_duplicates(
        &contents,
        policy.near_dup_dedup,
        &media_hashes,
        &superseded_flags,
    );
    let query_terms = relevance::terms(&request.query);
    let mut analyses: Vec<Analysis<'a>> = request
        .fragments
        .iter()
        .zip(duplicates)
        .zip(media_analyses)
        .enumerate()
        .map(|(seq, ((fragment, dup), media_analysis))| {
            let content_hash = stable_id(&fragment.content);
            let rule = classify::classify(fragment, policy);
            let abstract_collapse = (rule.action == ContextAction::Abstract).then(|| {
                classify::collapse_repeated_lines(
                    &fragment.content,
                    policy.normalize_log_timestamps,
                )
            });
            let tokens = media_analysis.as_ref().map_or_else(
                || estimator.estimate(&fragment.content),
                |media| media_fragment_tokens(media, &fragment.content, estimator),
            );
            // A caller can opt out via `disabled_rules`, exactly like every
            // other named rule — even though this one is not a `RULES` row.
            let superseded = superseded_flags[seq]
                && !policy
                    .disabled_rules
                    .iter()
                    .any(|disabled| disabled == classify::SCREENSHOT_SUPERSEDED_RULE_ID);
            Analysis {
                seq,
                fragment_id: fragment.id.unwrap_or(content_hash),
                content_hash,
                original: &fragment.content,
                tokens,
                rule,
                relevance: relevance::lexical_relevance(&query_terms, &fragment.content),
                priority: fragment.priority.unwrap_or(0),
                dup,
                abstract_collapse,
                media: media_analysis,
                superseded,
            }
        })
        .collect();
    retain_safe_duplicates(&mut analyses);
    analyses
}

/// Keep a duplicate mark only when dropping the fragment loses nothing:
/// the kept twin must be classified to emit **verbatim** (Preserve or Cache
/// — an abstracted twin would collapse the duplicate's content), and a
/// *critical* fragment is never sacrificed to near-deduplication (its bytes
/// differ from the twin's, and its own classification demands them).
fn retain_safe_duplicates(analyses: &mut [Analysis<'_>]) {
    for index in 0..analyses.len() {
        let Some(dup) = analyses[index].dup else {
            continue;
        };
        let twin_verbatim = matches!(
            analyses[dup.kept_seq].rule.action,
            ContextAction::Preserve | ContextAction::Cache
        );
        let critical_near = dup.kind == DupKind::Near && analyses[index].rule.critical;
        if !twin_verbatim || critical_near {
            analyses[index].dup = None;
        }
    }
}
