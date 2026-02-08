# Phase 4: Complexity Reduction & Error Handling — Context

**Captured:** 2026-02-08

## Vision

Phase 4 doit atteindre la **qualité logicielle production-grade** sur l'ensemble du crate `velesdb-core`. L'objectif n'est pas de corriger quelques points isolés, mais d'appliquer exhaustivement les standards Rust à tout le code : zéro fichier >500 lignes, zéro warning clippy pedantic, zéro panic en production, erreurs structurées partout.

Après Phase 4, le code doit être un exemple de ce à quoi ressemble un projet Rust bien maintenu.

## User Experience

Un développeur qui ouvre n'importe quel fichier du projet doit trouver :
- Un module focalisé (<500 lignes) avec une responsabilité claire
- Des erreurs typées et documentées (pas de `format!("...")` anonymes)
- Zéro `panic!`, `unwrap()`, ou `expect()` injustifié en production
- Du code idiomatique Rust moderne (`let...else`, `From` au lieu de `as`, etc.)

## Essentials

Ce qui DOIT être vrai après Phase 4 :
- [ ] **Zéro fichier >500 lignes** — les 20 fichiers identifiés sont splittés en sous-modules
- [ ] **Zéro warning clippy pedantic** — les 476 warnings sont corrigés
- [ ] **`clippy::pedantic` activé comme lint workspace** — tolérance zéro sur les nouveaux warnings
- [ ] **Zéro panic/expect injustifié** — conversion en `Result` avec contexte structuré
- [ ] **57 fonctions documentées avec `# Errors`** — chaque `fn → Result` documente ses erreurs
- [ ] **64 bare-string errors convertis** — erreurs structurées avec champs typés
- [ ] **Tests GPU error handling** — fallback gracieux, validation paramètres, edge cases

## Boundaries

Ce qu'on NE fait PAS :
- Pas de refactoring d'API publique (signatures stables)
- Pas de changement de logique métier pendant le splitting
- Pas de réécriture de code qui fonctionne — uniquement réorganiser et durcir
- Pas de suppression de tests existants
- Les 344 backticks dans les docs : corrigés par batch auto-fix, pas manuellement

## Implementation Notes

Préférences techniques :
- **Module splitting** : pattern directory module (`foo/mod.rs` + submodules) avec façade re-export
- **Exceptions pedantic** : documentées au cas par cas avec `#[allow(clippy::xxx)]` + commentaire `// Reason:`
- **Erreurs structurées** : utiliser les variants `thiserror` existants dans `error.rs`, ajouter les manquants
- **Format des plans** : groupés par domaine (graph, query, storage, etc.) pour minimiser les conflits
- **Waves** : error foundation → module splitting → pedantic + hardening → GPU tests

## Open Questions

Résolus pendant la discussion :
- ✅ Scope module splitting : TOUT (20 fichiers), pas seulement les pires
- ✅ Pedantic enforcement : activer en workspace lint après correction
- ✅ Exceptions : documentées dans AGENTS.md au cas par cas

---
*Ce contexte guide la planification. Les plans respecteront ces préférences.*
