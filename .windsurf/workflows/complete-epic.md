---
description: Finalise une EPIC après complétion de toutes les US
---

# Complétion EPIC

## Étape 1: Vérification Statut

1. Lire `.epics/EPIC-XXX-nom/progress.md`
2. Confirmer que Complétées = Total US
3. Vérifier qu'aucun bloqueur n'est actif

## Étape 2: Mise à jour progress.md

Mettre à jour les métriques:
```markdown
| Métrique | Valeur |
|----------|--------|
| Progression | 100% |
```

Mettre à jour statut de chaque US à 🟢 DONE

## Étape 3: Validation Qualité

// turbo
1. `cargo fmt --all --check`

// turbo
2. `cargo clippy --workspace --all-targets -- -D warnings`

// turbo
3. `cargo test --workspace`

// turbo
4. `cargo deny check`

## Étape 4: Review Experts

Exécuter `/expert-review` pour validation multi-experts

## Étape 5: Documentation

1. Mettre à jour CHANGELOG.md avec les nouvelles features
2. Vérifier que la documentation est à jour
3. Mettre à jour EPIC.md Definition of Done

## Étape 6: Commit Final

```
git add -A
git commit -m "docs(epic): mark EPIC-XXX as complete [EPIC-XXX]"
git push
```

## Étape 7: PR

Créer ou mettre à jour la PR vers develop avec:
- Résumé des US complétées
- Résultats des validations
- Verdict des experts
