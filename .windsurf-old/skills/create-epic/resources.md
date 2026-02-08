# Ressources Create-Epic

Ce skill utilise les templates situés dans:
- `.epics/_templates/EPIC.md` - Template pour les EPICs
- `.epics/_templates/US.md` - Template pour les User Stories
- `.epics/_templates/progress.md` - Template pour le suivi

## Numérotation

### EPICs
- Format: `EPIC-XXX` où XXX est un numéro séquentiel (001, 002, ...)
- Dossier: `.epics/EPIC-XXX-nom-court/`

### User Stories
- Format: `US-YYY` où YYY est un numéro séquentiel par EPIC
- Fichier: `.epics/EPIC-XXX-nom/US-YYY-description.md`

## Complexité des US

| Taille | Effort | Description |
|--------|--------|-------------|
| S | < 2h | Changement isolé, un seul fichier |
| M | 2-8h | Plusieurs fichiers, même module |
| L | 1-3 jours | Nouveau module ou refactoring |
| XL | > 3 jours | Architecture, plusieurs crates |

## Critères d'Acceptation

Utiliser le format Gherkin:
`
GIVEN [contexte initial]
WHEN [action utilisateur]
THEN [résultat attendu]
`
