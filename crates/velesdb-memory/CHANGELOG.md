# Changelog — velesdb-memory

All notable changes to the `velesdb-memory` crate are documented here. This
crate is versioned independently of the VelesDB workspace (0.x cadence) and is
released on its own `velesdb-memory-vX.Y.Z` tag.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Daemon-owned online embedding migration (#1796).** Four MCP tools now
  start, inspect, cancel, and recover a background re-embedding without
  opening the live source from a second process. A bounded, checksummed dirty
  journal captures every mutation before it reaches the source; catch-up
  reports convergence and refuses an unsafe cutover; the exclusive live
  generation switch enforces the operator's pause budget and recovers across
  either rename. Durable job state, epoch/model witnesses, verified
  cancellation cleanup, process-level concurrent-write coverage, and measured
  no-capture/capture overhead complete the operator contract.

- **Role-named HTTP inference features (#1766).** `embedder-http` enables the
  Ollama and OpenAI-compatible embedding backends; `extractor-http` enables
  their extraction counterparts. The former `ollama` and `extract` features
  remain aliases for compatibility with existing consumers.

- **`openai`: an OpenAI-compatible backend for BOTH roles (#1751).** Set
  `VELESDB_MEMORY_EMBEDDER=openai` or `VELESDB_MEMORY_EXTRACTOR=openai` to
  reach oMLX, llama.cpp's server, LM Studio, vLLM or a hosted provider. The
  value names a **protocol, not a vendor**: reaching a different server is a
  different URL, never a new backend name — which is what stops the selector
  from growing a vendor list.

  Neither the URL nor the model has a default here, deliberately: guessing
  either would pick one of those servers for the operator. Both roles are
  configured independently — embedding on a local Ollama while extracting on
  an OpenAI-compatible server is a supported combination.

  The daemon now dispatches on the backend name the library hands it. It
  previously matched `NeedsRemoteConfig(_)` and built an Ollama client
  regardless, in **two** places (`attach_extractor` and the autograph path),
  so a second backend would have been selected and then silently ignored —
  asking for `openai` answered `VELESDB_MEMORY_EXTRACTOR=ollama requires …`.

- **Role-named configuration for the embedding role.** `VELESDB_MEMORY_EMBEDDER_URL`,
  `_MODEL` and `_API_TOKEN` mirror the extraction role's variables, which were
  role-named from the start. `VELESDB_MEMORY_OLLAMA_URL` / `_MODEL` keep
  working as aliases, so no existing setup changes; when both names are set to
  **different** values the role-named one wins and the daemon says so **once**
  at startup. `velesdb-memory.toml`'s `[embedder]` section now writes the
  role-named variables.

- **API tokens, environment-only.** `VELESDB_MEMORY_EMBEDDER_API_TOKEN` /
  `VELESDB_MEMORY_EXTRACTOR_API_TOKEN`. No token means **no `Authorization`
  header at all** — not an empty one, which a server rejects as a bad
  credential rather than a missing one; a token set to an empty string is
  refused at startup for the same reason. There is deliberately **no
  `api_token` field in the config file**, and one written there is refused:
  a credential at rest in a versionable file is one `git add .` away from a
  public history. The refusal is rewritten rather than passed through, because
  the TOML parser quotes the offending line back — which would have printed
  the secret to stderr.

- **The store records which embedding model filled it.**
  `embedding-provenance.json`, beside the store. `velesdb-core` already refuses
  a collection whose dimension differs, which catches the loud half of the
  problem; two different models of the **same** width open fine and return
  noise (this crate's `hash` embedder is 384-dimensional, and so is
  `all-minilm`). The **backend is not recorded** — it is a transport, and the
  same model served by Ollama, oMLX or a hosted API produces the same vectors,
  so refusing on it would block a valid migration. The record is written only
  for a store holding **no facts**, never over existing data where one open
  with the wrong model would carve a provenance every later check would trust.
  A store created before this stays unrecorded, and its check degrades to the
  dimension alone — saying so explicitly rather than letting a successful open
  read as a verified match. Deleting the file is safe; the store is untouched.

- **`OutlineExtractor`: a deterministic, network-free extraction backend.**
  Until now `OllamaExtractor` was the crate's only `Extractor`, so every
  contract `remember_extracted` publishes was reachable only through a
  network call — which is why `skipped_over_cap` and the incoming half of an
  entity profile had no test on any binding, and were declared known gaps
  instead. This backend reads the structure a passage STATES rather than
  inferring it, one directive per line:

  ```text
  edge: Camille | sister of | Theo
  attr: Theo Durand | age | 15
  fact: Camille ships the parser. | camille
  ```

  It stands to `OllamaExtractor` exactly as `HashEmbedder` does to
  `OllamaEmbedder`: a public, documented, offline choice — not a test double.
  A malformed directive is an `ExtractError::Parse`, never a silently dropped
  line. Selected by name through the new `extractor` parameter on the
  bindings' `remember_extracted` (`"ollama"` by default, `"outline"` for this
  one); an unknown name is refused rather than silently substituted.

- **WASM gains `rememberExtracted`.** Its exemption in the parity guard was
  justified by `OllamaExtractor` being the crate's only `Extractor` — a
  reason the backend above annuls. Note that nothing mechanical caught this:
  the stale-exemption check asks whether a binding started publishing the
  tool, never whether the exemption's REASON is still true.

- **TypeScript SDK gains `entity`, `unrelate` and `rememberExtracted`**, the
  methods it had been missing for its whole life (#1721), and enters a
  guard's perimeter for the first time.

### Fixed

- **`why`/`recall_fused`'s graph walk had no width budget: a hub could dump its
  entire neighborhood, full fact content included, into one response (#1743).**
  The only existing guard, `MAX_WHY_HOPS`, bounds traversal *depth* — its own
  comment claimed it "prevents exponential graph fan-out", which was false: an
  entity hub is a super-node by construction (degree scales with the whole
  store), so a single hop through one could still return thousands of
  full-content nodes. Two width caps now bound the walk directly:
  `MAX_WHY_NODE_DEGREE` (64) limits how many outgoing edges are followed from
  any one node, and `MAX_WHY_NODES` (500) caps the total nodes a walk collects
  across every hop. Both apply to `why` and `recall_fused` alike — they share
  the same internal traversal. `MAX_WHY_HOPS`'s comment now says what it
  actually bounds. Review hardening on the same fix: the node ceiling is now
  exact — enforced at the push site, where checking only between expansions
  let the crossing expansion overshoot to a measured 522 of the documented
  500 — and `MAX_WHY_EDGES` (2000) caps the edges a walk records, the half of
  #1743's ask ("nodes AND edges") the first cut left unbounded: 60
  fully-connected nodes returned 3 540 edges against a node count of 60.

- **`recall_fused`'s graph reach is no longer quadratic in a hub's fan-out
  (#1742).** `reach_weight` used to rescan every edge the traversal collected,
  once per reached node, to find the `mentions` edges pointing at it —
  O(hub degree) work repeated for each of O(hub degree) nodes. A user
  accumulating facts about the same entity (the product's nominal use case,
  not an edge case) paid a cost growing with the square of their history on
  that topic. `mentions` edges are now indexed by target once per call
  (`fact_id -> [hub_id, ...]`), turning the whole pass into O(edges + nodes).
  Weighting is unchanged — a fact reached through several hubs still ranks by
  the rarest (highest-idf) one — locked in by a new test
  (`recall_fused_weighs_a_dual_hub_fact_by_its_rarest_hub`) and tracked going
  forward by `benches/fused_recall_benchmark.rs`.

- **TypeScript SDK: `CompileContextFragment` gains `priority`.** The wire has
  accepted it since the context compiler shipped, and this SDK never declared
  it — so a TypeScript caller could reach `compileContext` and not express the
  one input that controls what a tight budget drops. Found by deriving both
  field lists from source rather than reading either one.

  A new parity check in `tests/binding_parity_bdd.rs` holds the SDK's fragment
  against the canonical `ContextFragment`, so a field added to one and not the
  other turns red. Method parity already proved a tool was *reachable*; this
  proves its *input* can be expressed.

  **`path` stays undeclared, on every binding, by decision.** Resolving a path
  fragment is a server-side I/O pre-pass gated on `VELESDB_MEMORY_INGEST_ROOTS`,
  an operator-configured allowlist — the WASM binding has neither a filesystem
  nor that setting, and neither `velesdb-node` nor `velesdb-python` resolves
  paths. Declaring it would publish a field that always fails. The exemption is
  written down next to the check, with its reason, because an absence by
  decision and an absence by oversight look identical from the outside.

- **A base URL carrying the `/v1` prefix no longer doubles it (#1751).**
  Servers advertise their OpenAI-compatible endpoint *with* the version prefix
  — oMLX's console shows `http://127.0.0.1:8019/v1` beside a copy button — and
  concatenating `/v1/embeddings` onto that produced `/v1/v1/embeddings` and a
  `404` whose cause was invisible from the message. Both spellings now reach
  the same endpoint. A trailing `/v1` is stripped only when something precedes
  it, so a host genuinely named `v1` is not truncated to `http:/`.

### Changed

- **BREAKING (MCP) — `remember_extracted` is now a durable asynchronous job
  (#1839).** It returns `{request_id, state, reused}` immediately after an
  `accepted` record is synced, rather than holding the transport open across
  model generation and graph writes. Callers should supply one
  `idempotency_key` per logical operation and poll the new
  `extraction_status` tool for `{state, ids, ids_str, skipped_over_cap,
  error}`. Identical retries reuse the persisted receipt; a changed payload
  under the same key is rejected. Accepted/running jobs recover after restart,
  and the generated extraction is persisted before writes so an interrupted
  commit replays stable output. The Rust `MemoryService` and language-binding
  methods remain synchronous and keep their existing return envelopes.

- **BREAKING — `remember_extracted` returns an envelope**, `{ids,
  skipped_over_cap}` (`{ids, skippedOverCap}` on the JS surfaces), where Node
  returned `Array<string>` and Python `List[int]`. **Migration**: read
  `.ids` / `["ids"]`. A bare list could not say why it was short — nothing
  distinguished a passage holding three facts from one holding twelve of
  which nine were dropped for exceeding the embeddable cap. That is a silence
  about lost data, not a missing convenience (#1692).

- **BREAKING (Python) — `model` is now optional** on `remember_extracted`,
  since it configures the `"ollama"` backend only. Selecting `"ollama"`
  without one raises `ValueError` naming the alternative.

- **`entity` relays `relations_in` on all three bindings** (#1690). Without
  it a question was only answerable from one side: the graph holds
  `camille --sister of--> theo`, so reading Theo's outgoing edges never found
  Camille. WASM's `EntityProfileOut` gained `rename_all = "camelCase"` in the
  same change — a no-op on its five single-word fields, and the only way
  `relationsIn` does not cross as a snake_case key beside the `targetId` of
  the object inside it.

- **The Node binding relays `compile_context.warnings`** (#1691), and
  `compiled_envelope` now drains its input and asserts nothing was left
  behind. The comment that used to bless the loss — "the envelope is the
  binding's contract, not a mirror of the domain type" — was the reading that
  caused it, and is rewritten.

- **All six `KNOWN_GAP` entries are gone** from `SHAPE_DIVERGENCES`, deleted
  by the fixes above; it now holds deliberate unwraps only. The constant
  itself is kept, unused, so the next honest admission has wording to reach
  for. Worth stating plainly: nothing caps how many gaps may be declared, and
  adding a seventh would have been exactly as green as fixing six.

## [0.12.0] - 2026-07-30

### Changed

- **BREAKING — `load_working_context` returns the same three-field envelope on
  every surface.** The MCP tool has served `{found, working, other_sessions}`
  since V2a-1. The three bindings — Node `loadWorkingContext`, Python
  `load_working_context`, WASM `loadWorkingContext` — and the TypeScript SDK
  returned only the working context, or `null`/`None`. They now return the
  whole envelope.

  **Migration**: read `.working` (JS/Rust) or `["working"]` (Python) where you
  used the returned value directly. A `null`/`None` check becomes a `found`
  check.

  Why this is worth a breaking change rather than a new method: the bare form
  gave one answer to two different questions. "Nothing was ever saved here"
  and "you typed `task-1235`, and `task-1234` is right there" arrived
  identical, so an agent resuming a session silently started over on top of
  work that existed. `other_sessions` names the near-misses, and it is filled
  in on a **hit** as well — a typo that lands on another real session returns
  `found: true`, which is the case a caller can least detect on its own.

  The drift was pre-existing, not introduced here: the bindings, the declared
  type stubs, the guides, the READMEs, the LangGraph tool docstring rendered
  to a model, the three agent-hook scripts injected into a model's context,
  and the ready-made `AGENTS.md` block in `integrations/agent-hooks/codex/`
  all announced `WorkingContext | null`. Every one found is corrected — that
  `AGENTS.md` block last, because the first sweep listed only files naming the
  tool and missed prose describing its RESULT.

  A count is not a guard, so two of those surfaces are now swept mechanically:
  `integrations/agent-hooks/test/hooks.test.sh` fails on any `.sh` or `.md`
  under that tree that still tells a model to expect a null result, and
  asserts the three envelope field names inside each injected context. It
  previously asserted only that the tool's NAME appeared — which the stale
  wording satisfied exactly as well as the correct one.

### Added

- **`MemoryService::resume_working_context(project, session)`** — composes
  `load_working_context` + `list_working_contexts` and owns the three policy
  rules in one place: list on a hit too, never re-emit the requested session,
  and treat an unreadable index as fatal on a MISS but survivable on a HIT.
  Every surface calls it; nothing recomposes the envelope.

  That third rule matters because reading the index is new work on this path
  for the three bindings, which previously only read the session's own fact.
  Propagating its failure unconditionally would have made a corrupt project
  index deny resumption for **every** session of that project, including the
  many that read back perfectly — a fault in an auxiliary hint costing the
  answer the caller actually asked for. On a hit the hint therefore degrades
  to an empty list; the corruption stays reachable through
  `list_working_contexts`, published on every surface. On a **miss** it stays
  fatal: there, `other_sessions: []` is the positive claim "nothing else was
  ever saved here", and an agent told that starts over on top of live work.
- **`LoadedWorkingContext`** in `velesdb_memory::context`, re-exported from
  `context.rs`. The envelope type used to be `pub(super)` inside the
  `mcp` module — a Cargo feature the bindings do not enable — so it could not
  be relayed at all.
- **Shape parity is now guarded, not just method-name parity.**
  `tests/binding_parity_bdd.rs` reads each tool's `output_schema` ROOT KEYS
  off the LIVE server and requires every binding to relay them — by naming the
  server's own output type, by naming the field, or by declaring the drop in
  the new `SHAPE_DIVERGENCES` table (twin of `EXEMPTIONS`, with the same
  staleness check). The old guard compared method NAMES only, which is why
  this defect lived: the method was there, under the right name, and nothing
  looked at what it returned. The guard's limit is written in the file's
  header — it is a text search over source, so it proves DECLARATION, never
  MARSHALLING.

  It found three further divergences on its first run, declared as known gaps
  (six entries, since two of them span more than one binding): all three
  bindings drop `entity.relations_in` (added server-side by #1681), the Node
  binding's `CompiledContextJs` drops `compile_context.warnings`, and both the
  Node and Python bindings drop `remember_extracted.skipped_over_cap`. None is
  a deliberate unwrap — each one really does lose the field.

  For `load_working_context` the text search is **not** accepted as proof.
  Every one of the three bindings satisfied it through prose alone — two
  doc comments and a `ts_return_type` string — on the one tool whose silent
  shape drift is the reason the guard exists; each of those bindings carried
  such a comment throughout the months it was broken, so a text search would
  have been green the entire time. `envelope_tools_are_relayed_by_type_never_
  by_prose_alone` now requires the strong route for it: each binding binds the
  relayed value as `let loaded: LoadedWorkingContext = …`, which the compiler
  enforces and prose cannot fake.

- **Shape-drift detection across package boundaries.** Presence checks
  (`ensureCapability` in the TS SDK, `hasattr` in the LangGraph toolkit) prove
  a method EXISTS; they cannot see that an older resolved build returns the
  pre-envelope shape. Both dependency floors admit exactly such builds, so the
  skew is reachable by an ordinary install. Each surface now inspects the
  returned value and fails loudly — a `ConnectionError` naming the cause in
  the SDK, an `{"error": …}` tool payload in LangGraph — instead of casting
  the bare form into the new type, where `found` reads as `undefined`/`None`,
  is falsy, and sends the agent back to a fresh start on top of live work.


### Fixed

- **`explain_compilation` refused the `fragment_id` it had just emitted.**
  `compile_context` rewrites every id of its response into a decimal string
  when the request carries `policy.ids_as_strings` — the option that exists
  so a float-lossy JSON client keeps its ids intact. `explain_compilation`,
  whose whole job is to explain the decision behind one of those ids, took
  `fragment_id` as a strict `u64` and rejected the string with
  `invalid type: string "…", expected u64`. The fallback was no better: a
  `fragment_id` is an FNV-1a 64 content hash, so it is past 2^53 in ~99.95 %
  of cases and a JavaScript client has already rounded it on arrival. Both
  forms failed, which made the tool unreachable by its own documented
  selector on exactly the clients the id contract was written for. It now
  accepts a number or a decimal string, and advertises the string.

- **The two halves of a working-context round trip disagreed.**
  `save_working_context` advertises its nested `fragment_id`/`memory_id` as
  decimal strings (an input schema may announce only one form), while
  `load_working_context` answered with JSON numbers. An agent that resumed a
  session, enriched what it loaded and saved it back therefore had to
  convert — and on a float-lossy client the value was already rounded at read
  time, so it stored a corrupted id with the apparent exactness of a string.
  `load_working_context` now answers in decimal strings, the exact bytes its
  writing half accepts. The advertised output schema already typed those
  fields `["integer", "string"]`, so no SDK-side validation changes.

### Added

- **`save_working_context` returns `id_str`.** It was the one tool handing
  back an id without its decimal-string twin, while `forget`/`feedback`
  advertise only the string form — leaving no way to relay the id it had just
  returned. A test now derives the rule from the published surface (a name
  the input side wants as a string and the output side answers as an
  `integer` must carry an `_str` twin) instead of relying on the convention
  being remembered.

## [0.11.6] — 2026-07-29

### Added

- **`recall_fused` exposes `pool` over MCP.** The three bindings (Node
  `{pool?}`, WASM `{pool?}`, Python `options={"pool": …}`) had long exposed
  the depth of the oversampled vector pool fusion re-ranks; the MCP tool —
  the server they all sit on — never did, so an MCP caller could not narrow
  or widen it. Worse than merely absent: an undeclared argument is not
  refused, so `{"pool": 1}` looked accepted and still returned a full-depth
  result. The knob is now advertised (carrying a direct `type`, as every slot
  must) and routed through the same `FusionOptions::from_knobs` as the
  bindings, so all four transports share one default (`max(limit × 8, 64)`),
  one floor (1 — `pool: 0` never oversamples an empty set) and one ceiling.

- **`unrelate` — `relate`'s exact undo (#1661).** The graph was the only
  facet whose writes were one-way: a mistaken edge could only be removed by
  destroying the facts at its endpoints. `unrelate(from, to, relation)`
  removes exactly the named edge(s), touches neither the facts nor any
  entity, refuses exactly what `relate` refuses (empty label, self-loop),
  and is idempotent — an absent edge answers `{ found: false }`, not an
  error, so a cleanup is replayable. Exposed as an MCP tool, on
  `MemoryService`, and on the `MemoryStore` trait (native + WASM backends).
  Scope: the store does not distinguish an explicit edge from an
  autograph-derived one, so both are removable — correcting an autograph
  edge is better done by `forget` + `remember` of the source fact.

### Fixed

Five input defects found by a systematic scenario campaign against the 0.11.4
daemon (#1654). All five shared one shape: an input the tools **accepted**,
then did something other than what the caller asked — a silent wrong answer
rather than a refusal.

- **An over-long `remember` came back as a backend fault.** A fact past what
  the embedding model accepts surfaced as
  `embedding error: embedding backend error: ollama embeddings call failed`,
  naming neither a limit nor the offending size. Facts are now capped at
  `limits::MAX_EMBEDDABLE_TEXT_BYTES` (2 KiB — the default `all-minilm`
  backend's 512-token window at this crate's own prose rate), checked *before*
  the embedder, and refused with both numbers and what to do instead
  (`MemoryError::FactTooLarge`). The cap is stated in the tool description.

- **`ttl_seconds: 0` meant "never", silently.** An explicit per-call `0` was
  normalised to "no expiry", so a caller who meant "expire immediately" got a
  **permanent** fact with no signal. It is now refused
  (`MemoryError::ZeroTtl`). A TTL supplied as *configuration*
  (`with_default_ttl`, a compile policy's `source_ttl_seconds`) still reads `0`
  as "no TTL policy" — that is a default about a server, not an intent about
  one fact.

- **`entity` returned an empty name on a miss.** `found: false` came back with
  `name: ""`, so several lookups could not be told apart. A miss now echoes the
  queried name in its canonical (trimmed, lowercased) form — through the same
  `service::canonical_entity_name` a hit goes through, so the two cannot drift.

- **`relate` accepted a self-loop.** `relate(X, X, …)` created an edge that
  states nothing and that `why` then traverses like any other, adding noise to
  the evidence trail. Refused (`MemoryError::SelfRelation`), on `remember`'s
  `links` as well as on `relate` itself.

- **`save_working_context` accepted an entirely empty state.** The write is an
  idempotent upsert, so an empty `working` **replaced** — destroyed — the rich
  state saved under the same project and session: the one tool whose job is
  surviving a context loss could cause one. Refused
  (`MemoryError::EmptyWorkingContext`) unless at least one field carries
  something.

## [0.11.5] — 2026-07-28

### Changed

- **Documentation examples no longer name real people.** The autograph
  examples used the names of an actual family, minors included. The most
  exposed of them sat in the `entity` tool's own description — the text
  every MCP client receives when it lists the tools — and the rest in both
  `SKILL.md` files, the `service`/`extract` rustdoc that ships to docs.rs,
  and the `graph_autocomplete_bdd` suite. Replaced throughout by wholly
  fictional names that keep the same family shape, because that shape is
  exactly what the tests contrast: a parent-child copula, which must stay
  correct, and a sibling possessive, which was the construction inverting
  the triple.

  A documentation example has no reason to carry anyone's identity. No
  behaviour changes: the names are example data, and entity ids are
  content-addressed, so they are recomputed either way.

  Note for anyone reading the registries: **0.11.3 never reached crates.io
  or npm** — its `Validate` job failed and skipped all five downstream
  publish jobs. Only its MCPB bundles exist.


## [0.11.4] — 2026-07-28

### Fixed

- **0.11.3 could not be packaged.** Its TTL fix reached for a
  `store_with_metadata_and_ttl` method added to `velesdb-core` in the same
  train. That compiles in the workspace, where core is resolved by path, but
  `velesdb-memory` publishes independently against the *released* core — which
  does not have it — so `cargo publish` failed to verify the tarball and
  nothing reached crates.io or npm. The fix now uses only published core API:
  the fact is stored with its metadata and **no expiry**, then `set_ttl_durable`
  applies the expiry. Same guarantee — the fact cannot expire mid-write, since
  it has no expiry until the last step — with no new core surface.

  0.11.3 is **not usable**: its MCPB bundles reached the MCP registry before the
  packaging step failed, but no crate or npm package was ever published. Use
  0.11.4.

## [0.11.3] — 2026-07-28

### Fixed

- **`remember_with_ttl` could fail on a perfectly valid fact.** A TTL'd write
  was two store calls — `store_with_ttl` then `update_metadata` — and the fact
  was already live and expiring between them, so a short TTL could lapse in the
  gap and the metadata write then failed with
  `NotFound(... is expired ...)`. Not a narrow edge case: the automatic date
  stamp means metadata is always present, so *every* TTL'd write took that
  path. Core now exposes `store_with_metadata_and_ttl` (its `store_internal`
  always accepted both) and the service issues ONE write. The storage trait
  method carries a default implementation reproducing the old sequence, so a
  backend written before this keeps working. (#1641)
- **The config file was looked up beside the DEFAULT store, not the effective
  one.** `VELESDB_MEMORY_PATH` moves the store and `velesdb-memory.toml` lives
  beside it, so moving the store silently read the config of a store you were
  not using — including a developer's personal `~/.velesdb-memory` in the
  middle of a test run. (#1633)
- **SIGTERM killed the daemon instead of draining it.** Only `ctrl_c` (SIGINT)
  was handled, while `launchctl kickstart`, `systemctl restart` and
  `docker stop` all send SIGTERM. Unhandled, it dropped the streamable-HTTP
  sessions clients hold mid-flight, so the next call on a live session hung
  until the client's own timeout instead of reconnecting — a daemon upgrade
  looked like a broken memory. (#1636)
- **`forget` left the entities its fact had created behind.** Entity hubs
  outlived every fact that created them, so a retraction was silently
  incomplete and the graph accumulated unreachable nodes. `forget` now collects
  the hubs no surviving fact mentions — an entity another fact still refers to
  is kept. (#1634)

### Changed

- **Autograph predicates are bounded and entities must be linked.** On real
  content the extractor answered with restated sentences
  (`est utilisé pour la surveillance de fuites de données`) where a label was
  asked for, and an entity could receive attributes without a single edge — a
  dead end in the graph. The prompt now carries a hard three-word bound, a
  counter-example, and the rule that a related entity appears in at least one
  triple. Measured on the same content: 9 words → 3, and an entity that had no
  edge now has one. (#1635)

## [0.11.2] — 2026-07-26

Patch. Four defects found in real usage, each reproduced by a test that failed
before its fix.

### Fixed

- **`other_sessions` was hard-coded empty on a hit.** `load_working_context`
  returned `Vec::new()` whenever the requested session was found, so the field
  the tool advertises as its typo-recovery aid never helped the one case where
  an agent needs it — a session name off by a character. It is now populated
  from the project index on a hit as well as on a miss.
- **Concurrent saves silently erased each other's index entry.**
  `update_working_index` was an unsynchronised read-modify-write of a single
  shared fact per project, so two overlapping `save_working_context` calls left
  the last writer's view: the other session's record still existed but had
  vanished from `list_working_contexts`, with no error anywhere. Writes are now
  serialised. The lock is intra-process — two processes on the same store still
  race, which needs a CAS on the `MemoryStore` trait and is not in this patch.
- **A lost index body read back as "nothing was ever saved".** The index
  lookup collapsed "absent" and "corrupt" into `None`, so an agent told the
  project was empty started over instead of surfacing a problem a human could
  fix. Corruption is now an error on the read path. The write path deliberately
  rebuilds instead of propagating: the only writer of the index is
  `update_working_index`, so failing there would have bricked every future save
  for that project with no way back.
- **The Ollama embedder never retried a dropped connection.** The client keeps
  a keep-alive pool; Ollama closes an idle connection, `ureq` takes it from the
  pool and gets an `ECONNRESET` — and refuses to replay it, since a POST with a
  body is neither idempotent nor empty. That surfaced as
  `Connection reset by peer (os error 54)` while `/api/tags` answered in 7 ms.
  Transport failures are now replayed on a fresh connection, classified on
  `ureq::Error` variants rather than by matching error text. Timeouts are
  deliberately *not* replayed: `remember_extracted` issues one embed per fact
  plus one per entity hub, so replaying a 60 s timeout would have tripled a
  worst case already measured in minutes. Connect timeout drops from 30 s to
  2 s for a localhost daemon. The error now names the URL, the model, the
  attempts made and the environment variables that change them — and the
  extractor's message names *its* variables, which are not the embedder's.

### Changed

- **MCP input schemas no longer publish untyped `items`.** The schema inliner
  descended only one level and skipped any slot that already carried a `type`,
  so every array-of-struct parameter reached clients as `items: {}` —
  `save_working_context`, `compile_context`, `explain_compilation`, `remember`
  and `recall_where`. Callers had to discover `ContextFact`, `ContextDecisionRef`
  and `SourceReference` by trial and error. Inlining is now recursive, bounded
  by depth and a visited-`$ref` set. `fragment_id` also accepts a decimal
  string, since ids exceed 2^53 and float-lossy clients cannot send them as
  numbers.

## [0.11.1] — 2026-07-24

Patch: fixes the MCP wire-contract bug (harness-stringified parameters) plus
the Claude Desktop onboarding automation — no new memory-shape/API change.

### Added

- `scripts/install-memory-daemon.sh` now wires **Devin CLI**
  (`~/.config/devin/config.json`) alongside Claude Code, Claude Desktop and
  Windsurf; documented in the README's stdio and HTTP-transport client
  sections.
- Both installers now wire **Claude Desktop** automatically (macOS and
  Windows): an `mcp-remote` stdio→HTTPS bridge entry is written into
  `claude_desktop_config.json` (non-destructive merge, timestamped backup)
  with `NODE_EXTRA_CA_CERTS` pointing at the daemon's local CA, so the bridge
  verifies TLS strictly — no manual proxy setup, no
  `NODE_TLS_REJECT_UNAUTHORIZED=0`. The exact TLS path the bridge will use is
  probed (Node HTTPS request to `/health` with `NODE_EXTRA_CA_CERTS`) before
  the entry is written; absolute command paths are resolved so Desktop's
  shell-less, minimal-`PATH` spawn works. Without Node.js the installers fall
  back to printing the Settings → Connectors instructions. Documented in the
  README's new "Claude Desktop (macOS / Windows)" section (happy path,
  troubleshooting, CA-trust removal).
- `--wire-only` (`.sh`) / `-WireOnly` (`.ps1`): re-verify CA trust and
  re-wire all clients against an already-installed daemon — no build, no
  daemon restart, no interactive prompts.

### Fixed

- **MCP tool parameters are now harness-proof on both wire directions**
  (issue #1575). Real MCP client harnesses were observed serializing
  non-string arguments as JSON-encoded strings once their view of the
  advertised schema degraded — `save_working_context`'s `working` object
  arrived as `"{\"goal\": ...}"` and failed with `invalid type: string,
  expected struct WorkingContext`, silently losing session handoffs;
  `recall_fused` rejected `limit: "6"` and stringified `filter` objects
  the same way. Same defensive-interop class as the #1468 string-id
  contract. Two-sided fix: (1) `$ref`-only top-level parameter schemas
  are now inlined so every parameter advertises a direct `type` keyword
  (`working` exposes `type: object`); (2) a lenient deserializer on every
  non-string tool parameter accepts the properly-typed value first and
  falls back to parsing a JSON-encoded string, with a precise error when
  neither form fits.
- The installer no longer writes a `type:"http"` entry into
  `claude_desktop_config.json` — confirmed Desktop's config file never reads
  that shape (silently ignored). Superseded in this same release by the
  `mcp-remote` bridge entry above, which Desktop's config file *does* read.
- The CA-trust step on both platforms now **verifies** after trusting instead
  of assuming: a strict HTTPS request to the daemon (no `--cacert`, i.e.
  against the OS trust store) must succeed, and a trust command that exits 0
  without actually taking effect is reported with the exact command to re-run
  by hand. (Follows up the idempotency ground-truth check from #1537.)

### Changed

- `id::stable_id`/`id::stable_id_bytes` now delegate to
  `velesdb_core::hash_id`/`hash_id_bytes` instead of re-declaring their own
  FNV-1a offset/prime constants — internal dedup only, byte-identical output
  (pinned by a golden-vector regression test). (#1542)

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
