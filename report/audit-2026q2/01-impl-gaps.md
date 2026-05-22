# Audit 2026-Q2 — Angle 1: Implementation Gaps

**Worktree**: `feat/code-health-audit-2026q2` (basé sur `origin/develop` @ bb0aa5ad)
**Date**: 2026-05-22
**Méthode**: Grep direct sur `crates/*/src/**.rs`, exclusion `_tests.rs` et `tests/`.

## Synthèse — l'hygiène est meilleure que ce que la mémoire v5 (42 jours) suggérait

| Catégorie | Mémoire v5 | État actuel | Évolution |
|---|---|---|---|
| Bare `TODO`/`FIXME`/`HACK` non gouvernés | F11: 4 wildcards silencieux | **0** non gouvernés en prod | ✅ résolu |
| `unimplemented!()`/`todo!()` en prod | 0 | **0** | ✅ stable |
| `panic!()` en prod (hors tests) | 0 | **0** | ✅ stable |
| `std::sync::Mutex/RwLock` en prod | implicite 0 | **0** (4 occurrences uniquement dans `auto_reindex/tests.rs`) | ✅ stable |
| `#[allow(dead_code)]` en prod | ~60 | **65 occurrences sur 30 fichiers** | ➡️ stable |
| `FIXME(PRE-SEED)` en prod | 4 (F11) | **2** (wasm/parsing.rs:134, wasm/fusion.rs:49) | 🟢 50% résolu |

## Findings détaillés

### 1. Scaffolded-but-unused — `#[allow(dead_code)]` × 65 sur 30 fichiers

Top concentration (vérifié par `Grep -count`):

| Fichier | Occurrences | Catégorie |
|---|---|---|
| [crates/velesdb-core/src/collection/graph/property_index/range.rs](crates/velesdb-core/src/collection/graph/property_index/range.rs) | 9 | scaffolding feature graph property index |
| [crates/velesdb-core/src/collection/search/sparse.rs](crates/velesdb-core/src/collection/search/sparse.rs) | 6 | scaffolding sparse search |
| [crates/velesdb-core/src/collection/graph/property_index/composite.rs](crates/velesdb-core/src/collection/graph/property_index/composite.rs) | 5 | scaffolding composite index |
| [crates/velesdb-cli/src/session.rs](crates/velesdb-cli/src/session.rs) | 4 | CLI session state |
| [crates/velesdb-core/src/collection/graph/property_index/advisor.rs](crates/velesdb-core/src/collection/graph/property_index/advisor.rs) | 3 | advisor non branché |

**Severity**: MEDIUM (LOW individuellement, MEDIUM en agrégat — risque de drift)
**Action recommandée**: Audit ciblé sur `property_index/` (17 occurrences sur 3 fichiers) — soit brancher, soit supprimer. Voir Angle 5 pour le contexte architectural.

### 2. Bare TODO/FIXME/HACK non gouvernés

**Severity**: NONE — gouvernance CI fonctionne. `scripts/check-todo-annotations.py` rejette les TODO bare.

Recherche pattern `// (TODO|FIXME|HACK|XXX|BUG)` SANS `(...)`: **0 résultat** dans `crates/*/src/**.rs`.

### 3. Wildcard fallthroughs silencieux

#### F-1: [`crates/velesdb-wasm/src/parsing.rs:135`](crates/velesdb-wasm/src/parsing.rs:135)
```rust
const fn core_to_wasm_storage_mode(core: velesdb_core::StorageMode) -> StorageMode {
    match core {
        velesdb_core::StorageMode::Full => StorageMode::Full,
        // ... 4 autres variants explicites ...
        velesdb_core::StorageMode::RaBitQ => StorageMode::RaBitQ,
        // FIXME(PRE-SEED): New StorageMode variants silently map to Full.
        _ => StorageMode::Full,
    }
}
```

**Analyse**: `velesdb_core::StorageMode` est `#[non_exhaustive]` (5 variants actuels, tous mappés explicitement). Le `_ =>` est requis par Rust mais maps silencieusement les nouveaux variants à `Full` — un user qui sélectionnerait un nouveau mode pourrait croire travailler en `SQ8` et obtenir `Full` sans erreur.

**Severity**: LOW (tous les variants actuels sont couverts; pas de bug actuel; risque uniquement à l'ajout futur).
**Fix**: Remplacer `_ => StorageMode::Full` par `_ => StorageMode::Full // unknown variant — fail-closed to Full; tracked by compile_error pattern below`, ajouter un `compile_error!` via macro qui force la mise à jour du tableau à chaque nouvelle variante (Strategy Pattern: enum dispatch via méthode `velesdb_core::StorageMode::canonical_name()` déjà en place — l'utiliser plutôt qu'un mapping local). **OU** supprimer le FIXME (devenu obsolète).

#### F-2: [`crates/velesdb-wasm/src/fusion.rs:49`](crates/velesdb-wasm/src/fusion.rs:49)
```rust
match strategy.to_lowercase().as_str() {
    "average" | "avg" => fuse_average(&scores),
    "maximum" | "max" => fuse_maximum(&scores),
    "weighted" => fuse_weighted(&scores, all_results.len()),
    "relative_score" | "rsf" => fuse_relative_score(all_results),
    "rrf" => fuse_rrf(&ranks, rrf_k),
    // FIXME(PRE-SEED): New fusion strategies must be added here explicitly.
    _ => { return Err(...) }  // ✅ renvoie Err explicite
}
```

**Severity**: **FALSE POSITIVE** — le wildcard renvoie déjà `Err`. Le FIXME est obsolète et devrait être supprimé.
**Fix**: Retirer le commentaire FIXME (code correct).

### 4. `unimplemented!()` / `todo!()` / `panic!()` en prod

**Résultat**: 0 occurrence en code prod.

Tous les `panic!()` trouvés (40+) sont dans `_tests.rs` ou `#[cfg(test)]` (acceptable). Exemples: `cache/deadlock_tests.rs` (panic explicite si deadlock détecté = test legitime).

### 5. `.unwrap()` en code prod sans justification

**Volume**: l'agent angle 1 indiquait 839 unwrap() workspace-wide; à filtrer par production-only. Hors scope grep simple — à traiter par lizard/clippy run dédié (Angle 2).

### 6. Public APIs retournant `Ok(())` sans travail

Recherche heuristique nécessaire — couvert par Angle 5 (architecture).

### 7. Re-exports vers modules stub

Couvert par Angle 5.

### 8. Cross-crate propagation — vérification de `StorageMode`

Vérifié: tous les 5 variants (`Full`, `SQ8`, `Binary`, `ProductQuantization`, `RaBitQ`) sont définis dans `crates/velesdb-core/src/quantization/mod.rs:72`. Le mapping WASM (F-1) est complet. Voir Angle 5 pour la matrice complète des autres types (DistanceMetric, SearchQuality, FusionStrategy, HnswParams, QuantizationConfig).

### 9. Observer hook stubs

`crates/velesdb-core/src/observer.rs` expose `DatabaseObserver` avec méthodes par défaut no-op — **architecture délibérée** pour extension velesdb-premium. Pas un gap; à confirmer Angle 5 que c'est documenté.

## Verdict Angle 1

**État**: ÉTAT SAIN. La mémoire v5 (42 jours) sur-estimait les gaps qui sont aujourd'hui largement résolus par v1.14.x.

**Findings HIGH résiduels**: **0**
**Findings MEDIUM**: 1 (F-1 — silent fallthrough WASM StorageMode, à durcir par Strategy Pattern)
**Findings LOW**: 2 (F-2 commentaire obsolète; cleanup ciblé property_index dead_code)
