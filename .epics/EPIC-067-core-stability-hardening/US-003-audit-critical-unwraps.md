# US-003: Audit et Fix des unwrap() Critiques

## Status: TODO
## Priorité: MOYENNE
## Estimation: 3h

## Description

Auditer tous les `unwrap()` dans le code de production (hors tests) et remplacer par `?` ou `expect("message contextualisé")` selon le cas.

## Contexte

Les `unwrap()` en production peuvent causer des panics inattendus. Ils doivent être:
- Remplacés par `?` si l'erreur peut être propagée
- Remplacés par `expect("contexte")` si le panic est intentionnel
- Documentés si le contexte garantit le succès

## Fichiers Prioritaires (hors tests)

1. `storage/async_ops.rs` - 7 occurrences
2. `lib.rs` - dans doc examples (OK)
3. `collection/core/statistics.rs` - 10 occurrences (tests)

## Pattern de Correction

```rust
// AVANT
let value = map.get(&key).unwrap();

// APRÈS (si erreur possible)
let value = map.get(&key).ok_or(Error::KeyNotFound(key))?;

// APRÈS (si garanti par invariant)
let value = map.get(&key).expect("key always exists after initialization");
```

## Critères d'Acceptation

- [ ] Aucun `unwrap()` sur données utilisateur
- [ ] Tous les `unwrap()` restants ont un commentaire justificatif
- [ ] Tests de cas d'erreur ajoutés
- [ ] `cargo clippy -- -W clippy::unwrap_used` passe (ou exceptions documentées)

## Exclusions

- Fichiers `*_tests.rs` - `unwrap()` acceptable dans les tests
- Doc examples dans `///` - `unwrap()` acceptable pour clarté
