---
trigger: glob
globs: ["**/*.rs"]
description: Règles strictes pour code unsafe
---

# Code Unsafe Rust

## Détection automatique

Si le fichier contient `unsafe`, les règles suivantes s'appliquent :

## Règles OBLIGATOIRES

### 1. Documentation SAFETY
Chaque bloc `unsafe` DOIT avoir un commentaire `// SAFETY:` expliquant :
- Pourquoi unsafe est nécessaire
- Quels invariants sont maintenus
- Pourquoi c'est sûr

```rust
// ✅ CORRECT
// SAFETY: We have exclusive access to the buffer and the pointer is aligned.
// The length has been validated in the caller.
unsafe {
    std::ptr::copy_nonoverlapping(src, dst, len);
}

// ❌ INCORRECT - pas de justification
unsafe {
    std::ptr::copy_nonoverlapping(src, dst, len);
}
```

### 2. Minimiser le scope

```rust
// ✅ CORRECT - scope minimal
let value = unsafe { *ptr };
process(value);

// ❌ INCORRECT - scope trop large
unsafe {
    let value = *ptr;
    process(value);  // process n'a pas besoin d'être unsafe
}
```

### 3. Vérification pré-commit

```powershell
# Tous les unsafe doivent avoir SAFETY
rg "unsafe\s*\{" --type rust -B1 | rg -v "SAFETY"
```

## Alternatives à considérer

| Pattern unsafe | Alternative safe |
|----------------|------------------|
| Raw pointers | `&[T]` slices |
| `transmute` | `bytemuck::cast` |
| `ManuallyDrop` | RAII patterns |
| FFI calls | Safe wrappers |

## Review obligatoire

Tout nouveau bloc `unsafe` nécessite :
- [ ] Justification SAFETY
- [ ] Test couvrant le code unsafe
- [ ] Review par un second développeur
