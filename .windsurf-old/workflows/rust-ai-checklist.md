---
description: Checklist des pièges Rust pour la génération de code IA - À vérifier après chaque implémentation
---

# 🦀 Rust AI Generation Checklist

> **Pourquoi ce document?** Rust est particulièrement difficile pour la génération IA en raison de son système de propriété, du borrow checker et de son système de types strict. Ce checklist capture les erreurs les plus fréquentes.

---

## 🔴 Erreurs Critiques à Vérifier IMMÉDIATEMENT

### 1. Ownership & Move Semantics

```rust
// ❌ ERREUR FRÉQUENTE: Use after move
let data = vec![1, 2, 3];
process(data);        // data est "moved"
println!("{:?}", data); // ERREUR: value borrowed after move

// ✅ CORRECT: Clone si nécessaire ou passer référence
let data = vec![1, 2, 3];
process(data.clone()); // ou process(&data)
println!("{:?}", data);
```

**Checklist:**
- [ ] Pas d'utilisation de variable après un move
- [ ] `clone()` justifié (commentaire `// Clone needed: ...`)
- [ ] Préférer `&T` ou `&mut T` au lieu de `T` quand possible

### 2. Borrow Checker - Références Mutables/Immutables

```rust
// ❌ ERREUR FRÉQUENTE: Multiple mutable borrows
let mut vec = vec![1, 2, 3];
let first = &mut vec[0];
let second = &mut vec[1]; // ERREUR: cannot borrow `vec` as mutable more than once
*first = 10;

// ✅ CORRECT: Scope les emprunts ou utilise split_at_mut
let mut vec = vec![1, 2, 3];
{
    let first = &mut vec[0];
    *first = 10;
}
let second = &mut vec[1];
```

**Checklist:**
- [ ] Pas de `&mut` simultanés sur la même donnée
- [ ] Pas de `&` et `&mut` simultanés
- [ ] Emprunts scopés au minimum nécessaire

### 3. Lifetimes Explicites

```rust
// ❌ ERREUR FRÉQUENTE: Lifetime manquant
fn get_first(items: &[String]) -> &str {
    &items[0] // Lifetime implicite OK ici
}

// ❌ ERREUR: Lifetime ambigu
fn longest(x: &str, y: &str) -> &str { // ERREUR: missing lifetime specifier
    if x.len() > y.len() { x } else { y }
}

// ✅ CORRECT: Lifetime explicite
fn longest<'a>(x: &'a str, y: &'a str) -> &'a str {
    if x.len() > y.len() { x } else { y }
}
```

**Checklist:**
- [ ] Fonctions retournant des références ont des lifetimes explicites
- [ ] Structs contenant des références ont des lifetimes
- [ ] Pas de dangling references

---

## 🟡 Patterns Problématiques Courants

### 4. Unwrap/Expect sans Justification

```rust
// ❌ DANGEREUX: unwrap() sans contexte
let value = some_option.unwrap();

// ✅ MIEUX: expect() avec message
let value = some_option.expect("Config file should have 'port' field");

// ✅ IDÉAL: Propagation d'erreur
let value = some_option.ok_or(ConfigError::MissingPort)?;
```

**Checklist:**
- [ ] Aucun `unwrap()` en code de production
- [ ] `expect()` avec message descriptif si justifié
- [ ] Préférer `?` pour propager les erreurs

### 5. Clone() Excessif

```rust
// ❌ ANTI-PATTERN: Clone pour contourner borrow checker
fn process(data: Vec<String>) {
    for item in data.clone() { // Clone de 1000 strings...
        // ...
    }
}

// ✅ CORRECT: Utiliser références
fn process(data: &[String]) {
    for item in data {
        // ...
    }
}
```

**Checklist:**
- [ ] Chaque `clone()` a un commentaire justificatif
- [ ] Pas de `clone()` dans des boucles hot path
- [ ] Considérer `Cow<'_, T>` ou `Rc/Arc` selon le cas

