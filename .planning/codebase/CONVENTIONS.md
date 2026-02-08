# Coding Conventions

**Analysis Date:** 2026-02-06

## Naming Patterns

**Files:**
- Module source: `snake_case.rs` (e.g., `quantization.rs`, `simd_native.rs`)
- Test files: `[module_name]_tests.rs` (e.g., `simd_native_tests.rs`, `error_tests.rs`)
- Benchmark files: `[name]_benchmark.rs` in `benches/` directory

**Functions:**
- Public API: `snake_case` (e.g., `dot_product_native`, `cosine_similarity`)
- Test functions: `test_[unit]_[scenario]_[expected_result]`
  - Example: `test_simd_level_cached`
  - Example: `test_validate_multiple_similarity_with_or_detected`
  - Example: `test_cosine_normalized_native`

**Variables:**
- Local variables: `snake_case` (e.g., `ground_truth`, `candidates`)
- Constants: `SCREAMING_SNAKE_CASE` (e.g., `MIN_RECALL_AT_1`, `MAX_VECTOR_DIMENSION`)
- Type parameters: `PascalCase` (e.g., `T`, `K`, `V`)

**Types:**
- Structs/Enums: `PascalCase` (e.g., `QueryValidator`, `DistanceMetric`)
- Traits: `PascalCase` with descriptive names (e.g., `VectorIndex`, `ReinforcementStrategy`)
- Error types: `PascalCase` with `Error` suffix (e.g., `ValidationError`, `ParseError`)

**Modules:**
- Private modules: `snake_case`
- Public modules: `snake_case` with `pub mod` declaration

## Code Style

**Formatting:**
- Tool: `cargo fmt` (enforced via pre-commit hook)
- Max line length: 100 characters (implied by .clippy.toml settings)
- Indentation: 4 spaces

**Linting:**
- Tool: `cargo clippy` with strict rules (`-D warnings`)
- Configuration: `.clippy.toml` with custom thresholds
  - `cognitive-complexity-threshold = 25`
  - `too-many-arguments-threshold = 7`
  - `too-many-lines-threshold = 100`
  - `type-complexity-threshold = 250`
  - `missing-docs-in-crate-items = true`

**Global Lint Configuration (lib.rs):**
```rust
#![warn(missing_docs)]
#![warn(clippy::all)]
#![warn(clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]
// Numeric casts allowed globally for SIMD/performance code
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::cast_sign_loss)]
```

**Numeric Casts (Required Pattern):**
```rust
// ❌ INTERDIT - Cast silencieux
let id = index as u32;

// ✅ OK - try_from avec gestion d'erreur
let id = u32::try_from(index).map_err(|_| Error::Overflow)?;

// ✅ OK - allow avec Reason explicite
#[allow(clippy::cast_possible_truncation)]
// Reason: value clamped to [0.0, 1.0] before cast
let percent = (value.clamp(0.0, 1.0) * 100.0) as u32;
```

## Import Organization

**Order:**
1. Standard library imports (`std::`)
2. External crate imports (e.g., `serde::`, `tracing::`)
3. Internal crate imports (`crate::`)

**Grouping Style:**
```rust
// ✅ CORRECT - imports groupés et ordonnés
use crate::{
    collection::Collection,
    index::hnsw::HnswIndex,
    storage::MmapStorage,
};

// ❌ ÉVITER - imports éclatés sur plusieurs lignes individuelles
use crate::collection::Collection;
use crate::index::hnsw::HnswIndex;
use crate::storage::MmapStorage;
```

**Path Aliases:**
- None observed - use explicit full paths

## Error Handling

**Error Type Pattern:**
```rust
use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Error, Debug)]
pub enum Error {
    #[error("[VELES-XXX] Descriptive message: {0}")]
    VariantName(String),
    
    #[error("[VELES-XXX] Complex error with fields: expected {expected}, got {actual}")]
    ComplexError { expected: usize, actual: usize },
    
    #[error("[VELES-XXX] Wrapped error: {0}")]
    Io(#[from] std::io::Error),
}
```

**Propagation Pattern:**
```rust
// Use ? operator for error propagation
let result = some_operation()?;

// Map errors with context
.map_err(|e| Error::Storage(format!("Failed to resize: {}", e)))?
```

**No unwrap() on User Data:**
```rust
// ❌ INTERDIT
let value = result.unwrap();

// ✅ OK - expect avec message explicite (pour code interne uniquement)
let value = result.expect("Lock poisoned - this is a bug");

// ✅ OK - propagation d'erreur
let value = result?;
```

