---
name: impact-analysis
description: Analyse complète des impacts et dépendances avant modification de code
---

# Impact Analysis Skill

Analyse systématique des impacts potentiels et dépendances lors de modifications de code.

## Quand utiliser

- Avant de modifier une fonction publique
- Avant de changer une signature de trait
- Avant de modifier un module partagé
- Avant d'ajouter/supprimer une dépendance
- Avant de refactorer un fichier > 200 lignes

## Phase 1: Identification du scope

### 1.1 Fichier cible
Identifier le fichier et la fonction/struct à modifier.

### 1.2 Type de modification
- [ ] **API Change**: Signature publique modifiée
- [ ] **Internal Change**: Implémentation interne uniquement
- [ ] **Dependency Change**: Ajout/suppression de crate
- [ ] **Refactoring**: Restructuration sans changement fonctionnel

## Phase 2: Analyse des dépendances

### 2.1 Dépendances entrantes (qui utilise ce code?)

```powershell
# Trouver tous les usages de la fonction/struct
rg "nom_fonction|NomStruct" --type rust -l
```

### 2.2 Dépendances sortantes (de quoi dépend ce code?)

```powershell
# Analyser les imports du fichier
rg "^use " fichier.rs
```

### 2.3 Matrice d'impact

| Composant | Type de dépendance | Impact si modifié |
|-----------|-------------------|-------------------|
| (à remplir) | Direct/Indirect | Haut/Moyen/Faible |

## Phase 3: Analyse cross-crate

### 3.1 Vérifier les crates impactés

```powershell
# Dans un workspace multi-crates
cargo tree --invert -p <crate-name>
```

### 3.2 Mapping des impacts

| Crate | Fichiers impactés | Tests à exécuter |
|-------|-------------------|------------------|
| velesdb-core | (liste) | `cargo test -p velesdb-core` |
| velesdb-server | (liste) | `cargo test -p velesdb-server` |

## Phase 4: Analyse des SDKs et intégrations

### 4.1 SDKs impactés

| SDK | Binding concerné | Action requise |
|-----|------------------|----------------|
| Python (PyO3) | `src/python/*.rs` | Vérifier #[pyfunction] |
| WASM | `velesdb-wasm/src/*.rs` | Vérifier #[wasm_bindgen] |
| TypeScript | `sdks/typescript/src/*.ts` | Mettre à jour types |

### 4.2 Intégrations tierces

| Intégration | Impact potentiel |
|-------------|------------------|
| LangChain | Vérifier VectorStore API |
| LlamaIndex | Vérifier NodeParser API |

## Phase 5: Plan d'action

### 5.1 Ordre des modifications

1. **Core d'abord**: Modifier le module central
2. **Tests ensuite**: Mettre à jour les tests
3. **Bindings après**: Propager aux SDKs
4. **Documentation enfin**: Mettre à jour README/CHANGELOG

### 5.2 Checklist de validation

- [ ] Tous les usages identifiés
- [ ] Tests de régression écrits
- [ ] Breaking changes documentés
- [ ] SDKs mis à jour si nécessaire
- [ ] CHANGELOG mis à jour

## Phase 6: Rapport d'impact

### Template de rapport

```markdown
## Impact Analysis Report

**Fichier modifié**: `path/to/file.rs`
**Modification**: [description]
**Date**: YYYY-MM-DD

### Dépendances impactées
- Crate X (fichiers: a.rs, b.rs)
- SDK Python (fonction: xyz)

### Tests requis
- `cargo test -p crate1`
- `cargo test -p crate2`

### Breaking changes
- [ ] Oui / [x] Non

### Risque estimé
- [x] Faible / [ ] Moyen / [ ] Élevé
```

## Commandes utiles

```powershell
# Graphe de dépendances du crate
cargo depgraph --dedup-transitive-deps | dot -Tpng > deps.png

# Fichiers modifiés récemment
git diff --name-only HEAD~5

# Usages d'un symbole
rg "NomSymbole" --type rust -C 2

# Vérifier les exports publics
rg "^pub " src/lib.rs
```
