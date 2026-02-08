---
name: create-epic
description: Crée une EPIC structurée avec US et critères d'acceptation depuis une description de feature
---

# Création d'EPIC depuis Description

Quand l'utilisateur décrit une fonctionnalité souhaitée, ce skill guide la création complète.

## Phase 1: Analyse de la Demande

1. Identifier le besoin principal et la valeur métier
2. Décomposer en sous-fonctionnalités (futures US)
3. Identifier les crates/modules impactés
4. Vérifier les EPICs existantes pour éviter les doublons

## Phase 2: Création Structure EPIC

1. Déterminer le prochain ID disponible:
   - Lister les dossiers dans .epics/
   - Prendre le numéro suivant (EPIC-001, EPIC-002, etc.)

2. Créer le dossier: .epics/EPIC-XXX-nom-court/

3. Créer EPIC.md en utilisant le template .epics/_templates/EPIC.md

## Phase 3: Découpage en User Stories

Pour chaque sous-fonctionnalité identifiée:

1. Créer US-YYY-nom.md avec le template .epics/_templates/US.md

2. Définir les critères d'acceptation au format Gherkin:
   GIVEN [contexte initial]
   WHEN [action utilisateur]
   THEN [résultat attendu]

3. Estimer la complexité:
   - S: < 2h, changement isolé
   - M: 2-8h, plusieurs fichiers
   - L: 1-3 jours, nouveau module
   - XL: > 3 jours, architecture

4. Identifier les tests requis (unitaires, intégration)

## Phase 4: Initialisation Suivi

1. Créer progress.md avec le template .epics/_templates/progress.md
2. Lister toutes les US avec status TODO

## Phase 5: Validation

1. Afficher le résumé de l'EPIC créée
2. Lister les US générées
3. Demander validation/ajustements à l'utilisateur

## En cas de modification d'EPIC existante

1. Rechercher les EPICs liées au sujet
2. Proposer: créer nouvelle EPIC ou enrichir existante
3. Si enrichissement: ajouter les nouvelles US
4. Mettre à jour progress.md
