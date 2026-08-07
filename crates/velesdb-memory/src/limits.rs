//! Resource caps shared by every adapter (the MCP server and the language
//! bindings).
//!
//! These are security-relevant DoS limits. They live here — not inside any one
//! adapter — so every transport enforces the *same* numbers without a manual
//! "keep in sync" comment, and so a build without the `mcp` feature still sees
//! them. Each adapter formats its own transport-native error; only the values
//! and the clamping policy are shared.

use crate::service::Metadata;

/// Default hop budget for `why` traversal when the caller supplies none.
pub const DEFAULT_WHY_HOPS: usize = 2;

/// Maximum accepted fact size (1 MiB) — prevents allocating huge embeddings.
pub const MAX_FACT_BYTES: usize = 1_048_576;

/// Maximum accepted size of a fact that has to be **embedded** (2 KiB).
///
/// Much tighter than [`MAX_FACT_BYTES`], and for a different reason: that cap
/// bounds an *allocation*, this one bounds what an embedding model actually
/// accepts. The default backend (`all-minilm`, see
/// [`crate::embedder::DEFAULT_OLLAMA_MODEL`]) has a 512-token context window;
/// at this crate's own prose rate of roughly 3–4 bytes per token (see
/// `context::estimator::TokenEstimator::bytes_per_token_hint`) that is about
/// 2 KiB. Measured against the 0.11.4 daemon: a 2 000-byte fact embeds, an
/// 8 000-byte one fails with `ollama embeddings call failed` — a raw backend
/// error naming neither a limit nor the offending size.
///
/// A guard rail, not a claim of exactness: a caller running a different
/// embedding model may have a wider or narrower real window. It turns the
/// *common* failure into an actionable message instead of an opaque backend
/// fault, and it sits at the largest size measured to work rather than at a
/// value that would reject facts the backend accepts today.
pub const MAX_EMBEDDABLE_TEXT_BYTES: usize = 2048;

/// Maximum accepted size of caller-supplied `metadata` (64 KiB), measured as
/// its serialized JSON form. Metadata is a keyed lookup facet (project,
/// author, status, …) — a porte-clés, not a payload — so it gets a much
/// tighter ceiling than [`MAX_FACT_BYTES`]: without one, a caller could smuggle
/// an arbitrarily large JSON blob through `metadata` on every write path
/// (`remember`, `remember_with_ttl`, `remember_extracted`, and each
/// context-compiler fragment's own `metadata`) and force the same unbounded
/// allocation and storage growth the fact-size cap exists to prevent.
pub const MAX_METADATA_BYTES: usize = 64 * 1024;

/// The serialized JSON size of `meta`, in bytes. Returns `usize::MAX` if the
/// map somehow fails to serialize (it never should — `Metadata` is always
/// valid JSON), so a serialization hiccup fails a size check closed rather
/// than silently passing an unmeasured payload.
#[must_use]
pub fn metadata_bytes(meta: &Metadata) -> usize {
    serde_json::to_vec(meta).map_or(usize::MAX, |v| v.len())
}

/// Cap on a `recall` limit — prevents unbounded vector scans (core does not
/// cap `k`, so the adapters do).
pub const MAX_RECALL_LIMIT: usize = 1_000;

/// Cap on `why`/`recall_fused` hop depth. Bounds DEPTH only: an entity hub
/// reached at any hop is, by construction, a super-node whose degree scales
/// with the whole store, so this alone does not bound how much a walk
/// returns — see [`MAX_WHY_NODE_DEGREE`] and [`MAX_WHY_NODES`] for the width
/// budget (issue #1743: a hub dumped its entire neighborhood, full fact
/// content included, into a single response).
pub const MAX_WHY_HOPS: usize = 10;

