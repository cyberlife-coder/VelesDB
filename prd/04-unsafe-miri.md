# Phase 4 - Unsafe Audit & Miri (45 min)

## Assertions d'alignement SIMD

- [ ] Ouvrir `crates/velesdb-core/src/simd_explicit.rs`
- [ ] Chercher tous les casts de pointeurs vers types SIMD (`as *const __m128`, `as *const __m256`, etc.)
- [ ] Avant chaque cast, ajouter `debug_assert_eq!(ptr.align_offset(std::mem::align_of::<__m128>()), 0, "Pointer must be 16-byte aligned for SSE");`
- [ ] Pour AVX2 (256-bit), utiliser `align_of::<__m256>()` et message "32-byte aligned for AVX2"
- [ ] Vérifier que les allocations utilisent `Vec::with_capacity` qui garantit l'alignement correct

## SAFETY comments dans HNSW

- [ ] Ouvrir `crates/velesdb-core/src/index/hnsw/native/graph.rs`
- [ ] Chercher tous les blocs `unsafe { }` dans le fichier
- [ ] Pour chaque bloc unsafe sans commentaire SAFETY, ajouter un commentaire expliquant:
  - Pourquoi unsafe est nécessaire
  - Quels invariants sont maintenus
  - Pourquoi ces invariants garantissent la sécurité mémoire
- [ ] Exemple de format: `// SAFETY: vec.capacity() >= new_len vérifié par l'assertion ci-dessus, et les éléments sont initialisés dans la boucle précédente`
- [ ] Chercher tous les appels à `Vec::set_len()`
- [ ] Avant chaque `set_len(new_len)`, ajouter `assert!(vec.capacity() >= new_len, "Capacity {} insufficient for len {}", vec.capacity(), new_len);`
- [ ] Vérifier que les éléments entre l'ancienne et la nouvelle longueur sont initialisés

## Tests Miri ciblés

- [ ] Créer le fichier `crates/velesdb-core/tests/miri_unsafe.rs`
- [ ] Ajouter `#![cfg(miri)]` en haut du fichier pour que ces tests ne tournent qu'avec Miri
- [ ] Créer test `fn test_hnsw_graph_basic()` qui crée un petit graphe HNSW et fait des insertions/recherches
- [ ] Créer test `fn test_simd_distance_alignment()` qui teste le calcul de distance avec différentes tailles de vecteurs
- [ ] Créer test `fn test_vec_set_len_safety()` qui reproduit les patterns de set_len utilisés dans graph.rs
- [ ] Ajouter `#[test]` à chaque fonction de test

## Configuration Miri

- [ ] Vérifier que `.cargo/config.toml` ou le projet n'a pas de flags incompatibles avec Miri
- [ ] Créer `.github/workflows/miri.yml` ou modifier `ci.yml` pour ajouter un job Miri:
  - `rustup +nightly component add miri`
  - `cargo +nightly miri test -p velesdb-core --test miri_unsafe`

## Validation

- [ ] Exécuter `cargo +nightly miri setup` pour installer Miri
- [ ] Exécuter `cargo +nightly miri test -p velesdb-core --test miri_unsafe` pour valider les tests Miri
- [ ] Exécuter `cargo +nightly miri test -p velesdb-core -- hnsw --test-threads=1` pour tester HNSW sous Miri
- [ ] Si Miri trouve des erreurs, les corriger avant de continuer
- [ ] Exécuter `cargo clippy -p velesdb-core -- -D warnings -W clippy::undocumented_unsafe_blocks` pour vérifier documentation unsafe
