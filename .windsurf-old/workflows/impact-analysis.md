---
description: Analyse d'impact avant modification de code (dépendances, SDKs, intégrations)
---

# /impact-analysis [fichier ou fonction]

Analyse systématique des impacts avant toute modification significative.

## Phase 1: Scope de la modification

1. Identifier le fichier/fonction cible
2. Classifier le type de changement:
   - [ ] API publique (signature, types)
   - [ ] Implémentation interne
   - [ ] Dépendance externe
   - [ ] Configuration

## Phase 2: Dépendances entrantes

### 2.1 Usages dans le crate

// turbo
```powershell
rg "NOM_FONCTION|NomStruct" --type rust -l
```

### 2.2 Usages cross-crate

// turbo
```powershell
rg "NOM_FONCTION|NomStruct" crates/ --type rust -l
```

### 2.3 Usages dans les SDKs

// turbo
```powershell
rg "NOM_FONCTION|NomStruct" sdks/ integrations/ --type-add "ts:*.ts" --type-add "py:*.py" -l
```

## Phase 3: Analyse des dépendances sortantes

### 3.1 Imports du fichier

// turbo
```powershell
rg "^use " FICHIER_CIBLE
```

### 3.2 Arbre de dépendances

// turbo
```powershell
cargo tree -p velesdb-core --prefix none | Select-Object -First 30
```

## Phase 4: Matrice d'impact

Générer une matrice:

| Composant | Fichier(s) impacté(s) | Type d'impact | Action requise |
|-----------|----------------------|---------------|----------------|
| (compléter) | | Direct/Indirect | |

## Phase 5: Plan d'action

### 5.1 Ordre des modifications

1. Core: `crates/velesdb-core/src/...`
2. Tests: `crates/velesdb-core/tests/...`
3. Server: `crates/velesdb-server/src/...`
4. SDKs: `sdks/*/src/...`
5. Intégrations: `integrations/*/src/...`

### 5.2 Tests à exécuter

// turbo
```powershell
cargo test --workspace --no-fail-fast 2>&1 | Select-Object -First 50
```

## Phase 6: Rapport

Générer un rapport markdown:

```markdown
## Impact Analysis Report

**Cible**: `path/to/file.rs::function_name`
**Type**: [API Change / Internal / Dependency]
**Date**: YYYY-MM-DD

### Composants impactés
- [ ] velesdb-core (fichiers: ...)
- [ ] velesdb-server (fichiers: ...)
- [ ] SDK Python (fichiers: ...)
- [ ] SDK TypeScript (fichiers: ...)

### Breaking changes
- [ ] Oui - Nécessite CHANGELOG
- [ ] Non

### Tests requis
- `cargo test -p velesdb-core`
- `cargo test -p velesdb-server`

### Risque estimé: [Faible/Moyen/Élevé]
```

## Après validation

Si l'analyse est validée, procéder avec `/implement-us` ou la modification directe.
