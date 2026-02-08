---
description: Corriger automatiquement les tests qui échouent avant de mettre à jour les métriques
---

# /fix-failed-tests - Correction automatique des tests échoués

Ce workflow est déclenché automatiquement par `/release-metrics` lorsque des tests échouent.
Il analyse, corrige et valide chaque test jusqu'à 100% passing.

---

## 🔄 Boucle de correction (max 10 itérations)

```
┌─────────────────────────────────────────────────────────────────┐
│                    BOUCLE DE CORRECTION                         │
│                                                                 │
│  1. Parser test_results.txt → Liste des tests FAILED            │
│                         ↓                                       │
│  2. Pour CHAQUE test échoué:                                    │
│     a. Lire le message d'erreur                                 │
│     b. Localiser le fichier source                              │
│     c. Analyser la cause (assertion, panic, timeout)            │
│     d. Appliquer la correction                                  │
│                         ↓                                       │
│  3. Re-exécuter cargo test --workspace                          │
│                         ↓                                       │
│  4. Si FAILED > 0 → Recommencer (max 10 itérations)             │
│     Si PASSED 100% → Retourner à /release-metrics               │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

---

## Étape 1: Parser les tests échoués

```powershell
# Extraire les noms des tests FAILED
$failedTests = Select-String -Path test_results.txt -Pattern "^test .* FAILED$" | ForEach-Object { $_.Line }
Write-Host "Tests échoués: $($failedTests.Count)"
$failedTests | ForEach-Object { Write-Host "  - $_" }
```

**Format attendu**:
```
test module::submodule::test_name ... FAILED
```

---

## Étape 2: Analyser chaque test échoué

Pour CHAQUE test dans `$failedTests`:

### 2.1 Localiser le fichier source

```powershell
# Exemple: test velesql::parser::tests::test_parse_select
# → Fichier: crates/velesdb-core/src/velesql/parser.rs ou parser/tests.rs
```

**Règles de localisation**:
| Pattern | Fichier |
|---------|---------|
| `module::tests::test_xxx` | `src/module.rs` ou `src/module/mod.rs` |
| `module::submodule::tests::test_xxx` | `src/module/submodule.rs` |
| `tests::test_xxx` (integration) | `tests/*.rs` |

### 2.2 Lire le message d'erreur complet

Chercher dans `test_results.txt` le bloc entre:
```
---- module::test_name stdout ----
[message d'erreur]
```

### 2.3 Classifier le type d'échec

| Type | Pattern | Action |
|------|---------|--------|
| **Assertion failed** | `assertion failed` | Mettre à jour la valeur attendue |
| **Panic** | `panicked at` | Corriger le code ou ajouter handling |
| **Timeout** | `test timed out` | Optimiser ou augmenter timeout |
| **Compile error** | `error[E` | Corriger l'erreur de compilation |
| **Expected vs Got** | `left: X, right: Y` | Ajuster assertion ou code |

---

## Étape 3: Appliquer les corrections

### 3.1 Cas: Assertion avec nouvelle valeur attendue

Si le test vérifie une métrique qui a changé légitimement:

```rust
// AVANT
assert_eq!(result.len(), 10);

// APRÈS (si la nouvelle valeur est correcte)
assert_eq!(result.len(), 12);
```

**⚠️ IMPORTANT**: Ne modifier l'assertion que si la NOUVELLE valeur est correcte.
Si le code est cassé, corriger le CODE, pas le test.

### 3.2 Cas: Code cassé

Si le test révèle un vrai bug:

```
→ Lancer /debug-taskforce pour investiguer et corriger
```

### 3.3 Cas: Test obsolète

Si le test teste une fonctionnalité supprimée ou modifiée:

```rust
// Option 1: Supprimer le test
#[test]
#[ignore = "Feature removed in v1.5.0"]
fn test_old_feature() { ... }

// Option 2: Adapter le test à la nouvelle API
```

---

## Étape 4: Re-exécuter les tests

```powershell
# Re-exécuter uniquement les tests qui ont échoué (plus rapide)
cargo test --workspace --release -- $failedTestNames 2>&1 | Tee-Object -FilePath test_rerun.txt

# Vérifier le résultat
$stillFailed = (Select-String -Path test_rerun.txt -Pattern "FAILED").Count
if ($stillFailed -eq 0) {
    Write-Host "✅ Tous les tests corrigés!"
} else {
    Write-Host "⚠️ Encore $stillFailed tests échoués. Itération suivante..."
}
```

---

## Étape 5: Validation finale

```powershell
# Exécuter TOUS les tests pour confirmer
cargo test --workspace --release 2>&1 | Tee-Object -FilePath test_final.txt

$totalPassed = (Select-String -Path test_final.txt -Pattern "test result: ok").Count
$totalFailed = (Select-String -Path test_final.txt -Pattern "FAILED").Count

if ($totalFailed -eq 0) {
    Write-Host "✅ 100% tests passing - Retour à /release-metrics"
} else {
    Write-Host "❌ Échec après 10 itérations - Investigation manuelle requise"
    Write-Host "Lancer: /debug-taskforce"
}
```

---

## 🔀 Décision: Correction vs Investigation

```
┌─────────────────────────────────────────────────────────────────┐
│                    ARBRE DE DÉCISION                            │
│                                                                 │
│  Test échoué                                                    │
│       │                                                         │
│       ├── Assertion avec nouvelle valeur ?                      │
│       │   └── OUI → Mettre à jour l'assertion                   │
│       │                                                         │
│       ├── Bug dans le code ?                                    │
│       │   └── OUI → /debug-taskforce                            │
│       │                                                         │
│       ├── Test obsolète ?                                       │
│       │   └── OUI → #[ignore] avec raison                       │
│       │                                                         │
│       └── Incompréhensible ?                                    │
│           └── OUI → /debug-taskforce                            │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

---

## 📋 Checklist par test corrigé

- [ ] Cause racine identifiée
- [ ] Correction appliquée (code OU assertion, pas les deux)
- [ ] Test passe en isolation (`cargo test test_name`)
- [ ] Pas de régression sur autres tests
- [ ] Commentaire si changement non-trivial

---

## ⚠️ Règles strictes

1. **Ne JAMAIS supprimer un test** sans raison documentée
2. **Ne JAMAIS modifier une assertion** si le code est cassé
3. **Maximum 10 itérations** avant escalade manuelle
4. **Chaque correction = 1 commit** pour traçabilité
5. **Si doute → /debug-taskforce** plutôt que deviner

---

## 🔗 Workflows liés

| Situation | Workflow |
|-----------|----------|
| Bug complexe | `/debug-taskforce` |
| Refactoring nécessaire | `/refactor-module` |
| Retour aux métriques | `/release-metrics` |
| Commit des corrections | `/pre-commit` |

---

## Exemple complet

```
1. cargo test → 3 FAILED
2. Parser: test_parse_select, test_hnsw_recall, test_simd_dot

3. test_parse_select:
   - Erreur: assertion failed: expected 5, got 6
   - Cause: Nouveau token ajouté dans parser
   - Action: assert_eq!(tokens.len(), 6)

4. test_hnsw_recall:
   - Erreur: recall 94.5% < 95% threshold
   - Cause: Changement d'algo HNSW
   - Action: → /debug-taskforce (bug potentiel)

5. test_simd_dot:
   - Erreur: test timed out after 60s
   - Cause: Boucle infinie introduite
   - Action: → /debug-taskforce

6. Après corrections: cargo test → 100% PASSED
7. Retour à /release-metrics
```

