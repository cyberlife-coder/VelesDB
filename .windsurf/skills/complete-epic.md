# Skill: Complete EPIC

Finalise une EPIC après que toutes les US sont complétées.

## Déclencheur

Utiliser quand:
- Toutes les US d'une EPIC sont marquées DONE
- Avant de créer la PR finale vers develop

## Checklist de Complétion

### 1. Vérifier Statut US
```
Lire .epics/EPIC-XXX-nom/progress.md
Confirmer: Complétées = Total US
```

### 2. Mettre à jour progress.md
```markdown
| Métrique | Valeur |
|----------|--------|
| Total US | N |
| Complétées | N |
| Progression | 100% |
```

### 3. Exécuter /fou-furieux
```
Boucle de validation complète sur tous les changements
```

### 4. Exécuter /pre-commit
```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo deny check
```

### 5. Mettre à jour EPIC.md
```markdown
## ✅ Definition of Done
- [x] Toutes les US complétées
- [x] Tests passent (coverage > 80%)
- [x] Documentation mise à jour
- [x] CHANGELOG.md mis à jour
- [x] Code review approuvée
```

### 6. Mettre à jour CHANGELOG.md
```markdown
## [Unreleased]

### Added
- Feature X (EPIC-XXX)

### Changed
- ...
```

### 7. Lancer Expert Review
```
/expert-review pour validation finale multi-experts
```

### 8. Créer/Mettre à jour PR
```
Titre: feat(scope): EPIC-XXX description
Body: Inclure résumé des US et résultats review
```

## Output

Message de confirmation avec:
- Résumé des US complétées
- Résultats des validations
- Lien PR si créée
