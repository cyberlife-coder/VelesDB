---
name: local-ci
description: Validation CI complète en local AVANT push vers origin (économie GitHub Actions)
---

# /local-ci

Valide le code localement avant push vers origin pour économiser les minutes GitHub Actions.

**OBLIGATOIRE avant tout `git push origin`**

## Mode Rapide (fmt + clippy uniquement)

// turbo
```powershell
.\scripts\local-ci.ps1 -Quick
```

## Mode Complet (recommandé avant push)

// turbo
```powershell
cascade: /pre-commit -Full
```

> Cette commande appelle directement le workflow `/pre-commit` avec l'option **-Full** (fmt + clippy + tests + sécurité + benches). Le script `scripts/local-ci.ps1` reste utilisable mais n'est plus nécessaire.
## Vérifications effectuées

| # | Check | Description | Bloquant |
|---|-------|-------------|----------|
| 1 | Formatage | `cargo fmt --all --check` | ✅ Oui |
| 2 | Clippy | `cargo clippy -- -D warnings` | ✅ Oui |
| 3 | Tests | `cargo test --workspace` | ✅ Oui |
| 4 | Sécurité | `cargo deny check` | ⚠️ Warning |
| 5 | Taille fichiers | < 500 lignes par fichier | ⚠️ Warning |

## Options

| Flag | Description |
|------|-------------|
| `-Quick` | Mode rapide (fmt + clippy uniquement) |
| `-SkipTests` | Sauter les tests |
| `-SkipSecurity` | Sauter l'audit sécurité |

## Workflow de développement optimisé

```
1. Développer sur branche feature
2. git add -A && git commit -m "..."
3. /local-ci                          ← OBLIGATOIRE
4. git push origin <branch>           ← Seulement si local-ci OK
5. Créer PR sur GitHub
```

## Économies GitHub Actions

- **Avant**: ~15-20 min de CI par PR
- **Après**: CI uniquement sur push final vers main/develop
- **Économie**: ~80% des minutes GitHub Actions

## Si échec

1. Lire les erreurs affichées
2. Corriger les problèmes
3. `cargo fmt --all` si problème de formatage
4. Relancer `/local-ci`
