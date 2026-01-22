# US-002: Implémenter AVX-512 VPOPCNTDQ

## Description

Implémenter true AVX-512 VPOPCNTDQ pour Hamming distance au lieu du fallback.

## Contexte

`simd_dispatch.rs:340` délègue actuellement à `hamming_popcnt` régulier.

## Critères d'Acceptation

### AC-1: Detection CPU Feature
```rust
#[cfg(all(target_arch = "x86_64", target_feature = "avx512vpopcntdq"))]
fn hamming_avx512_vpopcntdq(a: &[f32], b: &[f32]) -> u32 {
    // True AVX-512 implementation
}
```

### AC-2: Runtime Detection
Détecter au runtime si VPOPCNTDQ est disponible via `std::is_x86_feature_detected!`.

### AC-3: Benchmark
Benchmark montrant amélioration vs fallback popcnt.

## Fichiers Impactés

- `crates/velesdb-core/src/simd_dispatch.rs`
- `crates/velesdb-core/src/simd_avx512.rs`

## Definition of Done

- [ ] AVX-512 VPOPCNTDQ implémenté
- [ ] Runtime detection fonctionnel
- [ ] Fallback gracieux si non disponible
- [ ] Benchmark validant performance

## Estimation

- **Complexité**: M (Medium)
- **Effort**: 3h

## Priorité

**P3 - Low** - Optimisation performance, non critique
