---
name: implement-epic
description: Orchestre l'implémentation complète d'une EPIC avec toutes ses US en boucle TDD vérifiée
---

# Implémentation Complète d'une EPIC

Ce skill orchestre le cycle complet d'implémentation de TOUTES les US d'une EPIC, de manière ultra-complète et vérifiée.

## Invocation

```
@implement-epic EPIC-XXX
```

## Phase 0: Initialisation

1. Lire `.epics/EPIC-XXX-*/EPIC.md` pour récupérer :
   - Liste des US
   - Objectifs de l'EPIC
   - Dépendances

2. Invoquer `/status EPIC-XXX` pour afficher l'état actuel :
   - US déjà DONE
   - US IN PROGRESS
   - US TODO

3. Construire la liste ordonnée des US à implémenter :
   ```
   US_TODO = [US où status != DONE]
   ```

4. Demander confirmation :
   ```
   📋 EPIC-XXX contient X US dont Y à implémenter.
   Voulez-vous commencer l'implémentation complète ? (oui/non)
   ```

## Phase 1: Boucle d'Implémentation

**Pour CHAQUE US dans US_TODO :**

### Étape 1.1: Démarrage US
```
Invoquer: /start-us EPIC-XXX/US-YYY
```
- Crée branche `feature/EPIC-XXX-US-YYY`
- Affiche critères d'acceptation
- Met progress.md → IN PROGRESS

### Étape 1.2: Implémentation TDD
```
Invoquer: @implement-us
```
- Phase RED : écrire tests qui échouent
- Phase GREEN : implémenter le minimum
- Phase REFACTOR : nettoyer le code

### Étape 1.3: Contrôle Qualité Intensif
```
Invoquer: /fou-furieux
```
Boucle jusqu'à succès :
1. 🔴 Debug (tests passent ?)
2. 🟡 Code Smells (taille, DRY)
3. 🟠 Sécurité (unsafe, cargo deny)
4. 🔵 Performance (complexité, allocs)
5. 🟣 Multithreading (locks, races)

### Étape 1.4: Validation Pré-commit
```
Invoquer: /pre-commit
```
- cargo fmt --check
- cargo clippy -- -D warnings
- cargo test --workspace
- cargo deny check

### Étape 1.5: Commit
```powershell
git add -A
git commit -m "feat(scope): description [EPIC-XXX/US-YYY]"
```
Demander à l'utilisateur de valider le message de commit.

### Étape 1.6: Finalisation US
```
Invoquer: /complete-us EPIC-XXX/US-YYY
```
- Met progress.md → DONE
- Met US-YYY.md → DONE
- Vérifie si toutes les US sont DONE

### Étape 1.7: Point de Contrôle

Afficher :
```
✅ US-YYY terminée !

📊 Progression EPIC-XXX : X/Y US (XX%)
📝 Prochaine US : US-ZZZ - [titre]

Continuer avec la prochaine US ? (oui/non/pause)
```

- **oui** : continuer avec US suivante
- **non** : arrêter le skill
- **pause** : sauvegarder l'état pour reprendre plus tard

## Phase 2: Clôture EPIC

Quand toutes les US sont DONE :

```
Invoquer: /complete-epic EPIC-XXX
```
- Vérifie 100% US = DONE
- Valide tests, clippy, deny
- Renomme dossier → `EPIC-XXX-nom-done`
- Commit final

## Phase 3: Résumé Final

Afficher :
```
🎉 EPIC-XXX TERMINÉE !

📊 Statistiques :
- US implémentées : X
- Commits : Y
- Durée totale : Z heures
- Tests ajoutés : N

📁 Dossier : .epics/EPIC-XXX-nom-done/

🔗 Prochaines actions suggérées :
- /pr-create pour créer la PR vers develop
- /ecosystem-sync si API publique modifiée
```

## Gestion des Erreurs

### Si /fou-furieux échoue
1. Afficher les problèmes détectés
2. Proposer corrections
3. Après correction manuelle → reprendre à l'étape 1.3

### Si tests échouent
1. Afficher les tests en échec
2. Proposer de débugger avec l'utilisateur
3. Après correction → reprendre à l'étape 1.2

### Si l'utilisateur veut pause
1. Sauvegarder l'état dans `.epics/EPIC-XXX-*/progress.md`
2. Noter la dernière US complétée
3. Permettre de reprendre avec `@implement-epic EPIC-XXX --resume`

## Options

| Option | Description |
|--------|-------------|
| `--resume` | Reprendre depuis la dernière US non terminée |
| `--dry-run` | Afficher le plan sans exécuter |
| `--skip-fou-furieux` | Sauter le cycle qualité intensif (déconseillé) |
| `--auto-commit` | Ne pas demander confirmation pour les commits |
