# License boundary — what the parity audit must enforce

`velesdb-core` is the **single source of truth** *and* a **licensed asset**. The gap analysis must reason about both at once.

## The two licenses in this repo

| Surface | License | Notes |
|---------|---------|-------|
| `velesdb-core` (source of truth) | **VelesDB Core License 1.0** | Source-available, based on Elastic License 2.0 (ELv2). `Copyright (c) 2024-2026 Wiscale France`. `VelesDB®` is a registered trademark. |
| `crates/velesdb-{server,cli,python,wasm,mobile,migrate}`, `tauri-plugin-velesdb` | **VelesDB Core License 1.0** | `license-file = LICENSE` (root or `../../LICENSE`). |
| `sdks/typescript` | **VelesDB Core License 1.0** | `"license": "SEE LICENSE IN LICENSE"`. |
| `integrations/{langchain,llamaindex,haystack,common}` | **MIT** | `pyproject.toml license = MIT`. |
| `docs/`, `README.md` | repo (Core License 1.0) | |

## What the Core License protects (verbatim concepts)

The license names these as the Software's capabilities that may not be re-offered or competitively reimplemented: **query, administration, indexing, ingestion, storage, graph traversal, knowledge-graph, columnar filtering, management** — i.e. exactly the canonical logic in `velesdb-core`.

Key limitations the audit should be aware of:
1. **No Hosted/Managed Service** — exposing core's capabilities to third parties as a service needs a commercial license.
2. **No Competitive Offering** — may not build a competing database/vector-db/graph-db/search/query engine from it.
3. **No Circumvention of Paid Features** — do not disable/remove licensing-gated functionality.
4. **No Removal of Notices/Trademarks** — keep `VelesDB®` and copyright notices intact.

Internal use, and integrating VelesDB as a **backend component** of your own product (end users get only results, not core DB access), are explicitly allowed.

## Why this matters for gap-closing recommendations

The MIT integrations (`langchain`, `llamaindex`, `haystack`, `common`) wrap the **Core-Licensed** Python binding. That is the *intended* shape: an MIT thin layer that **calls** the licensed engine. It stays clean — and license-compliant — only as long as the MIT layer does **not embed or copy** core's protected logic.

Therefore the audit's remediation rules are:

- ✅ **Valid:** "Capability X is missing in the MIT integration → expose X on the `velesdb` binding (Core License) and have the integration **call** it." The protected logic stays in core; the MIT layer only orchestrates.
- ❌ **Invalid (never recommend):** "Copy core's fusion math / distance kernel / VelesQL execution / id-hashing into the MIT integration." That both forks the source of truth (architecture violation) **and** risks relicensing protected logic as MIT (license violation).
- ❌ **Invalid:** relicensing Core-License code as MIT, or pulling MIT code into core in a way that muddies the Core License.
- ⚠️ **Escalate:** any Phase-4 `duplication`/`reimplementation`/`fidelity` finding located in an **MIT** package that touches a protected capability — it is simultaneously an architecture smell and a potential license leak. Rank it above the same finding in a Core-Licensed crate.

## Reporting hygiene

- Reports, PRs, commits, and issues you generate must follow the project **no-AI-attribution** rule (no `Co-Authored-By`, no AI/tool credit) and must not strip or alter `VelesDB®`/copyright notices.
- The audit artifacts themselves are internal use — fine to produce and share within the org.
