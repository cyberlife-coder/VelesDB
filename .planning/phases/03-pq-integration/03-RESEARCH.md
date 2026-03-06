# Phase 3: PQ Integration - Research

**Researched:** 2026-03-06
**Domain:** VelesQL grammar extension, serde-compatible config redesign, Criterion recall benchmarking
**Confidence:** HIGH

## Summary

Phase 3 wires the Phase 2 internal PQ pipeline (k-means++, ADC SIMD, OPQ, RaBitQ, GPU training, codebook persistence) to user-facing surfaces: a VelesQL `TRAIN QUANTIZER` command, a backward-compatible `QuantizationConfig` redesign, and a Criterion `pq_recall` benchmark suite. No internal PQ engine changes are required -- all building blocks exist.

The integration surface is well-scoped. The grammar extension follows established pest patterns (case-insensitive keywords, `with_option` reuse). The config redesign is constrained by serde backward compatibility with existing `config.json` files. The benchmark suite follows the existing Criterion pattern in `benches/pq_hnsw_benchmark.rs`.

**Primary recommendation:** Implement in three sequential waves: (1) grammar + AST + parser, (2) config redesign + executor wiring, (3) benchmark suite. Each wave is independently testable and commitable.

<user_constraints>
## User Constraints (from CONTEXT.md)

### Locked Decisions
- Grammar extension: New `train_stmt` top-level rule in `grammar.pest` alongside existing `match_query`, `compound_query`, `insert_stmt`, `update_stmt`
- Syntax: `TRAIN QUANTIZER ON <collection> WITH (m=8, k=256)` -- key=value pairs inside WITH clause
- Supported params: `m` (required), `k` (optional=256), `type` (optional='pq'), `oversampling` (optional=4), `sample` (optional), `force` (optional=false)
- Training is always explicit, never automatic -- core design principle
- Execution model: synchronous, blocking until codebook trained and persisted
- AST: New `TrainStatement` struct with `collection: String, params: HashMap<String, Value>`
- Executor path: `Database::execute_train()` method, separate from query execution
- QuantizationConfig redesign: tagged enum `QuantizationMode` with `#[serde(tag = "type")]`
- Serde compatibility: `#[serde(alias = "none")]` etc. for old string values
- Benchmark file: `benches/pq_recall_benchmark.rs` -- separate from `pq_hnsw_benchmark.rs`
- Recall thresholds: PQ m=8 rescore >= 92%, OPQ m=8 rescore >= 95%, RaBitQ >= 85%
- Conformance cases added to `conformance/velesql_parser_cases.json`
- Error variants: `CollectionNotFound`, `InvalidQuantizerConfig`, `TrainingFailed`, `QuantizerAlreadyTrained`

### Claude's Discretion
- Exact pest grammar rule ordering and naming conventions for `train_stmt`
- Internal test fixture design (vector generation, cluster placement)
- `TrainStatement` AST struct layout details
- Error message wording for TRAIN QUANTIZER failures
- Benchmark iteration count and warm-up configuration
- Whether to split `QuantizationMode` into its own file or keep in `config.rs`

### Deferred Ideas (OUT OF SCOPE)
- REST API endpoints for TRAIN QUANTIZER -- Phase 5+
- Configurable distance metrics for ADC (cosine/dot product LUT paths)
- WAND-accelerated RaBitQ search for >1M vectors -- v1.6
- Async/streaming training progress reporting
</user_constraints>

<phase_requirements>
## Phase Requirements

| ID | Description | Research Support |
|----|-------------|-----------------|
| PQ-05 | `TRAIN QUANTIZER ON <collection> WITH (m=8, k=256)` -- explicit training via VelesQL | Grammar extension pattern verified in `grammar.pest` (line 9, `query` rule). Parser dispatch verified in `parser/select/mod.rs` (line 16-32). DML pattern (parse_insert_stmt, parse_update_stmt) provides template. AST extension via new `TrainStatement` in `ast/` module. Executor via new `Database::execute_train()`. Conformance cases in `conformance/velesql_parser_cases.json`. |
| PQ-06 | `QuantizationConfig` extended with PQ variant -- retrocompatible with SQ8/Binary | Current `QuantizationConfig` at `config.rs:234` uses stringly-typed `default_type: String`. `CollectionConfig` at `collection/types.rs:115` already has `storage_mode: StorageMode` and `pq_rescore_oversampling`. `save_config()` serializes to `config.json` via `serde_json::to_string_pretty`. Serde `#[serde(tag = "type")]` + `#[serde(alias)]` enables backward-compatible deserialization. |
| PQ-07 | Criterion `pq_recall` suite with recall@10 >= 92% for m=8 in baseline.json | Existing benchmark pattern in `benches/pq_hnsw_benchmark.rs`. Registration in `Cargo.toml` via `[[bench]]` section. Baseline at `benchmarks/baseline.json`. Export via `scripts/export_smoke_criterion.py`. |
</phase_requirements>

