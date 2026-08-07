# Core / Premium Split — Verified Plan

> **Single source of truth** for how `velesdb` (open core) and `velesdb-private`
> (premium/enterprise) divide responsibility, and the action plan to finish the
> split honestly. This document is kept **identical in both repos** so the two
> sides read the same contract.
>
> - Open core: `velesdb` — VelesDB Core License 1.0
> - Premium: `velesdb-private` — Commercial
>
> Grounded in the actual code as of the audit below, **not** in the specs alone
> (several spec tasks are marked done on an unlanded branch — see Truth 2).

---

## 1. Verified truths (what the code actually says)

These were confirmed by reading both repos, not by trusting the specs.

**Truth 1 — The split is already done, and done correctly.**
Premium does **not** fork core. `crates/velesdb-premium/src/observer.rs` implements
`velesdb_core::DatabaseObserver`, consumes `Database::open_with_observer`, and
re-exports core's engine instead of copying it (`velesdb-premium/src/lib.rs`:
*"P0: use Core's implementation, don't duplicate"* for HNSW / quantization / SIMD /
distance). `velesdb-node` depends only on `velesdb-memory`, not on core directly.
The "fork or extend?" question is settled: **extension by trait.**

**Truth 2 — The core seam is LANDED (resolved 2026-07).**
`crates/velesdb-core/src/observer/` (`mod.rs`, `context.rs`, `*_tests.rs`) contains
the full control-plane boundary: `on_query_request` read gate,
`AccessDecision::{Allow,Deny,AllowWithScope}`, `hash_id` (stable FNV-1a), `LockRank`
with a reserved premium range 40–59, `WalCursor`, the `conformance/` harness, and the
removal of the dead `hnsw_delta_wal`. Merged to `develop` as
`feat(core): control-plane boundary seam` (3f4dcc11) and tagged **`v3.9.0`**; the
CHANGELOG `[3.9.0]` entry (2026-07-07) documents the public surface. A follow-up
(`fix/query-double-telemetry`) removes a double-firing of `on_query` on the `/query`
REST path so any registered observer counts each request exactly once.

**Truth 3 — The "cognitive differentiators" already live in open core.**
`core/src/agent/` has `reinforcement.rs`, `temporal_index.rs`, `episodic_memory.rs`,
`procedural_memory.rs`, `semantic_memory.rs`, `ttl.rs`, `snapshot.rs`.
`velesdb-memory` has `dated_context.rs`, `fused_recall.rs`, and `why()` via the
`Explanation` proof-graph. RL, temporal, episodic memory and replay are **not** things
to build — they exist, open.

**Truth 4 — One real gap: provenance.**
Zero matches for `provenance`/`Provenance` in core or memory. `Recollection` carries
free-form `metadata: Option<Map>`; `Explanation`/`MemoryNode` carry only
`id`/`content`/`hop` — no `who`/`when`/`source`/`confidence`/`validated_by`. This is
the one genuine cognitive gap and the natural EU AI Act hook.

**Truth 5 — Version truth restored (resolved 2026-07).**
`velesdb-private/Cargo.toml` pins `velesdb-core` by git tag `v3.9.0` (the old note
claiming premium is stuck at `1.9.1`/rev `eebfd779` was out of date). Core's own
`workspace.dependencies velesdb-core` (which had drifted to `3.8.1` after the 3.8.1
release bump) and `crates/velesdb-python/pyproject.toml` are now both aligned to
`3.9.0`, matching the package version. Note: the git tag `v3.9.0` exists and is
consumed by premium, but no GitHub **release** artifacts are published for it yet
(latest published release is v3.8.1) — download URLs in the install guides still
point at v3.8.1 by design until a 3.9.x release is cut.

**Headline:** you are not mid-architecture, you are **mid-hardening**. The split is
real. Pending work is (a) landing the uncommitted core seam, (b) finishing premium's
"truthful over impressive" GA gaps (P6 / R16–R28), and (c) one net-new open
primitive: provenance.

---

## 2. The boundary rule (settles every future decision)

> **CORE** if it's a *primitive*: single-node, single-tenant, deterministic,
> measurable (data + algorithm).
> **PREMIUM** if it's *multiple* (multi-tenant, multi-node, org memory) **or**
> *enforcement* (auth, policy, signing, certification) **or** *operational at scale*
> (Raft, HA).

Corollary, already respected by the code: **core exposes the data and the algorithm;
premium applies, controls, and certifies them.** Provenance data → core. RBAC-driven
validation of that provenance → premium.

---

## 3. CORE plan — `velesdb-core` + `velesdb-memory` (open)

Everything here stays single-node / single-tenant / deterministic. No account, no
role, no cluster enters core.

