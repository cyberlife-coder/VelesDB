---
name: expert-review
description: Review complète des flags de code (style Devin) avant merge vers main
---

# /expert-review

Review systématique des flags potentiels avant merge d'une EPIC.

## Quand l'utiliser

- Avant merge d'une EPIC complète vers main
- Après refactoring majeur
- Avant release

## Étape 1: Scan des Anti-Patterns

Rechercher dans les fichiers modifiés:

```powershell
# unwrap() en production (hors tests)
rg "\.unwrap\(\)" --glob "!*test*" --glob "!tests/" -l

# Truncating casts
rg "as u32|as u16|as u8" --glob "*.rs" -l

# Arithmetic overflow potentiel
rg "\.sub\(|\.add\(" --glob "*.rs" -l
```

## Étape 2: Catégorisation des Flags

Pour chaque flag identifié, catégoriser:

| Catégorie | Action |
|-----------|--------|
| **BUG** | Corriger immédiatement |
| **DESIGN** | Documenter avec `// Note:` ou `// FLAG-X:` |
| **OK** | Vérifier et valider |
| **SDK** | Hors scope core |

## Étape 3: Checklist SecDev

- [ ] Pas de `unwrap()` sur données utilisateur
- [ ] Pas de `as uX` sans bounds check (utiliser `try_from`)
- [ ] Pas de `partial_cmp().unwrap()` (utiliser `total_cmp()`)
- [ ] Ressources partagées: `#[serial]` si tests parallèles
- [ ] Fichiers < 500 lignes
- [ ] Tests dans fichiers SÉPARÉS

## Étape 4: Validation Patterns

Vérifier les patterns critiques:

### GPU Tests
```rust
#[test]
#[serial(gpu)]  // OBLIGATOIRE
fn test_gpu_xxx() { ... }
```

### Option handling
```rust
// ❌ Éviter
.unwrap_or(0)  // Si 0 peut être valide

// ✅ Préférer
.filter_map(|x| x?)
```

### Float comparison
```rust
// ❌ Panic sur NaN
a.partial_cmp(&b).unwrap()

// ✅ Safe
a.total_cmp(&b)
```

## Étape 5: Rapport

Générer rapport avec:
- Nombre de flags par catégorie
- Flags corrigés
- Flags documentés (design decisions)
- Recommandations

## Étape 6: Validation Finale

Si tous les flags sont traités:
```powershell
/pre-commit
/local-ci
```

Puis `/pr-create` vers develop.
