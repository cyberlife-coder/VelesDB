---
name: complete-us
description: Marque une User Story comme terminée et met à jour le suivi
---

# /complete-us EPIC-XXX/US-YYY

Finalise proprement une User Story après validation complète de la Definition of Done.

## Étape 1: Lire la US

Lire le fichier `.epics/EPIC-XXX/US-YYY.md` pour récupérer:
- Les critères d'acceptation (AC-X)
- La section "Definition of Done (DoD)"
- Les tests requis
- Les fichiers impactés

## Étape 2: Validation DoD (OBLIGATOIRE)

**Exécuter les commandes de validation:**

```powershell
# 1. Validation CI locale complète
.\scripts\local-ci.ps1

# 2. Tests spécifiques à l'US
cargo test --package velesdb-core {module_name}

# 3. ThreadSanitizer (si tests concurrency existent)
# turbo
RUSTFLAGS="-Z sanitizer=thread" cargo +nightly test {module}_concurrency

# 4. Clippy strict
# turbo
cargo clippy -- -D warnings

# 5. Security check
# turbo
cargo deny check
```

**Vérifier chaque item de la checklist DoD dans la US:**

### Code & Implémentation
- [ ] Code implémenté dans les fichiers listés
- [ ] Pas de `unwrap()` en production
- [ ] Documentation `///` sur fonctions publiques
- [ ] Fichier < 500 lignes

### Tests TDD
- [ ] Tous les tests listés dans "Tests Requis" sont implémentés
- [ ] Tests dans fichiers **séparés** (`tests/*.rs`)
- [ ] `cargo test` → **100% GREEN**
- [ ] Couverture > 80%

### Critères d'Acceptation
- [ ] Chaque AC-X de la US est validé

### Qualité
- [ ] `cargo fmt --all` → pas de changements
- [ ] `cargo clippy -- -D warnings` → 0 warnings
- [ ] `cargo deny check` → 0 vulnérabilités

### Review
- [ ] `/fou-furieux` passé
- [ ] `/pre-commit` passé
- [ ] PR créée vers `develop`
- [ ] Review approuvée
- [ ] CI GitHub Actions GREEN

## Étape 3: Confirmer avec l'utilisateur

Afficher un résumé des validations:
- ✅ Items passés
- ❌ Items échoués (si applicable)

Demander confirmation: "Tous les items DoD sont-ils validés? (oui/non)"

**Si NON**: Lister les items manquants et arrêter le workflow.

## Étape 4: Mise à jour Status

Modifier `.epics/EPIC-XXX/progress.md`:
- Status US: ✅ DONE
- Date completion: aujourd'hui
- Lien PR si disponible

## Étape 5: Mise à jour US

Modifier `.epics/EPIC-XXX/US-YYY.md`:
- Cocher tous les items de la DoD
- Ajouter entrée dans Historique: `| {date} | ✅ DONE | Validé via /complete-us |`
- Status en haut: 🟢 DONE

## Étape 6: Vérification Écosystème (OBLIGATOIRE pour Core)

**Si l'US est dans velesdb-core ET modifie une API publique:**

1. Vérifier si `ecosystem-sync.md` existe dans le dossier EPIC
2. Si NON: créer le fichier avec la checklist de propagation
3. Rappeler à l'utilisateur: "Cette feature doit être propagée dans l'écosystème. Exécuter `/ecosystem-sync EPIC-XXX`"

**Checklist à inclure:**
```markdown
| SDK | Status | Notes |
|-----|--------|-------|
| velesdb-server | 🔴 TODO | Endpoint API |
| velesdb-python | 🔴 TODO | PyO3 bindings |
| velesdb-wasm | 🔴 TODO | wasm-bindgen |
| velesdb-mobile | 🔴 TODO | UniFFI |
| sdks/typescript | 🔴 TODO | HTTP client |
| tauri-plugin | 🔴 TODO | Tauri commands |
| langchain | 🔴 TODO | Retriever |
| llamaindex | 🔴 TODO | VectorStore |
| velesdb-cli | 🔴 TODO | Commandes |
```

## Étape 7: Clôture EPIC automatique

Vérifier si toutes les US de l'EPIC sont DONE. Si oui, lancer automatiquement la clôture :

```powershell
# Vérification et clôture automatique
$status = cascade: /status EPIC-XXX
if ($status -match "US restantes: 0") {
    Write-Host "📦 Toutes les US sont DONE -> /complete-epic"
    cascade: /complete-epic EPIC-XXX
} else {
    Write-Host "⏳ US restantes à compléter"
}
```

## Étape 8: Résumé Final

Afficher:
- ✅ US complétée: EPIC-XXX/US-YYY
- 📊 Progression EPIC: X/Y US (XX%)
- 📝 Prochaine US suggérée
- 🔗 Lien PR (si disponible)
