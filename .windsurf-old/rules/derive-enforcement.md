---
title: Dérivations obligatoires
---

## Objectif
Garantir que tous les types publics exposent des traits basiques facilitant le debug, les tests et l’intégration.

## Règle
1. **Structures et enums publiques** (`pub struct`, `pub enum`) doivent dériver par défaut:
   - `Debug`
   - `Clone`
   - `PartialEq`
2. Ajouter `Eq`, `Hash`, `Copy`, `Default` selon le contexte (mais documenter le choix).
3. Si un type ne peut pas dériver l’un de ces traits, ajouter un commentaire `// Reason:` expliquant pourquoi.
4. Ajouter une doctest illustrant l’usage si le type est destiné à être instancié directement par les utilisateurs.

## Checklist PR
- [ ] Tous les nouveaux types publics dérivent `Debug + Clone + PartialEq`.
- [ ] Commentaire `// Reason:` pour chaque exception.
- [ ] Tests/doctests mettent en évidence les traits dérivés.
