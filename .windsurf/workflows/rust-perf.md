---
description: Optimiser les performances d'une fonction Rust avec benchmarks
---

# Workflow : Optimisation Performance

## 1. Baseline

1. Identifier la fonction à optimiser
2. Vérifier qu'un benchmark existe ou le créer dans `benches/` :
   ```rust
   use criterion::{black_box, criterion_group, criterion_main, Criterion};

   fn bench_my_function(c: &mut Criterion) {
       c.bench_function("my_function", |b| {
           b.iter(|| {
               black_box(my_function(input))
           });
       });
   }

   criterion_group!(benches, bench_my_function);
   criterion_main!(benches);
   ```

3. Lancer le benchmark pour avoir la baseline :
```powershell
cargo bench --bench search_benchmark
```

4. Noter les résultats (temps, throughput)

## 2. Profiling

1. Utiliser `cargo flamegraph` si disponible :
```powershell
cargo install flamegraph
cargo flamegraph --bench search_benchmark
```

2. Identifier les hotspots dans le code

## 3. Optimisation

Stratégies courantes :
- **Éviter les allocations** : Réutiliser les buffers, `Vec::with_capacity()`
- **SIMD** : Utiliser des opérations vectorisées pour les calculs sur vecteurs
- **Cache locality** : Organiser les données pour accès séquentiel
- **Parallelism** : Utiliser `rayon` pour paralléliser

Exemple :
```rust
// Avant (allocation à chaque appel)
fn compute(data: &[f32]) -> Vec<f32> {
    data.iter().map(|x| x * 2.0).collect()
}

// Après (réutilisation buffer)
fn compute_into(data: &[f32], output: &mut Vec<f32>) {
    output.clear();
    output.extend(data.iter().map(|x| x * 2.0));
}
```

## 4. Mesure

1. Relancer le benchmark :
```powershell
cargo bench --bench search_benchmark
```

2. Comparer avec la baseline
3. Documenter l'amélioration

## 5. Validation

1. S'assurer que le comportement est identique :
// turbo
```powershell
cargo test --all-features
```

2. Vérifier clippy :
// turbo
```powershell
cargo clippy --all-targets --all-features -- -D warnings
```

## 6. Documentation

1. Ajouter un commentaire expliquant l'optimisation :
   ```rust
   // Perf: Utilise un buffer pré-alloué pour éviter 
   // les allocations dans la boucle principale.
   // Amélioration: 2.3x sur vecteurs 768d
   ```