## Standard Stack

### Core
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| pest / pest_derive | (workspace) | PEG parser for VelesQL grammar | Already used -- grammar.pest is the single source of truth |
| serde / serde_json | (workspace) | Config serialization with backward compat | Already used for CollectionConfig persistence |
| criterion | (workspace dev-dep) | Benchmark framework with CI integration | Already used for 35+ benchmark suites |

### Supporting
| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| tempfile | (workspace dev-dep) | Temporary directories for benchmark collections | Benchmark test fixtures |
| rand | 0.8 (workspace dep) | Seeded RNG for reproducible benchmark data | Gaussian cluster generation in benchmark |

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| pest PEG grammar | nom combinator parser | pest is already the parser -- no reason to switch |
| serde tagged enum | Manual deserialization | Tagged enum is idiomatic serde and handles migration cleanly |

**Installation:** No new dependencies required. All libraries are already in the workspace.

## Architecture Patterns

### Recommended Project Structure
```
crates/velesdb-core/src/
  velesql/
    grammar.pest          # Add train_stmt rule
    ast/
      mod.rs              # Add TrainStatement re-export, extend Query
      train.rs            # NEW: TrainStatement struct
    parser/
      train.rs            # NEW: parse_train_stmt implementation
      select/mod.rs       # Add Rule::train_stmt arm to parse_query
    mod.rs                # Re-export TrainStatement
  config.rs               # Redesign QuantizationConfig with QuantizationMode enum
  database.rs             # Add execute_train() method
  error.rs                # Add TrainingFailed, QuantizerAlreadyTrained variants
  collection/
    types.rs              # Update CollectionConfig if needed
benches/
  pq_recall_benchmark.rs  # NEW: recall accuracy benchmark
conformance/
  velesql_parser_cases.json  # Add TRAIN QUANTIZER cases
benchmarks/
  baseline.json           # Add pq_recall thresholds
```

### Pattern 1: Grammar Rule Extension
**What:** Adding a new top-level statement to the pest grammar
**When to use:** Any new DDL/DML command in VelesQL
**Example:**
```pest
// In grammar.pest, line 9:
query = { SOI ~ (match_query | compound_query | train_stmt | insert_stmt | update_stmt) ~ ";"? ~ EOI }

// New rule:
train_stmt = {
    ^"TRAIN" ~ ^"QUANTIZER" ~ ^"ON" ~ identifier ~ with_clause
}
```
**Key insight:** The `with_clause` rule already exists (line 132-135) and parses `WITH (key=value, ...)`. Reuse it directly -- no new value parsing needed. The `with_value` rule already handles string, float, integer, boolean, and identifier types.

### Pattern 2: AST + Parser Dispatch
**What:** New statement type parsed from grammar into AST, dispatched in parse_query
**When to use:** Adding any new statement type
**Example:**
```rust
// In parser/select/mod.rs, parse_query():
match p.as_rule() {
    Rule::match_query => return Self::parse_match_query(p),
    Rule::compound_query => return Self::parse_compound_query(p),
    Rule::train_stmt => return Self::parse_train_stmt(p),  // NEW
    Rule::insert_stmt => return Self::parse_insert_stmt(p),
    Rule::update_stmt => return Self::parse_update_stmt(p),
    _ => {}
}
```

