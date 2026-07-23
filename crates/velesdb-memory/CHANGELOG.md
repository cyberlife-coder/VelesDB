# Changelog — velesdb-memory

All notable changes to the `velesdb-memory` crate are documented here. This
crate is versioned independently of the VelesDB workspace (0.x cadence) and is
released on its own `velesdb-memory-vX.Y.Z` tag.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `scripts/install-memory-daemon.sh` now wires **Devin CLI**
  (`~/.config/devin/config.json`) alongside Claude Code, Claude Desktop and
  Windsurf; documented in the README's stdio and HTTP-transport client
  sections.

### Fixed

- The installer no longer writes a `type:"http"` entry into
  `claude_desktop_config.json` — confirmed Desktop's config file never reads
  that shape (silently ignored). It now prints the Settings → Connectors →
  Add custom connector instructions instead, matching what the README already
  documented.

## [0.11.0] — 2026-07-23

Minor, not patch: the metadata shape `remember`/`recall` return changes
observably for every consumer (MCP, Python, Node, WASM) — see "Changed"
below.

### Added

- **HTTPS by default for the HTTP transport.** `--http`/`VELESDB_MEMORY_HTTP=1`
  now serves TLS by default, terminated with a self-signed local CA + a
  short-lived `localhost`/`127.0.0.1`/`::1` leaf certificate, both generated
  natively (`rcgen`, no shelled-out `mkcert`/`openssl`) and cached at
  `$VELESDB_MEMORY_TLS_DIR` (default `~/.velesdb-memory-tls`, a sibling of
  the store). The CA is generated once and never regenerated once present —
  a client only needs to trust it once, and every future leaf cert (even
  across restarts) is trusted automatically after that. Some MCP clients
  (Claude Desktop's "Add custom connector" UI) refuse a non-`https://` URL
  even for `127.0.0.1`, which this closes. `--http-insecure` /
  `VELESDB_MEMORY_HTTP_INSECURE=1` opts back into plain HTTP (loud warning
  at startup) for local debugging or when a trusted TLS-terminating proxy
  already sits in front. `scripts/install-memory-daemon.sh` adds the CA to
  the macOS login keychain (`security add-trusted-cert`, no `sudo`) and
  gained `--tls-dir` and `--skip-ca-trust` flags. See the README's "HTTP
  transport (multi-client)" section.
- **Automatic `_veles_date` stamp.** `remember`/`remember_with_ttl` (and
  `remember_extracted`, which delegates to `remember` per extracted fact)
  now auto-stamp every fact's metadata with `_veles_date` — today's date as
  a `YYYYMMDD` integer, read from the system clock at write time — unless
  the caller already set that key explicitly (an explicit value, e.g. for
  retroactive dating, is never overwritten). `recall_fused`'s `date_field`
  can now be pointed at `_veles_date` to get a correct `dated_context`
  timeline with zero caller setup — previously every temporal capability
  depended entirely on the caller managing a numeric date field itself,
  documented but never guaranteed. The new `AUTO_DATE_FIELD` constant is
  exported at the crate root as the single source of truth for the key
  name. The context compiler (`compile_context` and friends) reads no
  clock and is unaffected — the auto-stamp lives exclusively on the
  `remember` write path.

### Changed

- **Breaking (observable shape, not a compile break): `metadata` is no
  longer `None`/`null` for a fact stored with no caller metadata.** Because
  of the `_veles_date` auto-stamp above, `recall`/`recall_where`/
  `recall_fused` now return `metadata: {"_veles_date": <today>}` instead of
  `metadata: None`/`null` for such a fact, on every binding (MCP JSON-RPC,
  Python, Node, WASM). Callers that previously branched on "metadata is
  `None`" to mean "nothing was ever stored" should check for the
  caller-specific key(s) they care about instead.

## [0.10.1] — 2026-07-21

### Fixed

- The `compile_context` prompt-cache prefix could churn when only the query
  changed: `selection_order` (`src/context/budget.rs`) used lexical
  relevance to the query as a packing tie-break for every fragment,
  including `cache: true` ones, so when a budget was too tight to fit two
  same-priority cache-marked fragments, a query change alone could flip
  which one won, silently changing the Cache section's bytes and defeating
  provider prompt-caching on exactly the turn a new question was asked. A
  cache-marked fragment's rank now never consults relevance: it always
  outranks a non-cache fragment at the same criticality/priority (a fixed,
  query-independent tier), and two cache-marked fragments tied on priority
  fall straight to `seq`. **Trade-off, assumed:** cache stability over
  relevance, for cache-marked fragments only — a more-relevant non-cache
  fragment can now lose a tight-budget race it would have won before this
  fix against a same-tier cache fragment. Non-cache fragments are
  unaffected. (issue #1455)

## [0.10.0] — 2026-07-20

### Added

- **Binding parity for the compiler's read tools (V2d-2/A4).**
  `MemoryService::explain_compilation` is now a library method (extracted
  from the MCP-only implementation, behavior byte-identical — the MCP tool
  delegates to it), exposed as `explainCompilation` on Node and
  `explain_compilation` on Python; `contextSavings` lands on Node; the WASM
  binding (and the TypeScript SDK wrapping it) gains `retrieveContextSource`
  over its in-memory, per-session store.
- **`velesdb-memory --version` / `-V`.** The MCP server binary now
  short-circuits the version flags before opening the store — a sanity
  check for a fresh install that previously had no CLI surface at all.
- **Path-referenced context fragments.** A `compile_context`/
  `explain_compilation` fragment may set `path` (an absolute filesystem
  path) instead of inline `content` to ingest a file by reference — exactly
  one of `path`, `content`, or `media` per fragment. Opt-in via
  `VELESDB_MEMORY_INGEST_ROOTS` (a `PATH`-list of allowlisted directories,
  parsed fail-fast at startup); the resolved file must be a plain UTF-8
  text file under 1 MiB, and the resolved content flows through the same
  pipeline as an inline fragment (dedup, classification, budget packing,
  `ctx://source/` handles).
- **`compile_transcript` MCP tool.** A one-call shortcut over
  `compile_context` for a raw agent-session transcript: deterministically
  segments it into turns (plain marker-based —
  `System:`/`User:`/`Human:`/`Assistant:`/`AI:`/`Tool:`/`### User`/
  `### Assistant` — or JSONL, one line per turn) and, within each plain
  turn, into fenced-code/log-run/body sub-segments (fenced code stays
  atomic; runs of 8+ log-like lines collapse the same way
  `abstract.log_dedup` would), then compiles the result exactly like
  `compile_context`. Accepts `transcript` (inline) or `path` (reusing the
  ingest allowlist, capped at a wider 8 MiB since the transcript is
  segmented into sub-1-MiB pieces immediately after being read). Returns
  the compiled context plus a `segmentation` audit report (detected
  format, one entry per segment with turn/role/kind/byte range/
  `fragment_id`, and how many segments normalization merged).
  **Node/Python bindings: follow-up.** `compile_transcript` is MCP-only in
  this release — neither `@wiscale/velesdb-memory-node` nor the Python
  `MemoryService` binding exposes a one-call convenience method yet; Rust
  and Node/Python callers compose `context::segment_transcript` +
  `compile_context` themselves in the meantime.

## [0.9.2] — 2026-07-20

### Added

- **Agentic quick wins for the MCP surface.** `get_info().instructions` now
  covers all three tool families (memory, context compiler, working-context
  resumption) instead of just memory. New `list_working_contexts` tool
  (per-project index, updated on every `save_working_context`) so an agent
  can discover resumable sessions instead of guessing a session id;
  `load_working_context`'s response gains `found` (explicit hit/miss) and
  `other_sessions` (surfaced on a miss, to recover from a session-id typo) —
  wire-additive, the existing `working` field is unchanged. `compile_context`
  gains `warnings[]` (a mechanical shortlist of externalized fragments
  relevant enough to the query to double-check) and `policy.slim_response`
  (empties `sections`/`decisions` from the response once auditing is done).
  New `suggest_budget` tool: a starting `token_budget` for a named model,
  from a static, committed model→window table (never a network call).

### Fixed

- **Memory-tool id strings now tolerate surrounding whitespace.** Follow-up
  to issue #1468/#1471: some MCP harnesses (Claude Code included) coerce any
  all-digit scalar back into a JSON number even when the client sends a
  string, which defeats the `id_str` string-id workaround and reintroduces
  precision loss above 2^53. A caller working around this by padding the id
  with whitespace (e.g. `" 12732540571541475285"`) was rejected by the
  string-or-number id parser used by `relate`/`forget`/`feedback` (and
  `Link.target`) with "expected a u64 number or a decimal u64 string" — the
  id string is now trimmed before parsing. The `+`-prefixed workaround
  (`"+12732540571541475285"`, already accepted since `u64::from_str` allows a
  leading `+`) keeps working unchanged.

- **`recall_where`'s type-strict comparisons are now documented (issue
  #1473).** Behavior is unchanged (no runtime coercion added): a numeric
  filter value never matched a string-stored metadata value, and vice
  versa, silently returning an empty set. The tool description and the
  velesdb-memory skill now say so explicitly and recommend storing
  comparable values (dates, counters) numerically.

- **Memory-tool ids now survive float-lossy JSON clients (issue #1468).**
  `remember`, `recall`/`recall_where`/`recall_fused`, `relate`, `forget`,
  `feedback`, `remember_extracted`, and `why` return `u64` ids as plain JSON
  numbers, which a client whose JSON layer round-trips numbers through an
  IEEE-754 `f64` (JS `number`, Claude Code included) silently rounds once the
  id exceeds 2^53 — the rounded id is then rejected by `relate`/`forget`/
  `feedback` with "memory does not exist", reported from real dogfooding.
  Every MCP response now also carries a decimal-string twin of each id
  (`id_str` on `remember`/`recall*`/`forget`/`feedback`/`why`'s nodes,
  `edge_id_str` on `relate`, `ids_str` on `remember_extracted`,
  `from_str`/`to_str` on `why`'s edges) — purely additive, the numeric field
  is unchanged so 0.9.x callers are unaffected. `relate`'s `from`/`to`,
  `forget`/`feedback`'s `id`, and `remember`'s `links[].target` also accept
  that decimal-string form on input (in addition to a plain number), with the
  advertised tool schemas updated to match, so a client can safely resubmit
  an `id_str` it received. **Wire-only, no Rust API change**: the string
  twins live entirely in the MCP DTO layer (`mcp::dto`); the public domain
  types (`Recollection`, `MemoryNode`, `MemoryEdge`, `Explanation`) are
  unchanged, so library consumers of the crate (bindings, crates.io users)
  see no breakage — the only `model` change is that `Link::target`
  *deserialization* additionally tolerates a decimal string, which is
  strictly widening. (#1468)

## [0.9.1] — 2026-07-19

### Security

- **Metadata is now size-capped.** Caller-supplied `metadata` on
  `remember`/`remember_with_ttl` and per-fragment metadata in the context
  compiler were unbounded — only fact/fragment content was capped — letting
  an arbitrarily large JSON blob be persisted as a DoS vector. Added
  `MAX_METADATA_BYTES` (64 KiB) and a typed `MemoryError::MetadataTooLarge`,
  enforced centrally so every adapter (MCP, Python, Node, WASM) picks it up
  through the existing error mapping with no adapter-side changes. (#1458)
- **Working context integrity.** `save_working_context` had no size guard,
  unlike every other stored fact (now capped at the same 1 MiB
  `MAX_FACT_BYTES` ceiling), and `load_working_context` skipped the
  reserved-marker check every other bridge-stored slot uses — a slot
  squatted by an unrelated or forged fact would be deserialized and served
  back as a working context. `load_working_context` now requires the
  `_veles_ctx_working` marker and returns `None` (not an error) for
  anything else. (#1458)

### Fixed

- **A permanent `ctx://source/` handle can no longer expire silently.**
  `store_context_sources()` unconditionally skipped an already-occupied
  slot, so a source first written under a TTL was never promoted when a
  later compile asked for permanent storage. Added a never-downgrade
  upgrade rule: permanent always wins over an existing TTL, a TTL never
  downgrades an existing permanent slot, and a TTL-to-TTL request only
  extends. (#1454)
- **Two byte-identical screenshots of the same target no longer both drop
  from compiled context.** Media dedup anchored on the first occurrence
  while screenshot supersession keeps only the last, so with two identical
  copies both were dropped instead of one surviving. Dedup now re-anchors
  onto the freshest non-superseded occurrence in the chain. (#1453)

### Added

- **Python**: `MemoryService.feedback` is now exposed, closing the RL
  feedback loop from the Python binding (previously MCP/Rust/Node only).
  (#1452)

### Documentation

- Per-surface parity for the context compiler is now stated honestly (MCP
  and Rust: full; Node: everything except `context_savings` and
  `explain_compilation`; Python: merged on `develop` but not yet in the
  published wheel; WASM: `compileContext` only). Fixed
  `retrieve_context_source`'s documented Python return type (`str` ->
  `dict`), harmonized the estimator over-count margins across docs to the
  numbers the `exact_estimator` harness actually produces, and clarified
  that images are never resized (oversized media is externalized behind a
  `ctx://source/<hash>` handle instead). (#1459)
- Documented a known limitation: the compiled-context cache prefix is
  byte-stable only while the compile `query` stays the same — under a tight
  budget, a query change can reorder competing cache-marked fragments
  (issue #1455). (#1456)
- Regenerated the billed A/B campaign on a new 19-turn vibe-coding
  scenario (cli runner, claude-sonnet-5, 5 runs/turn/arm, raw logs
  committed under `examples/real-session-benchmark/results/`): with
  screenshots, 10.9% billed dollars saved at unchanged answer adequacy
  (raw 22.8/23 vs compiled 23.0/23); without screenshots, 2.5% — the
  delta is the measured value of the media supersession/dedup mechanisms.
  The realistic metadata ceiling was also validated against the new 64 KiB
  cap (largest realistic fragment: 7% of the cap). (#1462)

### Changed

- CI now runs on `examples/**` changes; a test guards the generated Node
  `index.d.ts` against stale hand edits; four previously-unpinned
  context-compiler behaviors are now covered by regression tests. (#1456)

## [0.9.0] — 2026-07-18

### Added

- **Media source storage & screenshot supersession (complete as of PR3: MCP schemas, Node retrieve, WASM compile, TS types of
  US-009 in EPIC-P-071)** — the memory bridge now persists a media
  fragment's base64 payload alongside its caption when storing a compiled
  source (reserved key `_veles_ctx_source_media`, embedded with a
  deterministic bytes-hash-derived placeholder vector rather than the text
  embedder — `retrieve_context_source` resolves media sources by
  content-addressed hash only, never by vector search). A media fragment's
  handle — and its storage slot, still under the same salted system-fact
  namespace — is keyed on the **raw decoded bytes' hash** (the identity
  PR1's dedup already uses), never the caption text: two different images
  always get two different, independently resolving handles even with
  identical (typically blank) captions, while byte-identical images share
  one handle and resolve the same stored bytes with the kept instance's
  caption. Storage note: each distinct media source fact carries its full
  base64 payload — up to 4 MiB (`limits::MAX_MEDIA_BYTES`), above the 1 MiB
  `MAX_FACT_BYTES` ceiling which only guards MCP `remember`/`extract` text
  input — bounded per request by `MAX_MEDIA_BYTES`/`MAX_TOTAL_MEDIA_BYTES`
  and by `policy.source_ttl_seconds` over time. PR1's provisional
  `drop.media_unavailable` verdict is gone: a media fragment that cannot fit
  the budget now externalizes exactly like text (`budget.externalize`, a
  resolvable `ctx://source` handle), and a duplicate media fragment whose
  twin also failed to pack recovers through its own handle too.
  `MemoryService::retrieve_context_source` returns the new `ContextSource {
  content, media? }` shape (`media` is `#[serde(default)]`, so every
  pre-PR2 text-only source round-trips unchanged); the MCP
  `retrieve_context_source` tool result gained the same optional `media`
  field, and the Python `retrieve_context_source` binding now returns a
  dict instead of a bare string.
  **Screenshot supersession**: fragments sharing `media` + `kind:
  "screenshot"` + the same `metadata.target` value are a succession
  series — only the LAST one (input order, no clock) stays inline
  (`Preserve`, budget permitting); every earlier one is proactively
  reclassified `retrieve.screenshot_superseded` and externalized behind a
  resolvable handle, regardless of budget, with an explicit reason. A
  screenshot with no `metadata.target` is never superseded (no target is no
  evidence of succession). Opt out per request via
  `policy.disabled_rules: ["retrieve.screenshot_superseded"]`. Byte-compat:
  a request with no media is unaffected.
- **Media fragments — foundational primitive (PR1 of 3 for US-009 in
  EPIC-P-071; wired end-to-end by the entry above)** —
  `ContextFragment.media: Option<MediaRef>` lets a fragment carry an inline
  base64-encoded image (`mime` + `bytes_b64`) alongside its text/caption
  `content`. A media fragment packs as one atomic, unsplittable piece (never
  chunked mid-image), is deduplicated on its *raw decoded bytes* (never the
  caption text, and never near-duplicated), and its token cost comes from a
  new dependency-free `ImageTokenEstimator` (PNG/JPEG header dimensions,
  `ceil(width * height / 750)`; unsupported mimes or unreadable headers fall
  back to a safe text-based over-count). Capped at 4 MiB of base64
  (`limits::MAX_MEDIA_BYTES`), separate from the existing per-fragment text
  cap; malformed base64 is rejected at validation time. Wire-compatible:
  `media` is `#[serde(default)]`, so every existing request still
  deserializes unchanged.
- **Usage-driven importance blend in `context_memories`** —
  `CompilePolicy.importance` (`{confidence: 0.2, recency: 0.1,
  recency_field: null}`, serde-defaulted so 0.8.0 requests stay
  wire-compatible) folds the RL confidence `feedback` trains and a
  batch-relative recency term into the fused ranking of pulled memories:
  `fused_norm + w_c·(confidence−0.5)·2 + w_r·recency_norm`. Applies only to
  the similarity-selected pool (confidence is not relevance — an adversarial
  test pins that an over-reinforced off-topic fact never enters), reads no
  clock (recency is min-max normalised within the batch; an absent key or a
  degenerate batch contributes 0), composes with the
  `compile_context_reranked` seam, and ventilates all four signals in each
  decision's `reason` (`vector …, graph …, confidence …, recency …`). Both
  weights at 0 reproduce the 0.8.0 output byte for byte (golden-pinned).
  **Behavioral change**: with the default policy the blend is active
  (`confidence: 0.2`), so RL-reinforced memories now rank higher out of the
  box after an upgrade; set the importance weights to 0 to restore the exact
  0.8.0 ordering (byte-identical, golden-tested). Recommended weight range
  is `[0, 1]`; out-of-range values are accepted verbatim, never clamped
  (documented and regression-tested). [EPIC-P-071/US-002]
- **Node** (`@wiscale/velesdb-memory-node`): `feedback(id, success)` binding
  (resolves to the fact's new learned confidence), and a committed RL×graph
  synergy case in the `tri_engine_rescue` benchmark: a fact reinforced by
  `feedback` and reachable only through the typed-edge walk out-ranks a
  merely-similar fact once `policy.importance` is active — reproducible
  across runs. [EPIC-P-071/US-002]
- **Benchmark**: `examples/context_savings`, reproducible (75–82 % estimated
  token savings on the committed corpus in ~2 ms; figures are local
  estimates, not billed tokens — cross-checked against a real cl100k
  tokenizer by the committed `real_measures/` scripts).
- **MCP**: two working-context tools on the one existing server —
  `save_working_context` / `load_working_context` (pure delegation to the
  memory bridge), so an agent can persist its distilled session state and a
  later session can resume from it; the committed `mcp_e2e.py` harness
  proves the round-trip **across two separate server processes** on one
  store. [EPIC-P-071/US-003]
- **Node** (`@wiscale/velesdb-memory-node`): `saveWorkingContext` /
  `loadWorkingContext` — same wire shape, ids as decimal strings in both
  directions (u64::MAX-safe), `null` when nothing was saved; the spec suite
  proves the cross-process round-trip via a child-process save.
  [EPIC-P-071/US-003]
- **`velesdb-memory`**: `CompilePolicy.normalize_log_timestamps` (default
  `false`, serde-defaulted so existing requests stay wire-compatible) — an
  opt-in, deterministic mask of `kind: "log"` fragments' volatile prefixes
  (ISO/syslog timestamps, bracketed hex/pid counters, fixed patterns only)
  applied before `abstract.log_dedup` groups repeated lines, so lines
  identical modulo timestamp now collapse; the emitted line is still the
  first occurrence's exact bytes, and the decision `reason` says so when
  normalization actually changed the grouping. Off by default: byte-exact
  grouping is unchanged for existing callers (pinned by a golden test).
  [EPIC-P-071/US-006]
- **Proof harness**: `examples/context_savings/real_measures/cache_prefix.mjs`
  measures the `cache: true` section's byte-stable-prefix percentage across
  10 consecutive compiles with changing volatile content (100 % stable
  prefix on all 9 consecutive turn pairs, reproducible) and frames the
  naive full-input-rate cost of not caching it against an injected,
  never-hardcoded `policy.pricing` table. [EPIC-P-071/US-008]
- **Proof harness**: `examples/node-llm-middleware/` — a minimal
  middleware wrapper measuring `compile_context` savings offline (real
  cl100k via `gpt-tokenizer`, always) and, opt-in
  (`RUN_BILLED_MEASURE=1` + an API key never asked for by the harness), the
  provider's own billed `usage` on a real minimal-cost call.
  [EPIC-P-071/US-007]
- **MCP**: `CompilePolicy.ids_as_strings` (default `false`) — opts the
  `compile_context` / `explain_compilation` response into decimal-string ids
  (`fragment_id`, `content_hash`, `memory_id`, `fragment_ids`), reusing the
  same tree walk the Node/WASM bindings already apply, for raw MCP clients
  without u64-safe JSON number parsing. `fragments[].id` on input now also
  accepts either a JSON number or a decimal string, and the advertised tool
  schemas type every such field `["integer", "string"]` so schema-validating
  clients accept the opt-in form. [EPIC-P-071]
- **MCP**: `explain_compilation` gains an optional `fragment_index` (0-based
  position in `request.fragments`), so byte-identical fragments — which
  share a content-addressed `fragment_id` — can be disambiguated instead of
  always resolving to the deduplication survivor's decision. Default
  behavior (no `fragment_index`) is unchanged. [EPIC-P-071]
- **Benchmark**: `examples/real-session-benchmark/` — realistic agentic
  sessions (screenshots with US-009 dedup/supersession, CI logs for
  `normalize_log_timestamps`, re-injected docs, re-read code files) run A/B:
  raw ("vraie vie", everything resent every turn) vs compiled
  (`compileContext`; the compiled arm is billed for the `ctx://source/`
  handles it sends). OFFLINE (default; real cl100k tokenizer + the API's own
  pixels/750 image-token formula; every variant reproduced twice,
  byte-identical) measures, on the committed corpora: **17.2%** saved on the
  base 14-turn session in lossless mode (pure redundancy elimination, zero
  unique information removed — externalized: 0), **20.3%** in
  window-enforcement mode (budget 8000; the extra ~3 points explicitly
  attributed to `budget.externalize` of unique content, not redundancy),
  **30.9%** lossless / **55.1%** windowed on the 36-turn long-session
  variant (with per-arm context-growth curves and labeled projected headroom
  to a configurable compaction threshold: raw ~234 tokens/turn vs compiled
  ~35/turn), and **18.4%** on the memory-enabled variant (docs stored once
  via `remember`/`relate`, pulled back per turn via the default
  `memory_scope` — the product's intended pattern, untuned k=5). ONLINE
  (opt-in, `RUN_BILLED_MEASURE=1` + `CONFIRM_SPEND=1`) bills the same
  session for real on `claude-sonnet-5` — reading the provider's own
  `usage.input_tokens` with cache fields reported separately — AND grades
  real generated answers in both arms against committed per-turn fact
  checklists (deterministic substring grader): token savings and answer
  adequacy are reported side by side, so a saving that costs answers is a
  reported failure. Runners: native `fetch` (`ANTHROPIC_API_KEY`) or the
  Claude Code CLI's headless mode (`BENCH_RUNNER=cli`, no key — the user's
  own authenticated account; wire shapes verified by a real calibration
  call). [EPIC-P-071]
- **CI gate — ground-truth facts survive compilation** (EPIC-P-071/A1):
  `examples/real-session-benchmark/test/facts-survive.test.mjs` turns the
  benchmark's per-turn fact checklists (`corpus/questions.mjs`) into an
  executable non-regression check: for every turn of the base session, in
  BOTH the lossless and the window-8000 compiled arms, every ground-truth
  fact must be present in what that arm would actually send to the model —
  inline, or PROVEN recoverable by really resolving its `ctx://source/`
  handle via `retrieveContextSource` (never assumed from a listed handle).
  Runs offline, no network, in CI's `Node Binding Tests` job (reuses the
  napi addon already built there). [EPIC-P-071]

### Changed

- **`forget` now reports whether the id actually existed** on every surface
  (Rust bridge → `bool`, MCP `{found}`, Node/WASM/TS `boolean`,
  Python `bool`): deleting an unknown id used to read as success, so an
  agent could not tell a real deletion from a typo'd or stale id. Wire-compatible
  everywhere (the MCP result gains an additive `found` field); the Node
  typings and the TS SDK's `forget` widen `Promise<void>` → `Promise<boolean>`
  — only a caller with an explicit `: Promise<void>`/`: void` annotation on
  the result needs a touch.
  [EPIC-P-071/US-004]

### Fixed

- **MCP server hardened against a leaked client process (#1448)**. The
  server itself was already healthy (it exits cleanly on stdin EOF), but a
  client that leaks its child process — observed in practice with a
  headless `claude -p` run — never closes stdin, so the server correctly
  kept serving forever and held the store's single-writer lock, making every
  later session fail with an opaque `Storage (DatabaseLocked)` / "Failed to
  connect". Two defensive fixes: (1) the server now detects a dead parent
  (`std::os::unix::process::parent_id()` polled every ~2s, Unix-only, no new
  dependency) and self-exits, releasing the lock, even when stdin is
  artificially held open; (2) a `DatabaseLocked` at startup now retries
  briefly (3 × 500ms, covering a normal close/reopen handover) and, if the
  store is still locked, prints an actionable message on stderr naming the
  fix (`pkill velesdb-memory` or set `VELESDB_MEMORY_PATH` elsewhere)
  instead of a bare error dump — and exits non-zero so client health-checks
  can detect the failure. Net effect: one leaked client can no longer brick
  every later session, and when a store really is locked, the user is told
  what to do about it.

## [0.8.0] — 2026-07-17

Retroactive cut — this release shipped without its own section here (its
full detail lives in the workspace-root CHANGELOG under "EPIC-P-070").

### Added

- **The deterministic context compiler** (`context` feature, on by default):
  `compile_context` / `retrieve_context_source` / `explain_compilation` /
  `context_savings` over MCP, plus the memory bridge (`memory_scope`
  tri-engine pull, content-addressed recoverable sources, compilation
  events) and `save_working_context`/`load_working_context` on the bridge.
  No LLM, no network, no clock: same request ⇒ byte-identical output.
- Node binding `@wiscale/velesdb-memory-node` 0.8.0 (`compileContext`),
  bundling the `velesdb-context-optimizer` agent skill.

## [0.7.0] — 2026-07-15

Retroactive cut — versions realigned with the workspace release train; no
crate-level feature change beyond dependency refreshes.

## [0.6.0] - 2026-07-06

### Changed

- Richer MCP tool descriptions and parameter docs for `relate` and `forget`
  (when to use them, directionality, examples, durability) — improves the
  schema quality MCP clients and directories surface.

## [0.5.0] - 2026-07-06

### Added

- **`format_dated_context` / `DatedContext` (new `dated_context` module)** —
  formats recalled facts into a chronological, "now"-anchored timeline for dated
  recall; the primitive behind `recall_fused`'s `date_field` (MCP/Python) and
  `recallFusedDated` (Node/WASM/TypeScript SDK). (#1315, #1316)
- **Node binding `recallFusedDated`** — fused recall returning the dated timeline
  plus a `now` anchor in a single call. (#1316)

## [0.4.0] - 2026-07-03

### Added

- **Fused vector+graph recall (`recall_fused` / `recall_fused_reranked`)**:
  vector recall combined with the graph reach `why()` walks, re-ranked with
  the entity-idf weighting validated on HotpotQA/TimeQA/LoCoMo. Exposed on
  the Node binding as `recallFused` (with `{hops, graphBoost, pool}` options,
  all DoS-clamped). Optional second-stage re-ranking via a bring-your-own
  `Reranker`.
- **Every recall path now includes the fact's caller-supplied metadata
  (`Recollection.metadata`)** — `recall`, `recall_where`, and `recall_fused`
  alike — enabling dated/chronological recall recipes (see
  `docs/guides/TEMPORAL_MEMORY.md`). Reserved system keys are never exposed.
- **Pluggable storage backend (`MemoryStore` trait)**: the wedge
  orchestration is now generic over its storage, with the native file-backed
  engine as the default `NativeStore` (existing callers see no change) and
  `velesdb-wasm` providing an in-memory backend so the full wedge runs in
  the browser. `persistence` becomes an optional, default-on feature.
- New `MemoryError::RollbackFailed` variant: a `remember` whose edge write
  failed after the fact was stored AND whose compensating delete also failed
  now reports both errors instead of silently keeping the fact.

### Fixed

- `recall_fused`'s metadata `filter` is enforced on graph-reached facts, not
  just the vector seed — a fact outside the caller's scope (e.g. another
  tenant) can no longer leak in through a graph connection.
- Score normalisation no longer sign-inverts a negative (in-range Cosine)
  vector score into an unbounded magnitude that dwarfed the whole ranking.
- The fused pool depth is DoS-clamped at the crate level (the default
  `k × 8` was previously unbounded), and metadata lookups across
  `recall`/`recall_fused` are batched into single storage round trips.
- An empty-but-present filter (`{}` at a JS boundary) now behaves exactly
  like no filter: entity hubs stay excluded from `recall`, `why`, and
  `recall_fused`; `recall_where` with no predicates routes through `recall`.
- `remember` validates all link input (targets AND relation labels) before
  any write, and rolls back a freshly-created fact if an edge write fails —
  a failed call can no longer overwrite a pre-existing fact's metadata or
  arm a TTL on a permanent memory.

## [0.3.1] - 2026-06-30

### Security

- **Upgraded `rmcp` 1.8.0 → 2.0.0**, which patches three advisories: OAuth token
  spoofing, SSRF via crafted MCP URLs, and a session-id leak in error responses.
  No code changes were needed — the MCP server/macros API stayed compatible.

### Fixed

- **`truncate()` UTF-8 panic** (extract error previews): the budget is now checked
  *before* appending a word, dropping the post-hoc `String::truncate` that could
  panic when the 120-byte limit fell mid-multibyte-character.
- **Dead code in `validate_relation`**: removed the redundant `is_ascii()` guard
  (`char::is_ascii_control()` is already `false` for non-ASCII code points).

## [0.3.0] - 2026-06-30

### Added

- **Durable TTL on `remember`.** Facts can now expire. `remember` (MCP tool) and
  `MemoryService::remember_with_ttl` take an optional `ttl_seconds`; the expiry is
  persisted with the fact (`_veles_expires_at`), so it survives a restart, and
  expired facts stop being recalled. Metadata and a TTL combine. Set a server-wide
  default with `VELESDB_MEMORY_DEFAULT_TTL` (seconds); `0` means permanent. The
  Node binding's `remember` gains the matching `ttlSeconds` argument.

### Fixed

- **Cleaner MCP tool schemas.** Stripped `schemars`' non-standard integer `format`
  keywords (`uint64`/`uint`) from the generated tool schemas, so strict MCP clients
  no longer log `unknown format "uint64" ignored` for every id field.

## [0.2.0] - 2026-06-29

Benchmark milestone: the tri-engine is no longer just *wired* — each leg is
*measured* to beat pure-vector retrieval on its specialty, generation-free, on
public/real data, and the engines are shown to compound.

### Added
- **Generation-free retrieval benchmarks** isolating each engine's contribution,
  reproducible from bundled examples (`examples/{multihop,timeqa,colfilter,triengine}`):
  - **Graph (`why()` BFS) — multi-hop supporting-fact recall.** On **HotpotQA**
    (3 000 dev, distractor) fused vector+graph lifts supporting-fact recall
    **+3.3pp** overall and **+5.6pp** on retrieving *both* bridge facts, with an
    idf-weighted bridge that suppresses the flooding a naive boost causes. The
    win **replicates on a second independent dataset, 2WikiMultiHopQA**,
    concentrated on the genuinely multi-hop question types.
  - **ColumnStore (`recall_where` numeric range) — time-scoped recall.** On real
    **TimeQA** Wikipedia bios, the year-range predicate lifts gold-sentence recall
    **+9.7pp** (+18.6pp on a controlled synthetic pilot) where cosine alone cannot
    disambiguate candidates that differ only by a number.
  - **Tri-engine compounding capstone** (`examples/triengine`): on a task that is
    multi-hop *and* time-scoped at once, Graph and ColumnStore together lift recall
    more than the sum of their individual gains — the engines stack.
- **LoCoMo harness** (`examples/locomo/`) extended into a tuning/diagnostic
  workbench: retrieval-only and explanation modes, per-category diagnostics, BM25
  baseline, idf-weighted graph fusion, date-context/date-routing and a temporal
  scaffold, an optional Claude judge/generator, and a configurable evidence budget.
- Positioning and benchmark write-ups (`POSITIONING.md`, `BENCHMARK.md`) grounding
  every claim in a reproducible measurement, with each engine's honest limit
  disclosed.

### Notes
- No public API change — this release adds benchmarks, examples and documentation
  around the existing `MemoryService` / MCP surface introduced in 0.1.0.

## [0.1.0] - Unreleased

First release of the local-first MCP memory server for AI agents.

### Added
- MCP tools over stdio mapping onto VelesDB's in-core Agent Memory SDK:
  `remember`, `recall`, `recall_where` (fused vector + ColumnStore range/filter
  recall), `relate`, `forget`, `why` (vector recall + multi-hop graph traversal —
  the connected-subgraph differentiator), and `remember_extracted` (auto text →
  fact↔topic graph via an `Extractor`).
- The same high-level `MemoryService` is consumed beyond the MCP server by the
  Python binding (`velesdb-python`) and the Node.js binding (`velesdb-node` /
  `@wiscale/velesdb-memory-node`); the library is feature-gated (`default-features
  = false` drops the rmcp/tokio MCP stack) so bindings link a lean core.
- `recall_where` activates a secondary bitmap-prefilter index on first use, so
  filtered recall stays flat as the collection grows (instead of an O(n) scan).
- Pluggable embeddings: a deterministic, offline `HashEmbedder` by default and an
  optional on-device `OllamaEmbedder` (`--features ollama`).
- Structured metadata (ColumnStore facet) with exact-match filtering on `recall`
  and `why`.
- Input guards (max fact size, capped recall limit and hop depth) and clean
  MCP error-code mapping: client-input errors map to `invalid_params`, faults to
  `internal_error`. `relate` validates both endpoints exist up front, so an
  unknown id is reported as `invalid_params` (not an internal fault) and the
  graph never gains an edge dangling off an unstored memory.
- License boundary by construction: memory semantics only, never raw database
  capabilities.

[0.3.1]: https://github.com/cyberlife-coder/VelesDB/releases/tag/velesdb-memory-v0.3.1
[0.3.0]: https://github.com/cyberlife-coder/VelesDB/releases/tag/velesdb-memory-v0.3.0
[0.2.0]: https://github.com/cyberlife-coder/VelesDB/releases/tag/velesdb-memory-v0.2.0
[0.1.0]: https://github.com/cyberlife-coder/VelesDB/releases/tag/velesdb-memory-v0.1.0
