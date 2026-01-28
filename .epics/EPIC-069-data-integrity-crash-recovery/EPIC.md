# EPIC-069: Data Integrity & Crash Recovery

## Status: TODO
## Priority: CRITIQUE
## Sprint: 2026-Q1

## Vision

Garantir la durabilité des données et la récupération après crash.

## Problème Actuel

- `Collection::flush()` appelle `mmap.flush()` mais pas `file.sync_all()` sur Windows
- `vacuum.rs` ignore certains error codes
- Risque de perte de données après coupure de courant

## User Stories

| US | Description | Priorité | Estimation |
|----|-------------|----------|------------|
| US-001 | Ajouter sync_all() après flush sur Windows | CRITIQUE | 2h |
| US-002 | Propager erreurs vacuum correctement | HAUTE | 2h |
| US-003 | Tests de crash recovery avec kill -9 | MOYENNE | 4h |

## Fichiers Critiques

- `storage/mmap.rs` - flush() sans sync_all()
- `index/hnsw/index/vacuum.rs` - erreurs ignorées
- `collection/core/lifecycle.rs` - flush orchestration

## Critères d'Acceptation

- [ ] `sync_all()` appelé après chaque flush critique
- [ ] Toutes les erreurs de vacuum propagées
- [ ] Tests de crash recovery passent
- [ ] Documentation des garanties de durabilité