### Pattern 3: Query Struct Extension for Non-SELECT Statements
**What:** The `Query` struct currently uses DML and match_clause optionals alongside a default SelectStatement
**When to use:** Adding statement types that don't produce result rows
**Design choice:** Add a `train` field to `Query` (like `dml: Option<DmlStatement>`) OR introduce a top-level `Statement` enum. The simpler approach (matching the DML pattern) is to add `train: Option<TrainStatement>` to `Query` and a `Query::new_train()` constructor, mirroring `Query::new_dml()`.

### Pattern 4: Serde Tagged Enum Migration
**What:** Replacing stringly-typed config with a tagged enum while maintaining backward compatibility
**When to use:** Config evolution with existing serialized data
**Example:**
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum QuantizationMode {
    #[serde(alias = "none")]
    None,
    #[serde(alias = "sq8")]
    SQ8,
    #[serde(alias = "binary")]
    Binary,
    #[serde(rename = "pq")]
    PQ {
        m: usize,
        #[serde(default = "default_k")]
        k: usize,
        #[serde(default)]
        opq_enabled: bool,
        #[serde(default = "default_oversampling")]
        oversampling: Option<u32>,
    },
    RaBitQ,
}
```
**Critical concern:** The existing `QuantizationConfig.default_type` is a flat `String` ("none", "sq8", "binary"). The new `QuantizationMode` is a tagged enum. These are structurally different in JSON. Two approaches:
1. **Replace `default_type` with `mode: QuantizationMode`** -- requires custom deserializer to accept old `"default_type": "sq8"` format
2. **Keep `default_type` for backward compat, add `mode` with `#[serde(default)]`** -- simpler but carries dead field

Recommendation: Use approach 1 with a custom `Deserialize` impl or `#[serde(untagged)]` wrapper that first tries the new format, then falls back to legacy string parsing. This is cleaner long-term.

### Anti-Patterns to Avoid
- **Adding TRAIN as a SELECT variant:** TRAIN is a DDL command, not a query. It should never go through the SELECT execution path.
- **Auto-training on collection create:** The requirement explicitly states training must be explicit. Never trigger `ProductQuantizer::train()` from `Collection::create_with_options()` when PQ mode is selected.
- **Blocking the global lock during training:** Training acquires `pq_quantizer` write lock. Ensure the training data is extracted (vectors cloned from storage) BEFORE acquiring the write lock, so read operations are not blocked during k-means iterations.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| WITH clause parsing | Custom key=value parser | Existing `with_clause` / `with_option` pest rules | Already tested, handles quoting and types |
| Benchmark harness | Manual timing loops | Criterion `criterion_group!` / `criterion_main!` | Statistical rigor, CI integration, baseline comparison |
| Config migration | Version detection + manual field mapping | Serde `#[serde(alias)]` + `#[serde(default)]` | Handles all edge cases, zero runtime cost |
| Recall computation | Custom recall@k metric | Compute inline (HashSet intersection / k) | Simple enough to inline, no library needed |

**Key insight:** The VelesQL grammar already has a reusable `with_clause` pattern. The TRAIN statement just needs `^"TRAIN" ~ ^"QUANTIZER" ~ ^"ON" ~ identifier ~ with_clause` -- about 3 lines of grammar.

## Common Pitfalls

### Pitfall 1: Serde Backward Compatibility Break
**What goes wrong:** Old `config.json` files with `"default_type": "sq8"` fail to deserialize with the new `QuantizationMode` enum
**Why it happens:** `#[serde(tag = "type")]` expects a `"type"` key in the JSON object, but old configs have `"default_type"` as a flat string
**How to avoid:** Test deserialization of actual old config.json snippets before finalizing the serde strategy. Write a dedicated test with the exact old JSON format. Consider `#[serde(untagged)]` with try-deserialize-new-then-old fallback.
**Warning signs:** Existing tests that create `QuantizationConfig` from hardcoded values pass, but loading from disk fails

### Pitfall 2: Grammar Ambiguity with TRAIN Keyword
**What goes wrong:** pest parser fails on queries containing "TRAIN" as a column name or identifier
**Why it happens:** `^"TRAIN"` as a keyword in the `query` rule could conflict with identifiers
**How to avoid:** `train_stmt` is a top-level alternative in the `query` rule, so it only matches when followed by `^"QUANTIZER"`. The two-keyword prefix `TRAIN QUANTIZER` is unambiguous. No risk of collision with identifiers since pest tries alternatives in order and `TRAIN QUANTIZER` is a distinct prefix from `SELECT`, `MATCH`, `INSERT`, `UPDATE`.
**Warning signs:** Parser tests with TRAIN as a column name fail

