---
trigger: always_on
description: Template obligatoire pour les blocs unsafe et les justifications de code
---

# Safety Comments & Code Justifications

## 1. Blocs `unsafe` - Template SAFETY Obligatoire

Tout bloc `unsafe` DOIT avoir un commentaire `// SAFETY:` explicant pourquoi le code est sûr.

### Format Standard

```rust
// SAFETY: [Invariant principal maintenu]
// - [Condition 1]: [Explication pourquoi c'est garanti]
// - [Condition 2]: [Explication pourquoi c'est garanti]
// Reason: [Pourquoi unsafe est nécessaire ici]
unsafe {
    // Code unsafe
}
```

### Exemple Concret

```rust
// SAFETY: Pointer arithmetic within bounds
// - `ptr` comes from `Vec::as_ptr()` and remains valid for the lifetime of `self`
// - `offset` is checked to be < len before this call
// - Memory is properly aligned for f32 (Vec guarantees this)
// Reason: SIMD intrinsics require raw pointer access for performance
unsafe {
    _mm256_loadu_ps(ptr.add(offset))
}
```

### Checklist avant d'écrire `unsafe`

- [ ] Y a-t-il une alternative safe ? (souvent oui!)
- [ ] Tous les invariants sont-ils documentés ?
- [ ] Le scope unsafe est-il minimal ?
- [ ] Les tests couvrent-ils ce code ?

---

## 2. Casts Numériques - Justification Obligatoire

Les casts avec `as` qui peuvent tronquer ou perdre de la précision doivent être justifiés.

### Pattern Recommandé

```rust
// Préférer try_from avec gestion d'erreur explicite
let id = u32::try_from(index).map_err(|_| Error::IndexOverflow)?;

// Si cast direct nécessaire, ajouter allow + Reason
#[allow(clippy::cast_possible_truncation)]
// Reason: selectivity is always in [0.0, 1.0], clamped before cast
let percent = (selectivity.clamp(0.0, 1.0) * 100.0) as u32;
```

### Anti-Patterns à Éviter

```rust
// ❌ Cast silencieux sans justification
let id = index as u32;

// ❌ Allow sans Reason
#[allow(clippy::cast_possible_truncation)]
let id = index as u32;
```

---

## 3. `clone()` dans Hot Paths - Justification Obligatoire

Les `clone()` dans les chemins critiques doivent être justifiés.

### Format

```rust
// Reason: Filter::apply takes ownership, cannot borrow across async boundary
let filter_clone = filter.clone();
```

### Questions à se poser

1. Peut-on utiliser une référence `&T` au lieu de `T` ?
2. Peut-on utiliser `Cow<T>` ou `Arc<T>` ?
3. Le clone est-il vraiment dans un hot path ?

---

## 4. `unwrap()` / `expect()` - Règles

### En production (hors tests)

```rust
// ❌ INTERDIT - unwrap silencieux
let value = map.get("key").unwrap();

// ✅ OK - expect avec message descriptif
let value = map.get("key").expect("key must exist: initialized in new()");

// ✅ PRÉFÉRÉ - propagation d'erreur
let value = map.get("key").ok_or(Error::MissingKey)?;
```

### En tests

```rust
// ✅ OK - unwrap autorisé en tests
#[test]
fn test_something() {
    let result = function().unwrap();
    assert_eq!(result, expected);
}
```

---

## 5. Validation Automatique

Ces règles sont vérifiées par :

1. **Clippy CI** (`.clippy.toml`) - Lints automatiques
2. **Pre-commit hook** - Validation avant commit
3. **Code review** - Vérification humaine des justifications

### Commande de vérification locale

```powershell
cargo clippy -- -D warnings -W clippy::unwrap_used
```
