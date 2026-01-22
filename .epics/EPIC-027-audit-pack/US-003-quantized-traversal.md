# US-003: Quantized Distances pour Graph Traversal

## Description

Utiliser distances quantifiées pour le graph traversal dans DualPrecisionIndex.

## Contexte

`dual_precision.rs:189` - TODO pour optimiser traversal avec distances quantifiées.

## Critères d'Acceptation

### AC-1: Quantized Traversal
```rust
// Utiliser f16 ou int8 pour le traversal initial
// Rerank avec f32 uniquement pour top-k final
```

### AC-2: Benchmark
Benchmark montrant:
- Latence améliorée pour traversal
- Recall maintenu > 95%

### AC-3: Config Option
```rust
pub struct DualPrecisionConfig {
    pub use_quantized_traversal: bool,
    pub rerank_factor: usize,
}
```

## Fichiers Impactés

- `crates/velesdb-core/src/index/hnsw/native/dual_precision.rs`

## Definition of Done

- [ ] Traversal quantifié implémenté
- [ ] Recall validé > 95%
- [ ] Benchmark performance
- [ ] Config option documentée

## Estimation

- **Complexité**: L (Large)
- **Effort**: 6h

## Priorité

**P2 - Medium** - Optimisation performance significative