### Pitfall 3: Lock Ordering Violation in execute_train
**What goes wrong:** Deadlock when training holds `pq_quantizer` write lock while trying to read `vector_storage`
**Why it happens:** Lock ordering is `config(1) > vector_storage(2) > pq_quantizer(5)`. Training needs to read vectors first, then write the quantizer.
**How to avoid:** Clone/extract training vectors from `vector_storage` (release read lock), then acquire `pq_quantizer` write lock. Never hold both simultaneously. Follow the existing pattern in `crud_helpers.rs` which does the same.
**Warning signs:** Concurrent operations hang during TRAIN QUANTIZER execution

### Pitfall 4: Benchmark Recall Variance
**What goes wrong:** Recall@10 measurements fluctuate across runs, sometimes failing the 92% threshold
**Why it happens:** PQ with random init can produce slightly different codebooks
**How to avoid:** Use seeded RNG for both training data generation AND k-means++ initialization. The existing `ProductQuantizer::train()` uses seeded k-means++ (Phase 2 decision). Ensure the benchmark passes a fixed seed.
**Warning signs:** CI flakes on the pq_recall benchmark

### Pitfall 5: QuantizationMode Name Collision
**What goes wrong:** `QuantizationMode` already exists in `velesql/ast/with_clause.rs` (line 12) -- it's the per-query quantization hint enum (F32/Int8/Dual/Auto)
**Why it happens:** CONTEXT.md proposes a new `QuantizationMode` enum for the config, but that name is taken
**How to avoid:** Name the config enum differently: `QuantizationType`, `QuantizationStrategy`, or `StorageQuantization`. Or move/rename the existing AST `QuantizationMode` (but that would be a breaking change to the AST API). Best option: name the new enum `QuantizationType` in `config.rs` to avoid collision.
**Warning signs:** Compile error on ambiguous import, or accidental shadowing

## Code Examples

### Grammar Extension (verified from existing grammar.pest patterns)
```pest
// Add to query rule (line 9):
query = { SOI ~ (match_query | compound_query | train_stmt | insert_stmt | update_stmt) ~ ";"? ~ EOI }

// New train_stmt rule:
train_stmt = {
    ^"TRAIN" ~ ^"QUANTIZER" ~ ^"ON" ~ identifier ~ with_clause
}
```

### TrainStatement AST (following DML pattern from ast/dml.rs)
```rust
use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use super::WithValue;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrainStatement {
    pub collection: String,
    pub params: HashMap<String, WithValue>,
}
```

### Parser Implementation (following parse_insert_stmt pattern)
```rust
impl Parser {
    pub(crate) fn parse_train_stmt(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Query, ParseError> {
        let mut collection = None;
        let mut params = HashMap::new();

        for inner in pair.into_inner() {
            match inner.as_rule() {
                Rule::identifier => {
                    if collection.is_none() {
                        collection = Some(extract_identifier(&inner));
                    }
                }
                Rule::with_clause => {
                    // Reuse existing with_clause parsing
                    let with = Self::parse_with_clause(inner)?;
                    for opt in with.options {
                        params.insert(opt.key, opt.value);
                    }
                }
                _ => {}
            }
        }

        let collection = collection.ok_or_else(|| {
            ParseError::syntax(0, "", "TRAIN QUANTIZER requires collection name")
        })?;

        Ok(Query::new_train(TrainStatement { collection, params }))
    }
}
```

### Config Backward Compatibility Test
```rust
#[test]
fn test_old_quantization_config_deserializes() {
    let old_json = r#"{
        "default_type": "sq8",
        "rerank_enabled": true,
        "rerank_multiplier": 2,
        "auto_quantization": true,
        "auto_quantization_threshold": 10000
    }"#;
    let config: QuantizationConfig = serde_json::from_str(old_json).unwrap();
    // Must not error -- backward compat is critical (PQ-06)
}
```

