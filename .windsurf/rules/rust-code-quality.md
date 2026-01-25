---
trigger: glob
globs: ["**/*.rs"]
description: Checklist qualité pour code Rust - rappels Ownership, Borrowing, Lifetimes
---

# Qualité Code Rust

Vous modifiez du code Rust. Appliquez ces vérifications.

## Checklist Ownership

- [ ] Pas de "use after move" - valeur utilisée après transfert
- [ ] `clone()` justifié par commentaire si dans hot-path
- [ ] Préférer `&T` à `T` en paramètre quand possible
- [ ] Préférer `&str` à `String` en paramètre

## Checklist Borrowing

- [ ] Pas de multiple `&mut` simultanés sur la même donnée
- [ ] Emprunts scopés au minimum nécessaire
- [ ] Pas de dangling references
- [ ] Éviter de stocker des références dans des structs (préférer owned)

## Checklist Lifetimes

- [ ] Lifetimes explicites si retour de référence
- [ ] Trait bounds complets (`where` clauses si complexe)
- [ ] Pas de lifetime 'static injustifié

## Checklist Types & Conversions

- [ ] `try_from()` au lieu de `as` pour conversions avec perte possible
- [ ] Bounds check avant cast `usize` → `u32`
- [ ] `total_cmp()` pour comparaison de floats (évite panic sur NaN)

## Checklist Error Handling

- [ ] Pas de `unwrap()` sur données utilisateur → utiliser `?`
- [ ] `expect("message explicite")` si unwrap nécessaire
- [ ] Match exhaustif (pas de `_` catch-all aveugle)
- [ ] Propager les erreurs avec `?` plutôt que panic

## Checklist Traits

- [ ] `#[derive(Debug, Clone)]` quand applicable
- [ ] `#[must_use]` sur fonctions retournant des valeurs importantes
- [ ] Préférer generics statiques (`T: Trait`) à dynamiques (`dyn Trait`)

## Checklist Unsafe

- [ ] Commentaire `// SAFETY:` obligatoire expliquant pourquoi c'est sûr
- [ ] Scope minimal du bloc unsafe
- [ ] Test couvrant le code unsafe
- [ ] Alternative safe vérifiée avant d'utiliser unsafe

## Commande de validation

```powershell
cargo clippy --workspace --all-targets -- -D warnings \
  -W clippy::unwrap_used \
  -W clippy::expect_used \
  -W clippy::clone_on_ref_ptr \
  -W clippy::cast_possible_truncation
```

## Anti-Patterns fréquents

### ❌ Mauvais
```rust
let id = len as u32;  // Troncation silencieuse
collection.last().unwrap_or(0);  // 0 peut être valide
a.partial_cmp(&b).unwrap();  // Panic sur NaN
```

### ✅ Correct
```rust
let id = u32::try_from(len)?;  // Erreur explicite
collection.last().copied();  // Option propagée
a.total_cmp(&b);  // Total ordering
```

## ⚠️ Règle d'Optimisation (EPIC-018 Leçon)

**Toujours benchmarker dans le contexte RÉEL du pipeline, pas isolément.**

### Exemple concret
- `process_batch()` isolé: **46-58x** plus rapide
- Intégré dans `execute_aggregate()`: **+8% régression**
- Cause: Le parsing JSON domine (~95% du temps), pas l'agrégation

### Avant d'optimiser
1. **Profiler** pour identifier le VRAI bottleneck
2. **Benchmarker** dans le pipeline complet
3. Si régression → abandonner et documenter pourquoi

### Anti-pattern
```rust
// ❌ Optimiser un composant sans vérifier le contexte
fn optimize_aggregation() {
    // 46x faster isolé mais +8% régression en production
    // car le bottleneck est le JSON parsing, pas l'agrégation
}
```
