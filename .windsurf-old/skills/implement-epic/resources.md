# Ressources @implement-epic

## Workflows Orchestrés

Ce skill orchestre les workflows suivants dans l'ordre :

| Workflow | Rôle | Invocation |
|----------|------|------------|
| `/status` | État de l'EPIC | `/status EPIC-XXX` |
| `/start-us` | Démarrer une US | `/start-us EPIC-XXX/US-YYY` |
| `@implement-us` | Guide TDD | `@implement-us` |
| `/fou-furieux` | Cycle qualité 5 phases | `/fou-furieux` |
| `/pre-commit` | Validation pré-commit | `/pre-commit` |
| `/complete-us` | Finaliser une US | `/complete-us EPIC-XXX/US-YYY` |
| `/complete-epic` | Clôturer l'EPIC | `/complete-epic EPIC-XXX` |

## Fichiers Manipulés

| Fichier | Action |
|---------|--------|
| `.epics/EPIC-XXX-*/EPIC.md` | Lecture liste US |
| `.epics/EPIC-XXX-*/progress.md` | Mise à jour statuts |
| `.epics/EPIC-XXX-*/US-*.md` | Lecture critères, mise à jour status |

## Format de Commit

```
type(scope): description [EPIC-XXX/US-YYY]
```

**Types autorisés** :
- `feat` : nouvelle fonctionnalité
- `fix` : correction de bug
- `refactor` : refactoring sans changement fonctionnel
- `test` : ajout/modification de tests
- `perf` : optimisation performance
- `docs` : documentation
- `chore` : maintenance

## Diagramme du Cycle

```
┌────────────────────────────────────────────────────────────┐
│                  @implement-epic EPIC-XXX                  │
├────────────────────────────────────────────────────────────┤
│                                                            │
│  Phase 0: Initialisation                                   │
│  ├── /status EPIC-XXX                                      │
│  └── Construire liste US_TODO                              │
│                                                            │
│  Phase 1: Boucle (pour chaque US)                          │
│  ┌──────────────────────────────────────────────────────┐  │
│  │  /start-us ──► @implement-us ──► /fou-furieux        │  │
│  │       │              │                │              │  │
│  │       │              │                ▼              │  │
│  │       │              │          /pre-commit          │  │
│  │       │              │                │              │  │
│  │       │              │                ▼              │  │
│  │       │              │           git commit          │  │
│  │       │              │                │              │  │
│  │       │              │                ▼              │  │
│  │       │              └────────► /complete-us         │  │
│  │       │                               │              │  │
│  │       └───────────────────────────────┘              │  │
│  │                     ▲                                │  │
│  │                     │ Répéter pour chaque US         │  │
│  └─────────────────────┴────────────────────────────────┘  │
│                                                            │
│  Phase 2: Clôture                                          │
│  └── /complete-epic EPIC-XXX                               │
│                                                            │
│  Phase 3: Résumé                                           │
│  └── Statistiques + Prochaines actions                     │
│                                                            │
└────────────────────────────────────────────────────────────┘
```

## Reprise après Interruption

Si le skill est interrompu, l'état est sauvegardé dans `progress.md`.

Pour reprendre :
```
@implement-epic EPIC-XXX --resume
```

Le skill détectera automatiquement :
- Dernière US complétée
- US en cours (IN PROGRESS)
- US restantes (TODO)