### Recall@10 Computation Pattern
```rust
fn recall_at_k(ground_truth: &[u64], results: &[u64], k: usize) -> f64 {
    let gt_set: HashSet<u64> = ground_truth.iter().take(k).copied().collect();
    let result_set: HashSet<u64> = results.iter().take(k).copied().collect();
    let intersection = gt_set.intersection(&result_set).count();
    intersection as f64 / k as f64
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| `default_type: String` in QuantizationConfig | Tagged enum `QuantizationType` | Phase 3 (this phase) | Enables structured PQ params |
| Auto-training PQ on insert threshold | Explicit TRAIN QUANTIZER command | Phase 3 (this phase) | User control, no surprises |
| PQ recall measured ad-hoc in pq_hnsw_benchmark | Dedicated pq_recall benchmark suite | Phase 3 (this phase) | CI-enforced recall thresholds |

**Important existing state:**
- `StorageMode` enum already has `ProductQuantization` and `RaBitQ` variants (quantization/mod.rs:41)
- `CollectionConfig` already has `storage_mode: StorageMode` and `pq_rescore_oversampling: Option<u32>` (collection/types.rs:130,156)
- `Collection` already has `pq_quantizer: Arc<RwLock<Option<ProductQuantizer>>>` and `pq_training_buffer` (collection/types.rs:211,215)
- `ProductQuantizer::train()` and `train_opq()` are ready (quantization/pq.rs:120,401)
- `RaBitQIndex::train()` is ready (quantization/rabitq.rs:305)
- `Error::InvalidQuantizerConfig` already exists (error.rs:154)

## Open Questions

1. **QuantizationConfig redesign scope**
   - What we know: The `QuantizationConfig` in `config.rs` is the global config (velesdb.toml). The `CollectionConfig` in `collection/types.rs` is per-collection (config.json). Both need to support PQ.
   - What's unclear: Should `QuantizationConfig` (global) also get the enum redesign, or only `CollectionConfig`? The CONTEXT.md mentions redesigning `QuantizationConfig`, but the per-collection `storage_mode` + `pq_rescore_oversampling` already carry the runtime PQ state.
   - Recommendation: Redesign `QuantizationConfig` in `config.rs` as the global default configuration (what new collections get). The `CollectionConfig` already has `storage_mode` and `pq_rescore_oversampling` which is sufficient for per-collection state. The TRAIN QUANTIZER executor updates the per-collection config, not the global one.

2. **New Error Variants**
   - What we know: `InvalidQuantizerConfig` exists. CONTEXT.md mentions `TrainingFailed` and `QuantizerAlreadyTrained`.
   - What's unclear: Whether these warrant new VELES error codes or can reuse existing ones.
   - Recommendation: Add `TrainingFailed(String)` as VELES-029 and use `InvalidQuantizerConfig` for the "already trained" case with a descriptive message (avoids error enum bloat). The `force=true` parameter bypasses the already-trained check.

3. **Query struct extension approach**
   - What we know: `Query` has `dml: Option<DmlStatement>` and `match_clause: Option<MatchClause>`.
   - What's unclear: Whether to add `train: Option<TrainStatement>` to the existing `Query` struct or introduce a `Statement` enum wrapper.
   - Recommendation: Add `train: Option<TrainStatement>` to `Query` with `#[serde(default, skip_serializing_if = "Option::is_none")]`, matching the DML pattern. Add `Query::new_train()` and `Query::is_train()` helper methods. This is the minimal-change approach.

## Validation Architecture

### Test Framework
| Property | Value |
|----------|-------|
| Framework | cargo test (built-in) + criterion 0.5 |
| Config file | `Cargo.toml` `[[bench]]` sections |
| Quick run command | `cargo test -p velesdb-core --features persistence -- --test-threads=1 train` |
| Full suite command | `cargo test --workspace --features persistence,gpu,update-check --exclude velesdb-python -- --test-threads=1` |

