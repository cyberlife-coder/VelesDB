---
name: bugfix
description: Démarre un cycle de correction de bug avec boucle Kaizen (distinct de feature)
---

# /bugfix "description du bug"

Cycle de correction de bug avec **boucle Kaizen d'amélioration continue** (max 25 cycles).

## Principe Kaizen

```
FIX → TEST → IMPACT → SMELLS → NEW BUGS? → FIX...
```

Chaque fix déclenche une ré-analyse jusqu'à stabilisation complète.

---

## Étape 1: Synchronisation

// turbo
```powershell
git checkout develop
git pull origin develop
```

## Étape 2: Création Branche

// turbo
```powershell
git checkout -b bugfix/XXX-description-courte
```

## Étape 3: Reproduction (RED)

1. Écrire un test qui reproduit le bug:
   ```rust
   #[test]
   fn test_reproduces_bug_xxx() {
       // Ce test DOIT échouer avant le fix
   }
   ```

2. Confirmer que le test échoue:
   ```powershell
   cargo test test_reproduces_bug
   ```

## Étape 4: Investigation

1. Identifier la **root cause** (pas juste le symptôme)
2. Vérifier si le bug existe ailleurs (patterns similaires)
3. Documenter la cause dans le commit

---

## Étape 4.1: Vision Produit (si patterns détectés)

Si l'investigation révèle un pattern problématique récurrent:

### Questions Long Terme

1. **Ce bug révèle-t-il une faiblesse architecturale?**
   - Si oui → Créer une issue ou EPIC pour refactoring futur
   - Documenter avec `// TODO(arch):` dans le code

2. **Le fix actuel est-il la bonne solution long terme?**
   - Fix minimal maintenant OK si non-bloquant
   - Si bloquant pour roadmap → fix complet maintenant

3. **Impact sur l'écosystème?**
   - Le bug affecte-t-il d'autres composants (SDKs, bindings)?
   - Propager le fix si nécessaire

### Matrice de Décision

| Situation | Action |
|-----------|--------|
| Bug isolé, pas de pattern | Fix minimal |
| Pattern récurrent détecté | Fix + créer issue refactoring |
| Bloque feature roadmap | Fix complet + refactor |
| Affecte écosystème | Fix + propager aux SDKs |

---

## Étape 5: Boucle Kaizen (max 25 cycles)

### 5.1 Fix Minimal (GREEN)
1. Appliquer le fix **le plus simple** possible
2. NE PAS refactorer en même temps
3. Vérifier que le test passe

### 5.2 Test
// turbo
```powershell
cargo test --workspace
```

### 5.3 Impact Analysis
```powershell
# Fichiers impactés par le fix
git diff --name-only HEAD~1

# Qui appelle la fonction modifiée?
grep -rn "function_name(" --include="*.rs"
```

Vérifier:
- [ ] Le fix impacte-t-il d'autres modules?
- [ ] Types/signatures modifiés?
- [ ] Appelants indirects affectés?

### 5.4 Code Smells Check
// turbo
```powershell
cargo clippy --workspace --all-targets -- -D warnings
```

Vérifier manuellement:
- [ ] Fichiers < 500 lignes?
- [ ] Fonctions < 30 lignes?
- [ ] Pas de duplication introduite?
- [ ] Nommage clair?

### 5.5 New Bugs Detection
Rechercher problèmes introduits par le fix:
- [ ] `unwrap()` ajoutés sans justification?
- [ ] `clone()` inutiles?
- [ ] Edge cases non gérés?
- [ ] Logique inversée ou incomplète?
- [ ] Pattern similaire ailleurs non corrigé?

### 5.6 🦀 Rust-Specific AI Check

**Le fix généré par IA respecte-t-il les règles Rust?**

#### Ownership & Borrowing
- [ ] Pas de "use after move" introduit
- [ ] Emprunts `&mut` correctement scopés
- [ ] Pas de dangling references

#### Type Safety
- [ ] Conversions numériques avec `try_from()` (pas `as`)
- [ ] Match exhaustif (pas de `_` catch-all aveugle)
- [ ] Lifetimes explicites si retour de référence

#### Error Handling
- [ ] `?` pour propagation (pas de nouveau `unwrap()`)
- [ ] Erreurs typées (pas de `String` comme erreur)

#### Thread Safety
- [ ] Si données partagées: `Arc`/`Mutex` appropriés
- [ ] Tests GPU avec `#[serial(gpu)]`

**Commande de validation:**
```powershell
cargo clippy -- -D warnings -D clippy::unwrap_used
```

**Référence:** `/rust-ai-checklist` pour détails

### 5.7 Decision Point

| Résultat | Action |
|----------|--------|
| Tout OK | → Étape 6 (sortie) |
| Nouveau problème | → Retour 5.1 (cycle++) |
| cycle >= 25 | → STOP + review humaine |

---

## Étape 6: Validation Finale

// turbo
```powershell
cargo fmt --all
cargo clippy -- -D warnings
cargo test --workspace
cargo deny check
```

## Étape 7: Commit

Format: `fix(scope): description`

```
fix(scope): description courte

Root cause: [cause identifiée]
Fix: [solution appliquée]
Test: [nom du test de régression]
Kaizen cycles: X
```

## Étape 8: PR

Exécuter `/pr-create` vers develop.

---

## Résumé Kaizen

À la fin, afficher:

| Métrique | Valeur |
|----------|--------|
| Cycles Kaizen | X |
| Tests ajoutés | X |
| Fichiers modifiés | X |
| Patterns similaires corrigés | X |
