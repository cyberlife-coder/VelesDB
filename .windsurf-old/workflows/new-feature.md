---
name: new-feature
description: Transforme une description de feature en EPIC structurée avec US
---

# /new-feature "description de la fonctionnalité"

Workflow pour créer une EPIC complète depuis une idée de feature.

## Étape 1: Réception

Récupérer la description de feature fournie par l'utilisateur.

## Étape 2: Analyse

Invoquer @create-epic pour:
1. Analyser la demande
2. Identifier les sous-fonctionnalités
3. Structurer en EPIC + US

## Étape 3: Création

Le skill create-epic va:
1. Créer le dossier .epics/EPIC-XXX-nom/
2. Créer EPIC.md avec la description
3. Créer les US-YYY.md pour chaque sous-fonctionnalité
4. Créer progress.md pour le suivi

## Étape 4: Résumé

Afficher:
- EPIC créée avec son ID
- Liste des US générées avec complexités
- Prochaines étapes suggérées

## Étape 5: Validation

Demander à l'utilisateur:
- Validation de la structure
- Ajustements éventuels
- Priorisation des US

## Étape 6: Rappel Écosystème (OBLIGATOIRE)

**Toujours rappeler à l'utilisateur:**

> ⚠️ **RAPPEL ÉCOSYSTÈME**: Cette feature Core devra être propagée dans tous les SDKs une fois implémentée.
> 
> Après implémentation, exécuter `/ecosystem-sync EPIC-XXX` pour:
> - Créer les US de propagation (Python, WASM, TypeScript, Mobile, CLI, etc.)
> - Mettre à jour la matrice de parité EPIC-016

**Composants à considérer:**
- velesdb-server (API HTTP)
- velesdb-python (PyO3)
- velesdb-wasm (wasm-bindgen)
- velesdb-mobile (UniFFI)
- sdks/typescript (HTTP client)
- tauri-plugin-velesdb
- integrations/langchain
- integrations/llamaindex
- velesdb-cli
