---
name: complete-epic
description: Finalise une EPIC et renomme son dossier avec -done lorsque toutes les US sont complétées
---

# /complete-epic EPIC-XXX

Vérifie que toutes les User Stories d'une EPIC sont complètes et renomme le dossier avec le suffixe `-done`.

## Étape 1: Lecture EPIC

Lire `.epics/EPIC-XXX-nom/EPIC.md` pour récupérer:
- Liste des US
- Objectifs
- Definition of Done de l'EPIC

## Étape 2: Vérification Status US

Pour **chaque US** listée dans l'EPIC:

1. Lire `.epics/EPIC-XXX-nom/US-YYY.md`
2. Vérifier que le status est `✅ DONE` ou `🟢 DONE`
3. Vérifier que tous les critères d'acceptation (AC-X) sont cochés
4. Vérifier que la DoD est complète

**Si une US n'est pas DONE:**
```
❌ EPIC ne peut pas être clôturée
US non complètes:
- US-002: 🔴 TODO
- US-005: 🟡 IN PROGRESS
```
→ Arrêter le workflow.

## Étape 3: Validation Tests

// turbo
```powershell
cargo test --workspace
```

**Tous les tests doivent passer.** Si échec → arrêter.

## Étape 4: Validation Qualité

// turbo
```powershell
cargo fmt --all -- --check
cargo clippy -- -D warnings
cargo deny check
```

**Aucune erreur tolérée.** Si échec → arrêter.

## Étape 5: Mise à jour EPIC.md

Modifier `.epics/EPIC-XXX-nom/EPIC.md`:
- Cocher tous les objectifs
- Cocher toutes les US dans le tableau (Status: ✅ DONE)
- Cocher tous les items de la Definition of Done
- Ajouter date de completion

Exemple mise à jour:
```markdown
## 📅 Dates

- **Créée**: 2026-01-24
- **Complétée**: 2026-01-XX  ← AJOUTER
- **Estimation**: X jours
```

## Étape 6: Renommage Dossier

Renommer le dossier EPIC avec le suffixe `-done`:

```powershell
# Depuis la racine du projet
$oldName = ".epics\EPIC-XXX-nom"
$newName = ".epics\EPIC-XXX-nom-done"

# Vérifier que le dossier -done n'existe pas déjà
if (Test-Path $newName) {
    Write-Error "Le dossier $newName existe déjà!"
    exit 1
}

# Renommer
Rename-Item -Path $oldName -NewName (Split-Path $newName -Leaf)
Write-Host "✅ EPIC renommée: $newName"
```

## Étape 7: Mise à jour Git

```powershell
git add .epics/
git commit -m "docs(epic): close EPIC-XXX - all US completed"
```

## Étape 8: Mise à jour CHANGELOG

Ajouter une entrée dans `CHANGELOG.md` section appropriée:
```markdown
### Changed
- **EPIC-XXX**: [Titre] - Completed (X US)
```

## Étape 9: Résumé Final

Afficher:
```
✅ EPIC-XXX clôturée avec succès!

📊 Statistiques:
- US complétées: X/X (100%)
- Tests: XXX passés
- Durée effective: Y jours

📁 Dossier renommé:
.epics/EPIC-XXX-nom → .epics/EPIC-XXX-nom-done

🔗 Commit: [hash]

📋 Prochaines EPICs suggérées:
- EPIC-YYY: [Titre]
```

## Conditions de Blocage

Le workflow **REFUSE** de clôturer si:

| Condition | Action |
|-----------|--------|
| US non DONE | Lister les US manquantes |
| Tests échouent | Afficher erreurs |
| Clippy warnings | Afficher warnings |
| cargo deny échec | Afficher vulnérabilités |
| Dossier -done existe | Erreur de duplication |

## Notes

- Ce workflow est **final** - il marque définitivement l'EPIC comme terminée
- Le renommage `-done` permet de filtrer facilement les EPICs actives vs terminées
- L'historique Git conserve le renommage proprement