| # | Action | Where | Status today |
|---|--------|-------|--------------|
| **C0** | **Land the uncommitted seam.** Commit/PR the `observer/`, `conformance/`, `LockRank`, `WalCursor`, `hash_id`, delta-WAL removal on `chore/kiro-workspace-setup` → `develop`. Until merged, everything premium relies on is a working-tree artifact. | branch merge | ⚠️ done in tree, **not landed** |
| **C1** | **Fix the version truth.** Bumped core's internal `workspace.dependencies velesdb-core` `3.8.1` → `3.9.0` and `velesdb-python/pyproject.toml` `3.8.1` → `3.9.0`; corrected the stale "1.9.1" note. | `Cargo.toml`, `velesdb-python/pyproject.toml` | ✅ resolved 2026-07 |
| **C2** | **Provenance as a first-class typed field.** Promote `who`/`source`/`created_at`/`confidence`/`validated_by` from ad-hoc `metadata` into a typed `Provenance` struct on `Recollection`/`Link`, and carry it on `MemoryNode` so `why()` returns a structured audit trail, not a blob. | `velesdb-memory/model.rs`, `service.rs` | ❌ genuine gap |
| **C3** | **Bitemporality on the existing temporal index.** Evolve `temporal_index.rs` from decay-only to valid-time + transaction-time ("what did we know yesterday"). | `core/src/agent/temporal_index.rs` | ⚠️ decay only |
| **C4** | **Auditable RL outcome API.** Formalize `record_outcome(decision_id, result, human_feedback) → score → update` over the existing strategies — deterministic, to preserve "measured, not vibes". | `core/src/agent/reinforcement.rs` | ✅ strategies exist, no outcome API |
| **C5** | **Scope axis on facts (single-tenant default).** Add an optional `scope` field so premium multi-agent/org memory can narrow without core knowing tenancy. Prepares the axis; doesn't contain it. | `velesdb-memory/model.rs` + `MemoryStore` | ❌ absent |

---

## 4. PREMIUM plan — `velesdb-private` (closed, commercial)

The premium crates already exist and are substantial: `rbac`, `multitenancy`,
`audit`, `encryption`, `hybrid_search`, `join`, `licensing`, `snapshots`,
`query_cache`, `product_quantization`, `observer`, `velesql`, `webadmin`, `metrics`,
plus `server-premium` / `cli-premium` / `wasm-premium` / `premium-python`. The
pending work is **truthfulness**, not features — premium spec Task 11 (P6, R16–R28),
the only unfinished block.

| # | Action | Grounded in (premium spec) | Status |
|---|--------|-----------------------------|--------|
| **P1** | **Route HTTP reads through the observer gate.** `handlers/query.rs` + `handlers/search.rs` still call `db.search` directly, bypassing `on_query_request`; move to `core_db.execute_query` in-task, cache lookup *after* the gate. | Task 11.3–11.5 | code `[x]`, tests `[ ]` |
| **P2** | **Mount JOIN + prove reachability.** `handlers::join::router()` not nested in `create_router` → `POST /api/v1/join` unreachable; nest it, flip the route registry, add the assembled-router test. | Task 11.1–11.2 | mount `[x]`, test `[ ]` |
| **P3** | **Truthful 501 + advertised-claim reconciliation.** Graph-only path returns `501` not empty-`200`; CI check that OpenAPI/README/WebAdmin claims map to mounted, non-501 routes. | Task 11.6–11.7, 11.19–11.20 | partial |
| **P4** | **License gating as a truthful biconditional.** `is_premium_enabled(state, feature)` gate on every premium handler: valid+granted ⇒ execute, else `403 license-required` — never silent success/fabrication. | Task 11.11–11.12 | code `[x]`, property test `[ ]` |
| **P5** | **Live replication lag + no-silent-data-loss.** Wire measured `replication_lag_ms` into the status endpoints; fail loud on non-durable writes; encryption-at-rest round-trip proof. | Task 11.8–11.9, 11.13–11.16 | partial |
| **P6** | **WebAdmin gated off the route registry + E2E journey.** Only mounted RBAC-enforced routes render controls; full authenticate→collection→upsert→search→JOIN→backup→status E2E against the assembled router. | Task 11.21, 11.24 | `[ ]` |
| **P7** | **Consume the new core primitives** as they land: provenance in the audit trail (C2), bitemporal replay (C3), scope for org memory (C5). New premium crates by trait, never a fork. | after C2/C3/C5 | future |

---

## 5. Anti-patterns that must never cross the line

All currently respected — keep them so.

- ❌ Gating a cognitive primitive (hiding `reinforcement.rs`) — kills the open pitch. They stay open.
- ❌ Copying core code into `velesdb-private` — parity debt (what `core-parity-audit` tracks). Premium depends + implements traits only.
- ❌ RBAC/tenancy hard-coded in the core engine — the seam exists precisely so core stays tenancy-blind.
- ❌ `velesdb-node` depending on core directly — today it only touches `velesdb-memory`; preserve that. This boundary (memory-wedge-only, no raw VelesQL/`MATCH`/administration) is now documented user-facing in the [Node README](../crates/velesdb-node/README.md#need-the-full-engine).

---

## 6. Sequencing (solo founder, budget-constrained)

1. **C0 + C1** — land the seam, fix versions. Unblocks everything; pure hygiene.
2. **C2 provenance** — cheap, builds on existing `metadata` / `Explanation`, feeds the EU AI Act argument (Aug 2026).
3. **P1 + P2 + P3 + P4** — premium GA truthfulness. Makes the current release *honest and sellable*; the only open task block.
4. **C3 bitemporal → P5** — the vendable differentiator (replay + governance).
5. **C4 RL outcome API + P6/P7** — vision layer, once the socle is honest and sold.

Do not parallelize. The two blocking hinges are **C0** (land the core seam) and
**P1** (make HTTP enforcement real, not middleware-only) — everything else is
downstream of those.

---

## 7. The one correction to remember

The differentiators aren't things to invent, and the split isn't a decision to make —
both largely exist. The real risk is **spec-vs-code drift** (tasks marked `[x]` on an
unlanded branch; a "501-truthful" premium whose reads still bypass the gate). The
pertinent action plan is: **land what's built, make the claims true, and add
provenance** — not re-architect.

---

*Last updated: 2026-07-27 · Applies to: velesdb-core 4.3.0 (this stamp tracks the document revision, not a re-verification of the split).*