### Phase Requirements -> Test Map
| Req ID | Behavior | Test Type | Automated Command | File Exists? |
|--------|----------|-----------|-------------------|-------------|
| PQ-05 | TRAIN QUANTIZER parses correctly | unit | `cargo test -p velesdb-core --features persistence -- train_stmt --test-threads=1` | Wave 0 |
| PQ-05 | TRAIN QUANTIZER executes on collection | integration | `cargo test -p velesdb-core --features persistence -- execute_train --test-threads=1` | Wave 0 |
| PQ-05 | TRAIN QUANTIZER conformance cases pass | integration | `cargo test -p velesdb-core --features persistence -- velesql_parser_conformance --test-threads=1` | Partial (file exists, new cases needed) |
| PQ-06 | Old config.json deserializes with new QuantizationConfig | unit | `cargo test -p velesdb-core -- quantization_config_compat --test-threads=1` | Wave 0 |
| PQ-06 | New PQ config roundtrips through serde | unit | `cargo test -p velesdb-core -- quantization_config_pq --test-threads=1` | Wave 0 |
| PQ-06 | Existing SQ8/Binary collections load without error | integration | `cargo test -p velesdb-core --features persistence -- existing_collection_compat --test-threads=1` | Wave 0 |
| PQ-07 | PQ m=8 k=256 rescore recall@10 >= 92% | bench | `cargo bench -p velesdb-core --bench pq_recall_benchmark -- --noplot` | Wave 0 |

### Sampling Rate
- **Per task commit:** `cargo test -p velesdb-core --features persistence -- --test-threads=1`
- **Per wave merge:** `cargo test --workspace --features persistence,gpu,update-check --exclude velesdb-python -- --test-threads=1`
- **Phase gate:** Full suite green + `cargo bench -p velesdb-core --bench pq_recall_benchmark -- --noplot` before `/gsd:verify-work`

### Wave 0 Gaps
- [ ] `crates/velesdb-core/src/velesql/ast/train.rs` -- TrainStatement AST struct
- [ ] `crates/velesdb-core/src/velesql/parser/train.rs` -- parse_train_stmt implementation
- [ ] `crates/velesdb-core/benches/pq_recall_benchmark.rs` -- recall accuracy benchmark
- [ ] `conformance/velesql_parser_cases.json` -- TRAIN QUANTIZER entries (P0XX IDs)
- [ ] `Cargo.toml` -- `[[bench]] name = "pq_recall_benchmark"` registration

## Sources

### Primary (HIGH confidence)
- `crates/velesdb-core/src/velesql/grammar.pest` -- current grammar structure, WITH clause pattern (lines 132-135)
- `crates/velesdb-core/src/velesql/parser/select/mod.rs` -- parse_query dispatch (lines 16-32)
- `crates/velesdb-core/src/velesql/parser/dml.rs` -- DML parsing pattern (template for TRAIN)
- `crates/velesdb-core/src/velesql/ast/mod.rs` -- Query struct, DML/Match optional fields
- `crates/velesdb-core/src/velesql/ast/dml.rs` -- DmlStatement pattern
- `crates/velesdb-core/src/velesql/ast/with_clause.rs` -- WithClause, WithValue, existing QuantizationMode (line 12)
- `crates/velesdb-core/src/config.rs` -- QuantizationConfig (lines 231-265)
- `crates/velesdb-core/src/collection/types.rs` -- CollectionConfig (lines 115-157), Collection fields
- `crates/velesdb-core/src/quantization/mod.rs` -- StorageMode enum (lines 38-54)
- `crates/velesdb-core/src/quantization/pq.rs` -- ProductQuantizer::train() (line 120), train_opq() (line 401)
- `crates/velesdb-core/src/error.rs` -- Error::InvalidQuantizerConfig (line 154)
- `crates/velesdb-core/benches/pq_hnsw_benchmark.rs` -- existing PQ benchmark pattern
- `crates/velesdb-core/src/collection/core/lifecycle.rs` -- save_config() (line 439)
- `crates/velesdb-core/src/database.rs` -- execute_query, execute_dml dispatch
- `conformance/velesql_parser_cases.json` -- existing conformance test structure
- `benchmarks/baseline.json` -- existing baseline structure

### Secondary (MEDIUM confidence)
- `.planning/phases/03-pq-integration/03-CONTEXT.md` -- user decisions and integration points

### Tertiary (LOW confidence)
- None

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- no new dependencies, all libraries already in workspace
- Architecture: HIGH -- all patterns verified directly from source code
- Pitfalls: HIGH -- lock ordering documented in codebase, serde compat concerns verified, QuantizationMode name collision discovered via source read

**Research date:** 2026-03-06
**Valid until:** 2026-04-06 (stable domain, no external dependencies changing)
