---
trigger: always_on
---

# Sécurité Obligatoire VelesDB

## Validation des Entrées

- Valider TOUTES les entrées utilisateur (taille, format, bornes)
- Utiliser des types forts (newtypes) pour les données sensibles
- Sanitizer les strings avant utilisation dans paths/queries

## Code Unsafe

- `unsafe` interdit sauf justification documentée en commentaire
- Chaque bloc `unsafe` doit avoir un `// SAFETY:` expliquant pourquoi c'est sûr
- Review obligatoire pour tout nouveau bloc `unsafe`

## Secrets & Configuration

- Secrets via variables d'environnement UNIQUEMENT
- Jamais de credentials en dur (même en tests)
- Utiliser `.env.example` pour documenter les variables requises
- Ne jamais logger de données sensibles

## Dépendances

- `cargo deny check` avant tout merge
- Pas de dépendances avec vulnérabilités connues
- Préférer les crates bien maintenus et audités
- Documenter pourquoi une dépendance est ajoutée

## Gestion d'Erreurs

- Pas de `unwrap()` ou `expect()` sans message en production
- Utiliser `thiserror` ou `anyhow` pour les erreurs
- Ne pas exposer les détails d'erreurs internes aux utilisateurs
- Logger les erreurs avec contexte suffisant pour debug
