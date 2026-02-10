# VelesDB Core — Refactoring Milestone

## What This Is

A comprehensive refactoring milestone for VelesDB Core, a local-first vector database and cognitive memory system for AI agents. This milestone focuses on improving code quality, performance, stability, and maintainability across the entire Rust codebase while maintaining backward compatibility with existing APIs.

principe fondamental de VelesDB : core = single source of truth, tout composant = binding/wrapper fidèle.

## Core Value

The codebase becomes faster, cleaner, more maintainable, and production-ready without breaking existing functionality or public APIs.

## Requirements

### Validated

- ✓ Vector similarity search with HNSW indexing (native Rust implementation)
- ✓ Knowledge Graph with nodes, edges, and property indexing
- ✓ Agent memory systems (episodic, semantic, procedural, reinforcement)
- ✓ SIMD-accelerated distance calculations (AVX-512, AVX2, SSE, NEON)
- ✓ Multi-platform bindings (WASM, Python, Tauri plugin)
- ✓ SQL-like query language (VelesQL) with parser and executor
- ✓ Memory-mapped file persistence with crash recovery
- ✓ HTTP REST API server (Axum-based)
- ✓ Interactive CLI with REPL
- ✓ Migration tools from other vector databases

### Active

- [ ] **REFACTOR-001**: Apply Rust best practices throughout codebase
  - Replace `as` casts with `try_from()` or explicit bounds checks
  - Remove global `#[allow]` clippy attributes, use targeted allows with justification
  - Replace `eprintln!` with proper `tracing` logging
  - Add comprehensive SAFETY comments for all unsafe blocks
  
- [ ] **REFACTOR-002**: Apply Martin Fowler refactoring patterns
  - Extract sub-modules from files >500 lines (simd_native.rs ~2400 lines, etc.)
  - Remove code duplication across modules
  - Simplify complex functions (cognitive complexity < 25)
  - Improve naming clarity and consistency
  
- [ ] **REFACTOR-003**: Fix hidden bugs and code smells
  - Audit numeric cast patterns for overflow/truncation risks
  - Fix incorrect comments that don't match code behavior
  - Resolve parser fragility in VelesQL (multiple BUG-XXX markers)
  - Strengthen HNSW lock ordering documentation and validation
  
- [ ] **REFACTOR-004**: Remove dead code and unused dependencies
  - Identify and remove unreachable code
  - Audit `cargo machete` output for unused dependencies
  - Clean up feature flags and conditional compilation
  
- [ ] **REFACTOR-005**: Improve error handling and documentation
  - Convert panics to proper errors where appropriate
  - Add missing error context and chain information
  - Document all public APIs with examples
  - Fix misleading or outdated documentation

### Out of Scope

- **New features** — This milestone is refactoring-only, no new capabilities
- **Breaking API changes** — All public APIs must remain stable
- **Major architectural changes** — Keep existing patterns, improve implementation
- **GPU implementation completion** — GPU benchmark TODO remains for future milestone
- **CART index leaf splitting** — Incomplete feature remains for future milestone
- **Database migrations** — Focus on code quality, not data migration tools

## Context

**Technical Environment:**
- Rust 1.83+ workspace with 8 crates
- Core library: `velesdb-core` with HNSW, SIMD, storage, query engine
- Bindings: WASM (wasm-bindgen), Python (PyO3), Tauri plugin
- Testing: Unit tests, integration tests, loom concurrency tests, fuzz targets
- CI/CD: cargo fmt, clippy, deny, test gates

**Existing Codebase State:**
- Comprehensive codebase map exists in `.planning/codebase/`
- Known concerns documented: tech debt, security risks, performance bottlenecks
- Quality gates defined but some globally disabled via `#[allow]`
- Multiple large modules exceeding 500-line threshold
- SIMD code has 100+ unsafe blocks requiring careful audit

**Refactoring Standards:**
- Follow Rust Book best practices: https://doc.rust-lang.org/book/
- Apply Martin Fowler refactoring patterns
- Maintain zero breaking changes to public APIs
- All existing tests must pass
- Benchmarks should show improvement or maintain parity

**Known Pain Points:**
1. `simd_native.rs` ~2400 lines — needs modularization
2. Global clippy allows masking potential bugs
3. Complex HNSW lock ordering — fragile to modification
4. Multiple BUG-XXX comments in VelesQL parser
5. Production code using `eprintln!` instead of tracing
6. Numeric cast patterns using `as` instead of `try_from()`

## Constraints

- **Tech Stack**: Rust 1.83+ only, maintain existing crate structure
- **Compatibility**: Zero breaking changes to public APIs
- **Quality Gates**: All must pass — `cargo fmt`, `clippy -D warnings`, `deny check`, `test --workspace`
- **Performance**: Benchmarks must not regress (ideally improve)
- **Timeline**: Methodical refactoring, not rushed — quality over speed
- **Safety**: All unsafe code must have documented invariants and SAFETY comments

## Key Decisions

| Decision | Rationale | Outcome |
|----------|-----------|---------|
| Zero breaking changes | Existing users rely on current APIs | — Pending |
| Martin Fowler patterns | Industry-standard refactoring approach | — Pending |
| Rust Book as reference | Authoritative Rust best practices | — Pending |
| All quality gates enforced | Prevent tech debt accumulation | — Pending |
| Module size limit (500 lines) | AGENTS.md guideline for maintainability | — Pending |

---
*Last updated: 2026-02-06 after initialization*
