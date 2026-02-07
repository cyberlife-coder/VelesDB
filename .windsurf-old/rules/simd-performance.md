---
trigger: glob
globs: ["**/simd/**/*.rs", "**/index/hnsw/**/*.rs", "**/gpu/**/*.rs"]
description: Règles pour code performance-critique
---

# Code Performance-Critique

Vous modifiez du code dans un hot-path (SIMD, HNSW, GPU).

## Règles de performance

### 1. Pas d'allocation dans les boucles

```rust
// ❌ INTERDIT
for item in items {
    let temp = format!("{}", item);  // Allocation!
}

// ✅ CORRECT
let mut buffer = String::with_capacity(128);
for item in items {
    buffer.clear();
    write!(&mut buffer, "{}", item).unwrap();
}
```

### 2. Préférer les références

```rust
// ❌ ÉVITER
fn process(data: Vec<f32>) { ... }

// ✅ PRÉFÉRER
fn process(data: &[f32]) { ... }
```

### 3. Clone() doit être justifié

Si vous utilisez `.clone()` dans ce fichier, ajouter un commentaire :
```rust
// PERF: Clone necessary here because ownership is transferred to thread pool
let data = source.clone();
```

## Benchmarks obligatoires

Avant de modifier ces fichiers :

```powershell
# Baseline
cargo bench --bench <bench_name> -- --save-baseline before

# Après modification
cargo bench --bench <bench_name> -- --baseline before
```

## Métriques cibles

| Opération | Latence cible (p99) |
|-----------|---------------------|
| Distance calc (1024d) | < 1µs |
| HNSW search (10k vectors) | < 10ms |
| Batch insert (1000 vectors) | < 100ms |

## Vérification SIMD

```powershell
# Vérifier que SIMD est utilisé
RUSTFLAGS="-C target-cpu=native" cargo build --release
objdump -d target/release/libvelesdb_core.rlib | rg "vmovaps|vfmadd"
```
