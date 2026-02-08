---
name: fou-furieux
description: Lance le cycle de contrôle qualité intensif post-implémentation
---

# /fou-furieux [type?]

Cycle de contrôle qualité exhaustif.

## Arguments Optionnels

- debug: uniquement phase debug
- smell: uniquement code smells
- security: uniquement sécurité
- perf: uniquement performance
- thread: uniquement multithreading
- (vide): cycle COMPLET

## Exécution

Invoquer @fou-furieux avec le type spécifié.

Le skill va exécuter les contrôles en boucle:
1. Debug
2. Code Smells  
3. Sécurité
4. Performance
5. Multithreading
6. **🦀 Rust-AI Compliance** (NOUVEAU)

## 🦀 Phase Rust-AI Compliance

**Vérifications spécifiques au code généré par IA:**

### Ownership & Borrowing
```powershell
# Rechercher patterns problématiques
Select-String -Path "**/*.rs" -Pattern "\.clone\(\)" | Measure-Object
# Chaque clone() doit avoir un commentaire justificatif
```

### Error Handling
```powershell
# Compter les unwrap() non justifiés
Select-String -Path "**/*.rs" -Pattern "\.unwrap\(\)" -Exclude "*test*"
```

### Type Conversions
```powershell
# Détecter les "as u32" dangereux
Select-String -Path "**/*.rs" -Pattern " as u32| as u64| as usize"
# Doivent utiliser try_from() ou avoir // SAFETY: comment
```

### Checklist Rust-AI
- [ ] Tous les `clone()` ont un `// Clone needed:` commentaire
- [ ] Aucun `unwrap()` en code de production (sauf avec `// SAFETY:`)
- [ ] Conversions numériques avec `try_from()` ou commentaire `// SAFETY:`
- [ ] Lifetimes explicites sur fonctions retournant `&T`
- [ ] Tests GPU marqués `#[serial(gpu)]`
- [ ] `Arc::clone(&x)` au lieu de `x.clone()` pour Arc

**Référence complète:** `/rust-ai-checklist`

## Boucle

Si un contrôle échoue:
1. Afficher les problèmes détectés
2. Proposer corrections
3. Après correction: retour au contrôle 1

## Succès

Quand TOUS les contrôles passent:
1. Afficher résumé complet
2. Proposer /pre-commit
