---
name: research-algo
description: Recherche des meilleurs algorithmes actuels sur internet et arXiv pour optimisation
---

# Recherche Algorithmique

Avant toute optimisation performance ou implémentation d'algorithme complexe.

## Phase 1: Définir le Problème

1. Clarifier le besoin exact d'optimisation
2. Identifier les contraintes:
   - Mémoire disponible
   - CPU/latence acceptable
   - Throughput requis
3. Définir les métriques de succès mesurables

## Phase 2: Recherche Internet

1. Rechercher avec MCP Brave:
   - "[problème] algorithm 2024 2025 rust"
   - "[problème] state of the art implementation"

2. Consulter:
   - GitHub trending repos dans le domaine
   - Blog posts techniques (Rust, HNSW, vector DB)
   - Benchmarks comparatifs existants

3. Documenter les options trouvées avec liens

## Phase 3: Recherche arXiv

1. Rechercher sur arXiv (via web search):
   - Mots-clés: algorithme + domaine
   - Exemples: "HNSW optimization", "approximate nearest neighbor"

2. Filtrer les papers:
   - Récents (< 2 ans de préférence)
   - Avec implémentations disponibles
   - Avec benchmarks reproductibles

3. Identifier les innovations applicables à VelesDB

## Phase 4: Synthèse

1. Créer document de recherche:
   - Chemin: .research/YYYY-MM-DD-sujet.md
   - Contenu:
     - Problème posé
     - Solutions évaluées (min 3)
     - Avantages/inconvénients de chaque
     - Recommandation finale
     - Liens sources

## Phase 5: Décision

1. Présenter synthèse à l'utilisateur
2. Discuter trade-offs
3. Obtenir validation avant implémentation
4. Si nouvelle implémentation: créer US dans EPIC appropriée

## Template .research/

# Recherche: [Sujet]

Date: YYYY-MM-DD
Auteur: [Dev]

## Problème
[Description du besoin d'optimisation]

## Contraintes
- Mémoire: 
- Latence: 
- Throughput: 

## Solutions Évaluées

### Option 1: [Nom]
- Source: [lien]
- Avantages: 
- Inconvénients: 
- Complexité implémentation: 

### Option 2: [Nom]
...

## Recommandation
[Solution choisie et justification]

## Plan d'Action
1. ...
2. ...
