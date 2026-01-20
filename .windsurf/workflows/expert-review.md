---
description: Lance une revue multi-experts sur une EPIC ou des changements majeurs
---

# Review Multi-Experts

## Étape 1: Inventaire des changements

1. Identifier l'EPIC concernée
2. Lister les fichiers modifiés: `git diff --name-only develop`
3. Lire le progress.md de l'EPIC

## Étape 2: Review Architecture (🔧 Architecte)

1. Vérifier taille des fichiers modifiés (< 500 lignes)
2. Vérifier modularité et séparation des responsabilités
3. Vérifier patterns SOLID et DRY
4. Évaluer: ✅ APPROUVÉ / ⚠️ À AMÉLIORER / ❌ REJETÉ

## Étape 3: Review Sécurité (🛡️ SecDev)

// turbo
1. `cargo deny check`
2. Rechercher `unsafe` sans commentaire `// SAFETY:`
3. Rechercher `unwrap()` en code de production
4. Vérifier validation des entrées utilisateur
5. Évaluer: ✅ APPROUVÉ / ⚠️ À AMÉLIORER / ❌ REJETÉ

## Étape 4: Review Tests (🧪 QA)

// turbo
1. `cargo test --workspace`
2. Compter les tests ajoutés/modifiés
3. Vérifier couverture des edge cases
4. Évaluer: ✅ APPROUVÉ / ⚠️ À AMÉLIORER / ❌ REJETÉ

## Étape 5: Review Performance (⚡ Perf)

1. Identifier les benchmarks pertinents
2. Exécuter benchmarks: `cargo bench --bench <name>`
3. Comparer avec objectifs de latence
4. Évaluer: ✅ APPROUVÉ / ⚠️ À AMÉLIORER / ❌ REJETÉ

## Étape 6: Synthèse

Produire tableau récapitulatif:

| Expert | Verdict | Notes |
|--------|---------|-------|
| 🔧 Architecte | ... | ... |
| 🛡️ SecDev | ... | ... |
| 🧪 QA | ... | ... |
| ⚡ Perf | ... | ... |

**Verdict Final**: PRÊT POUR MERGE / À CORRIGER