/// Maximum outgoing edges a `why`/`recall_fused` graph walk follows from any
/// ONE node. Without this, expanding a single entity hub pushes one edge
/// (and, for each unseen target, a full-content node) per fact that ever
/// mentioned it — the walk's cost is then `O(store size)` at a single hop,
/// no matter how shallow [`MAX_WHY_HOPS`] is set.
pub const MAX_WHY_NODE_DEGREE: usize = 64;

/// Maximum number of nodes one `why`/`recall_fused` graph walk may collect,
/// seed included. [`MAX_WHY_NODE_DEGREE`] bounds any single node's
/// contribution; this bounds the walk's total size across every node it
/// expands, so many hubs each under the per-node cap still cannot together
/// grow a response past a fixed ceiling.
///
/// An exact ceiling, enforced at the push site: the expansion that reaches it
/// stops mid-node. Checking only between expansions read as the same
/// guarantee but let the crossing expansion finish its whole degree first —
/// a measured 522 nodes of a documented 500.
pub const MAX_WHY_NODES: usize = 500;

/// Maximum edges one `why`/`recall_fused` graph walk may record.
///
/// [`MAX_WHY_NODES`] alone does not bound a response: every edge FOLLOWED is
/// recorded even when its target is already visited, so a dense subgraph far
/// under the node budget can still return on the order of
/// `nodes x MAX_WHY_NODE_DEGREE` edges — tens of thousands at the caps, a
/// multi-megabyte response, which is the other half of what issue #1743
/// asked to bound ("nombre maximal de noeuds et d'aretes retournes"). Four
/// edges per node of budget covers a spanning forest (which needs fewer than
/// one edge per node) plus three cross-links per node on top — a fixed
/// ceiling on the response, not a tuning knob.
pub const MAX_WHY_EDGES: usize = MAX_WHY_NODES * 4;

/// Maximum typed edges an `entity` profile resolves and returns PER
/// DIRECTION (`relations` and `relations_in` each) — the same width grammar
/// as [`MAX_WHY_NODE_DEGREE`], on the surface #1743 never covered: resolving
/// an entity followed every non-scaffolding edge of its hub, full target
/// content included, so `entity("X")` on a name mentioned by thousands of
/// facts was a constructible multi-megabyte response (#1820). Truncation is
/// REPORTED (`relations_truncated`/`relations_in_truncated` on the profile),
/// never silent: a profile with exactly this many edges is otherwise
/// indistinguishable from a cut one.
pub const MAX_ENTITY_RELATIONS: usize = 64;

/// Maximum RAW edges an `entity` profile may scan per direction while
/// looking for its [`MAX_ENTITY_RELATIONS`] typed ones. A hub's edges are
/// mostly bipartite scaffolding (`mentions`/`about`, one per mentioning
/// fact), filtered out AFTER the store hands them over — so the resolution
/// cap alone would leave the scan O(degree), and a scan capped at the
/// resolution cap would return scaffolding-only windows on any busy hub.
/// Two named numbers, one per concern: this one bounds the transient scan
/// (the #1743 cost class), the other bounds the resolved response. A typed
/// edge sitting past this window is not found — that blindness is declared
/// by the same truncation flags, not masked.
pub const MAX_ENTITY_SCAN_EDGES: usize = 4_096;

/// Maximum accepted size of a single context-compiler fragment (1 MiB, the
/// same ceiling as [`MAX_FACT_BYTES`]) — prevents a single fragment from
/// forcing huge allocations in the compile pipeline.
pub const MAX_FRAGMENT_BYTES: usize = 1_048_576;

/// Cap on the number of fragments in one compile request — bounds the work a
/// single call can demand across every adapter.
pub const MAX_FRAGMENTS: usize = 1_024;

/// Maximum accepted size of a fragment's base64-encoded media payload
/// (US-009, PR1: inline images) — 4 MiB of base64 text, roughly 3 MiB of raw
/// bytes once decoded. Deliberately separate from [`MAX_FRAGMENT_BYTES`],
/// which only ever measures [`crate::context::model::ContextFragment::content`]
/// (the caption): a screenshot is not text, and capping it at the 1 MiB text
/// ceiling would reject ordinary screenshots outright. Measured against
/// `bytes_b64.len()` (the encoded string), so the cap can reject an
/// oversized payload before any base64 decoding is attempted.
pub const MAX_MEDIA_BYTES: usize = 4 * 1024 * 1024;

