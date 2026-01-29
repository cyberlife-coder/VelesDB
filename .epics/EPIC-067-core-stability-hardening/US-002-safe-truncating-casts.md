# US-002: Sécuriser les Truncating Casts

## Status: TODO
## Priorité: HAUTE
## Estimation: 4h

## Description

Remplacer les casts `as u32/u16/u8` par `try_from()` avec bounds check ou documenter avec `#[allow(clippy::cast_possible_truncation)]` quand le contexte garantit la sécurité.

## Contexte Technique

Les casts comme `len as u32` tronquent silencieusement si `len > u32::MAX`, causant des bugs subtils et difficiles à diagnostiquer.

## Fichiers à Modifier (53 occurrences)

### Priorité CRITIQUE (hot-path)
1. `index/hnsw/native/backend_adapter.rs` - 8 occurrences
2. `index/hnsw/native/quantization.rs` - 6 occurrences
3. `index/trigram/index.rs` - 4 occurrences

### Priorité HAUTE
4. `collection/auto_reindex/mod.rs` - 5 occurrences
5. `index/bm25.rs` - 3 occurrences
6. `quantization.rs` - 3 occurrences
7. `storage/mmap.rs` - 3 occurrences

### Priorité MOYENNE
8. `collection/graph/label_table.rs` - 2 occurrences
9. `gpu/gpu_backend.rs` - 2 occurrences
10. Autres fichiers - 17 occurrences

## Patterns de Correction

### Pattern 1: Bounds check explicite
```rust
// AVANT
let id = len as u32;

// APRÈS
let id = u32::try_from(len).map_err(|_| Error::IndexOverflow)?;
```

### Pattern 2: Assert avec message (si contexte garanti)
```rust
// AVANT
let idx = i as u32;

// APRÈS
debug_assert!(i <= u32::MAX as usize, "Index overflow");
#[allow(clippy::cast_possible_truncation)]
let idx = i as u32;
```

### Pattern 3: Saturating (si acceptable)
```rust
// AVANT
let count = total as u32;

// APRÈS
let count = total.min(u32::MAX as usize) as u32;
```

## Critères d'Acceptation

- [ ] Tous les casts `as uX` ont soit:
  - Un `try_from()` avec gestion d'erreur
  - Un `#[allow]` avec commentaire justificatif
  - Un `debug_assert!` de bounds
- [ ] Tests avec valeurs limites (u32::MAX, u32::MAX + 1)
- [ ] `cargo clippy -- -W clippy::cast_possible_truncation` passe

## Tests Requis

```rust
#[test]
fn test_large_index_handling() {
    // Test avec index proche de u32::MAX
    let large_idx: usize = u32::MAX as usize;
    let result = safe_u32_index(large_idx);
    assert!(result.is_ok());
    
    // Test overflow
    let overflow_idx: usize = u32::MAX as usize + 1;
    let result = safe_u32_index(overflow_idx);
    assert!(result.is_err());
}
```
