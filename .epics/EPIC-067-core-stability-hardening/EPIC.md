# EPIC-067: Core Stability Hardening

## Status: IN PROGRESS
## Priority: CRITICAL
## Sprint: 2026-Q1

## Vision

Éliminer tous les patterns dangereux qui peuvent causer des panics en production :
- `partial_cmp().unwrap()` sur floats (NaN → panic)
- Truncating casts (`as u32`) sans bounds check (overflow silencieux)
- `unwrap()` sur données utilisateur

## Problème Actuel

Scan du codebase révèle :
- **19 fichiers** avec `partial_cmp().unwrap()` ou `.unwrap_or(Ordering::Equal)`
- **53 occurrences** de truncating casts (`as u32/u16/u8`)
- Risque de panic sur NaN dans les tris de scores de similarité

## User Stories

| US | Description | Priorité | Estimation |
|----|-------------|----------|------------|
| US-001 | Remplacer partial_cmp().unwrap() par total_cmp() | CRITIQUE | 2h |
| US-002 | Sécuriser truncating casts avec try_from/bounds check | HAUTE | 4h |
| US-003 | Audit et fix des unwrap() critiques en production | MOYENNE | 3h |

## Critères d'Acceptation Globaux

- [ ] `cargo clippy -- -D warnings` passe
- [ ] Aucun `partial_cmp().unwrap()` dans le code (hors tests)
- [ ] Tous les casts `as uX` ont un bounds check ou `#[allow]` documenté
- [ ] Tests de régression pour chaque fix
- [ ] Benchmarks avant/après pour hot-paths

## Fichiers Impactés

### partial_cmp (US-001)
- `fusion/strategy.rs` (4 occurrences)
- `distance.rs` (2 occurrences)
- `collection/search/text.rs` (2 occurrences)
- `index/bm25.rs` (3 occurrences)
- `index/hnsw/native/dual_precision.rs` (1 occurrence)
- `collection/search/query/parallel_traversal.rs` (1 occurrence)

### Truncating casts (US-002)
- `index/hnsw/native/backend_adapter.rs` (8 occurrences)
- `index/hnsw/native/quantization.rs` (6 occurrences)
- `collection/auto_reindex/mod.rs` (5 occurrences)
- `index/trigram/index.rs` (4 occurrences)

## Dépendances

- Aucune dépendance externe
- Impact sur tous les SDKs (Python, WASM, TS) si API change

## Risques

| Risque | Mitigation |
|--------|------------|
| Régression performance | Benchmarks obligatoires |
| Breaking change API | Pas de changement d'API publique |