/// Aggregate cap on ALL media payloads of one request (base64 length,
/// summed). Without it, `MAX_FRAGMENTS` fragments each at [`MAX_MEDIA_BYTES`]
/// would let a single request carry 4 GiB of media — far past the ~1 GiB
/// worst case the text caps allow. 64 MiB comfortably fits a real
/// screenshot-heavy session while bounding decode work.
pub const MAX_TOTAL_MEDIA_BYTES: usize = 64 * 1024 * 1024;

/// Maximum accepted size of a single file read through a `path`-referenced
/// context fragment (V2b-1 path ingestion) — 1 MiB, the same ceiling as
/// [`MAX_FRAGMENT_BYTES`]: an ingested file becomes an ordinary fragment's
/// `content`, so it must not exceed what a fragment is allowed to carry.
/// Checked from `fs::metadata` BEFORE the file is read, and re-checked after
/// (`fs::read` can race a concurrent write) — never clamped, always refused,
/// so a truncated read can never silently masquerade as the whole file.
pub const MAX_INGEST_FILE_BYTES: usize = 1_048_576;

/// Maximum number of `path`-referenced fragments accepted in one compile
/// request — bounds the filesystem work (and open-file churn) a single call
/// can demand, symmetric to [`MAX_FRAGMENTS`] for inline fragments.
pub const MAX_INGEST_FILES: usize = 64;

/// Aggregate cap on the bytes read across every `path`-referenced fragment of
/// one request (64 MiB) — symmetric to [`MAX_TOTAL_MEDIA_BYTES`]. Without it,
/// [`MAX_INGEST_FILES`] fragments each at [`MAX_INGEST_FILE_BYTES`] would
/// still admit 64 MiB (the two caps happen to coincide at these values), but
/// this cap is checked independently and first — a future change to either
/// per-item constant must not silently loosen the aggregate ceiling.
pub const MAX_TOTAL_INGEST_BYTES: usize = 64 * 1024 * 1024;

/// Maximum accepted size of a `compile_transcript` transcript (V2b-2), inline
/// or `path`-referenced — 8 MiB. The ONE caller-facing shape allowed to read
/// past the ordinary [`MAX_INGEST_FILE_BYTES`]/[`MAX_FRAGMENT_BYTES`] 1 MiB
/// ceiling: a transcript is segmented into sub-1-MiB pieces immediately after
/// being read (see `context::segment`), so it is never itself compiled as one
/// oversized fragment — only the raw pre-segmentation read gets the wider cap.
pub const MAX_TRANSCRIPT_BYTES: usize = 8 * 1024 * 1024;

/// Cap on a caller-supplied token budget. A budget cannot force allocations
/// by itself, but an absurd value would make the savings arithmetic
/// meaningless, so adapters clamp to this ceiling instead of erroring.
pub const MAX_TOKEN_BUDGET: u64 = 10_000_000;

/// Clamp a caller-supplied token budget to [`MAX_TOKEN_BUDGET`].
#[must_use]
pub fn clamp_token_budget(budget: u64) -> u64 {
    budget.min(MAX_TOKEN_BUDGET)
}

/// Clamp a caller-supplied recall limit to [`MAX_RECALL_LIMIT`].
#[must_use]
pub fn clamp_recall_limit(k: usize) -> usize {
    k.min(MAX_RECALL_LIMIT)
}

/// Clamp a caller-supplied `why` hop budget to [`MAX_WHY_HOPS`].
#[must_use]
pub fn clamp_hops(hops: usize) -> usize {
    hops.min(MAX_WHY_HOPS)
}
