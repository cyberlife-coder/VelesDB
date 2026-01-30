# Phase 3 - SIMD Cleanup arXiv (90 min)

Références scientifiques:
- PDX Layout (arXiv:2503.04422, Kuffo et al., 2025) - 40% plus rapide que SIMD horizontal
- simdjson (arXiv:1902.08318, Langdale & Lemire, 2019) - prefetch + branch-free
- Hyperscan Teddy (NSDI'19) - SIMD string matching

## Suppression code mort AVX-512

- [ ] Supprimer le fichier `crates/velesdb-core/src/simd_avx512.rs` entièrement (code scalaire déguisé en SIMD)
- [ ] Ouvrir `crates/velesdb-core/src/lib.rs` et supprimer la ligne `mod simd_avx512;` ou `pub mod simd_avx512;`
- [ ] Chercher dans tout le codebase les références à `simd_avx512` et les supprimer
- [ ] Exécuter `cargo check -p velesdb-core` pour vérifier qu'aucune référence ne subsiste

## PDX Distance (arXiv:2503.04422)

- [ ] Créer le fichier `crates/velesdb-core/src/simd/pdx_distance.rs`
- [ ] Ajouter le doc comment `//! PDX-inspired vertical layout for vector distance computation. Key insight: process dimension-by-dimension across multiple vectors.`
- [ ] Définir `const PDX_BLOCK_SIZE: usize = 64;` pour alignement cache L1
- [ ] Implémenter `pub fn pdx_l2_distance_batch(query: &[f32], vectors: &[&[f32]]) -> Vec<f32>` qui:
  - Initialise `let mut distances = vec![0.0f32; vectors.len()];`
  - Boucle `for d in 0..dim` (dimension par dimension)
  - Pour chaque dimension, boucle sur tous les vecteurs: `distances[i] += (query[d] - vectors[i][d]).powi(2)`
  - Applique `sqrt()` à la fin
- [ ] Ajouter `#[repr(C, align(64))] pub struct PdxBlock { pub data: Vec<f32>, pub num_vectors: usize, pub dimension: usize }`

## Teddy Trigrams (Hyperscan-inspired)

- [ ] Créer le fichier `crates/velesdb-core/src/simd/teddy_trigrams.rs`
- [ ] Ajouter `use std::collections::HashSet;`
- [ ] Implémenter `pub fn extract_trigrams_scalar(bytes: &[u8]) -> HashSet<[u8; 3]>`:
  - `if bytes.len() < 3 { return HashSet::new(); }`
  - `let mut trigrams = HashSet::with_capacity(bytes.len().min(1024));`
  - `for i in 0..bytes.len() - 2 { trigrams.insert([bytes[i], bytes[i + 1], bytes[i + 2]]); }`
- [ ] Ajouter `#[cfg(target_arch = "x86_64")] use std::arch::x86_64::*;`
- [ ] Implémenter `#[cfg(target_arch = "x86_64")] #[target_feature(enable = "avx2")] pub unsafe fn extract_trigrams_avx2(bytes: &[u8]) -> HashSet<[u8; 3]>`:
  - Prefetch avec `_mm_prefetch(bytes.as_ptr().add(i + 64) as *const i8, _MM_HINT_T0)`
  - Charger 32 bytes avec `_mm256_loadu_si256`
  - Extraire trigrams du chunk chargé
  - Tail scalar pour les bytes restants

## Runtime Dispatch

- [ ] Créer le fichier `crates/velesdb-core/src/simd/dispatch.rs`
- [ ] Ajouter `use std::sync::OnceLock;`
- [ ] Définir `#[derive(Clone, Copy, PartialEq)] pub enum SimdLevel { Scalar, Sse42, Avx2, Neon }`
- [ ] Ajouter `static DETECTED_LEVEL: OnceLock<SimdLevel> = OnceLock::new();`
- [ ] Implémenter `pub fn simd_level() -> SimdLevel` qui détecte une fois au démarrage:
  - `#[cfg(target_arch = "x86_64")] { if is_x86_feature_detected!("avx2") { return SimdLevel::Avx2; } }`
  - `#[cfg(target_arch = "aarch64")] { return SimdLevel::Neon; }`
  - Fallback `SimdLevel::Scalar`
- [ ] Implémenter `pub fn extract_trigrams(bytes: &[u8]) -> HashSet<[u8; 3]>` qui dispatch selon `simd_level()`

## Module SIMD restructuré

- [ ] Créer ou modifier `crates/velesdb-core/src/simd/mod.rs` pour exporter:
  - `pub mod pdx_distance;`
  - `pub mod teddy_trigrams;`
  - `pub mod dispatch;`
  - `pub use dispatch::{simd_level, SimdLevel, extract_trigrams};`
  - `pub use pdx_distance::pdx_l2_distance_batch;`

## Benchmarks Criterion

- [ ] Créer `crates/velesdb-core/benches/simd_bench.rs`
- [ ] Ajouter `use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};`
- [ ] Implémenter `fn bench_trigrams(c: &mut Criterion)` qui teste scalar vs avx2 sur [100, 1000, 10000, 100000] bytes
- [ ] Implémenter `fn bench_pdx_distance(c: &mut Criterion)` qui teste sur dimensions [128, 384, 768, 1536]
- [ ] Ajouter `criterion_group!(benches, bench_trigrams, bench_pdx_distance);` et `criterion_main!(benches);`
- [ ] Ajouter dans `crates/velesdb-core/Cargo.toml` section `[[bench]] name = "simd_bench" harness = false`

## Validation

- [ ] Exécuter `cargo check -p velesdb-core` pour vérifier compilation
- [ ] Exécuter `cargo test -p velesdb-core simd` pour tester
- [ ] Exécuter `cargo bench --bench simd_bench -- --save-baseline before` pour baseline
- [ ] Vérifier avec `cargo asm -p velesdb-core --lib -- extract_trigrams_avx2` que des instructions AVX2 sont générées
