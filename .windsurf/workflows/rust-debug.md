---
description: Debugger et corriger un bug Rust avec méthodologie systématique
---

# Workflow : Debug Rust

## 1. Reproduction

1. Comprendre le bug rapporté
2. Écrire un test qui reproduit le bug :
   ```rust
   #[test]
   fn test_reproduces_bug_123() {
       // Ce test DOIT échouer avant le fix
   }
   ```

3. Lancer le test pour confirmer l'échec :
// turbo
```powershell
cargo test test_reproduces_bug
```

## 2. Investigation

1. Ajouter des logs de debug si nécessaire :
   ```rust
   tracing::debug!("Variable state: {:?}", variable);
   ```

2. Utiliser `dbg!()` macro pour debug rapide :
   ```rust
   let result = dbg!(some_computation());
   ```

3. Chercher le code concerné avec `code_search` ou `grep_search`

## 3. Analyse root cause

1. Identifier la vraie cause (pas juste le symptôme)
2. Vérifier si le bug existe ailleurs (patterns similaires)
3. Documenter la cause dans un commentaire

## 4. Fix minimal

1. Appliquer le fix le plus simple possible
2. NE PAS refactorer en même temps que le fix
3. Le test de reproduction doit maintenant passer :
// turbo
```powershell
cargo test test_reproduces_bug
```

## 5. Vérification

1. S'assurer qu'aucune régression n'est introduite :
// turbo
```powershell
cargo test --all-features
```

2. Vérifier clippy :
// turbo
```powershell
cargo clippy --all-targets --all-features -- -D warnings
```

## 6. Documentation

1. Ajouter un commentaire expliquant le fix si non évident :
   ```rust
   // Fix: Le calcul de distance doit normaliser les vecteurs
   // avant comparaison pour éviter les erreurs de précision.
   // Voir issue #123
   ```

2. Mettre à jour la doc si le comportement change

## 7. Commit

Message de commit :
```
fix: correct distance calculation for normalized vectors

- Add normalization step before cosine computation
- Fixes #123
```