### 6. Conversion de Types Numéiques

```rust
// ❌ DANGEREUX: Cast silencieux avec troncation
let len: usize = large_number;
let id: u32 = len as u32; // Troncation si len > u32::MAX

// ✅ CORRECT: try_from avec gestion d'erreur
let id = u32::try_from(len).map_err(|_| Error::IdOverflow)?;
```

**Checklist:**
- [ ] Pas de `as` pour conversions qui peuvent perdre des données
- [ ] Utiliser `try_from()` / `try_into()`
- [ ] Documenter les conversions assumées safe

---

## 🟢 Bonnes Pratiques Rust

### 7. Pattern Matching Exhaustif

```rust
// ❌ FRAGILE: catch-all qui cache des erreurs
match result {
    Ok(v) => process(v),
    _ => (), // Quels cas sont ignorés?
}

// ✅ EXPLICITE: Tous les cas nommés
match result {
    Ok(v) => process(v),
    Err(Error::NotFound) => log::debug!("Not found, skipping"),
    Err(e) => return Err(e),
}
```

### 8. Traits et Génériques

```rust
// ❌ ERREUR: Bounds manquants
fn print_all<T>(items: &[T]) {
    for item in items {
        println!("{}", item); // ERREUR: T doesn't implement Display
    }
}

// ✅ CORRECT: Bounds explicites
fn print_all<T: std::fmt::Display>(items: &[T]) {
    for item in items {
        println!("{}", item);
    }
}
```

### 9. Thread Safety (Send/Sync)

```rust
// ❌ ERREUR: Type non-thread-safe partagé
let data = Rc::new(vec![1, 2, 3]);
std::thread::spawn(|| {
    println!("{:?}", data); // ERREUR: Rc cannot be sent between threads
});

// ✅ CORRECT: Arc pour partage cross-thread
let data = Arc::new(vec![1, 2, 3]);
let data_clone = Arc::clone(&data);
std::thread::spawn(move || {
    println!("{:?}", data_clone);
});
```

---

## 📋 Checklist Rapide Post-Génération

Après chaque génération de code Rust par IA, vérifier:

```
□ cargo check     → Compile sans erreur
□ cargo clippy    → Pas de warnings
□ cargo test      → Tests passent

OWNERSHIP
□ Pas de "use after move"
□ clone() justifié par commentaire
□ Préférer &T à T en paramètre

BORROWING
□ Pas de multiple &mut simultanés
□ Emprunts scopés au minimum
□ Pas de dangling references

TYPES
□ Lifetimes explicites si retour de référence
□ Bounds de traits complets
□ try_from() au lieu de as pour conversions

ERROR HANDLING
□ Pas de unwrap() non justifié
□ ? pour propagation d'erreurs
□ Match exhaustif (pas de _ catch-all aveugle)

THREAD SAFETY
□ Arc/Mutex pour données partagées cross-thread
□ Pas de Rc en contexte multi-thread
□ #[serial(gpu)] pour tests GPU
```

---

## 🔧 Commandes de Validation

```powershell
# Check 1: Compilation
cargo check --workspace

# Check 2: Clippy avec règles strictes
cargo clippy --workspace --all-targets -- -D warnings \
  -D clippy::unwrap_used \
  -D clippy::expect_used \
  -D clippy::clone_on_ref_ptr

# Check 3: Tests
cargo test --workspace

# Check 4: Miri (détection UB) - si disponible
cargo +nightly miri test
```

---

## 📚 Référence Rapide

| Problème | Solution |
|----------|----------|
| Value moved | `clone()`, `&T`, ou restructurer |
| Multiple &mut | Scoper les emprunts, `RefCell` |
| Missing lifetime | Ajouter `<'a>` explicite |
| unwrap() panic | `?`, `expect()`, ou match |
| as truncation | `try_from()` |
| not Send | `Arc` au lieu de `Rc` |

