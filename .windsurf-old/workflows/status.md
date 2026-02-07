---
name: status
description: Affiche le statut global des EPICs et US en cours
---

# /status [EPIC-XXX?]

Vue d'ensemble de l'avancement projet.

## Sans argument: Vue globale

Lister toutes les EPICs dans `.epics/`:

Pour chaque EPIC:
- Nom et description courte
- Progression: X/Y US (pourcentage)
- US en cours (IN PROGRESS)
- Bloqueurs éventuels

## Avec argument: Vue EPIC

`/status EPIC-001`

Afficher détails de l'EPIC:
- Description complète
- Tableau de toutes les US avec status
- Branche Git active si applicable
- Temps estimé restant

## Format Sortie

```
# Status VelesDB

## EPICs Actives

### EPIC-001: Dashboard Audit [60%]
- US-001: Log viewer ........... DONE
- US-002: Access report ........ IN PROGRESS (feature/EPIC-001-US-002)
- US-003: Export PDF ........... TODO

### EPIC-002: Performance Optim [0%]
- US-001: Benchmark suite ...... TODO
- US-002: SIMD optimization .... TODO

## Bloqueurs
- Aucun

## Prochaines Actions
1. Continuer EPIC-001/US-002
2. Démarrer EPIC-002 après completion EPIC-001
```