## Unsafe Code

**SAFETY Comment Template (Obligatoire):**
```rust
// SAFETY: [Invariant principal maintenu]
// - [Condition 1]: [Explication]
// - [Condition 2]: [Explication]
// Reason: [Pourquoi unsafe est nécessaire]
unsafe { ... }
```

**Examples from codebase:**
```rust
// SAFETY: This function is only called after runtime feature detection confirms AVX-512F.
// - CPU features verified at runtime via is_x86_feature_detected!("avx512f")
// - Load operations use unaligned loads (_mm512_loadu_ps) for safety
// Reason: SIMD intrinsics require unsafe blocks
unsafe { _mm512_loadu_ps(...) }

// SAFETY: VectorSliceGuard is Send+Sync because:
// - The guard enforces exclusive access via epoch validation
// - Pointer validity is guaranteed by the epoch check
// - No mutable aliasing is possible through the guard
unsafe impl Send for VectorSliceGuard<'_> {}
```

**Cosine Similarity - Clamp Obligatoire:**
```rust
let score = (dot / (norm_a * norm_b)).clamp(-1.0, 1.0);
```

## Documentation

**Module Documentation:**
```rust
//! # Module Name
//!
//! Brief description of module purpose.
//!
//! ## Example
//!
//! ```rust
//! // Usage example
//! ```
```

**Function Documentation:**
```rust
/// Compute recall@k between retrieved results and ground truth.
///
/// # Arguments
///
/// * `retrieved` - IDs of retrieved results (in order)
/// * `ground_truth` - IDs of true nearest neighbors (in order)
/// * `k` - Number of results to consider
///
/// # Returns
///
/// Recall value between 0.0 and 1.0
```

**When to Comment:**
- All public items must have doc comments (`#![warn(missing_docs)]`)
- Complex algorithms need inline comments explaining the "why"
- Safety invariants need `// SAFETY:` comments
- Epics/User Stories referenced: `// EPIC-XXX US-YYY: description`

**Example:**
```rust
// EPIC-044 US-001: Multiple similarity() with AND is now supported (cascade filtering)
```

## Function Design

**Size Guidelines:**
- Target: <100 lines per function (`.clippy.toml: too-many-lines-threshold = 100`)
- Cognitive complexity: <25 (`.clippy.toml: cognitive-complexity-threshold = 25`)
- Arguments: <7 (`.clippy.toml: too-many-arguments-threshold = 7`)

**Return Patterns:**
- Use `Result<T>` for fallible operations
- Use `Option<T>` for optional values
- Use `#[must_use]` for important return values that shouldn't be ignored

**#[must_use] Pattern:**
```rust
#[must_use]
pub fn normalize(v: &[f32]) -> Option<Vec<f32>> { ... }
```

## Module Design

**Visibility Hierarchy:**
| Élément | Visibilité recommandée |
|---------|------------------------|
| Struct publique API | `pub` dans lib.rs |
| Struct interne | `pub(crate)` |
| Fonction helper | `pub(super)` ou privée |
| Constante module | `pub(crate) const` |

**Test Module Organization:**
```rust
// In module file (mod.rs or lib.rs)
#[cfg(test)]
mod module_name_tests;

// Test file: module_name_tests.rs
```

**Integration Tests Location:**
- `tests/*.rs` for crate-level integration tests
- `#[cfg(test)]` modules for unit tests of private functions

## Logging

**Framework:** `tracing` (never use `println!`/`dbg!` in production)

**Log Levels:**
- `error!` - Errors that need attention
- `warn!` - Warning conditions
- `info!` - Informational messages
- `debug!` - Debug information
- `trace!` - Very detailed tracing

**Example:**
```rust
use tracing::{debug, error, info, warn};

info!(collection = %name, "Creating new collection");
error!(error = %e, "Failed to persist data");
```

## Comments

**Allowed Patterns:**
- `// TODO: ` - Known future work
- `// FIXME: ` - Known bugs/issues
- `// BUG-XXX: ` - Bug references
- `// EPIC-XXX US-YYY: ` - Epic/User Story tracking
- `// SAFETY: ` - Unsafe code invariants
- `// Reason: ` - Why a lint is allowed

**Example:**
```rust
// BUG-001 regression: VectorSearch (NEAR) was not being validated
// TODO: Implement position tracking in EPIC-044 US-008
```

---

*Convention analysis: 2026-02-06*
