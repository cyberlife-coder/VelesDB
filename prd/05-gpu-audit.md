# Phase 5 - GPU Audit (25 min)

## Tests GPU avec serial

- [ ] Ouvrir `crates/velesdb-core/src/gpu/gpu_backend.rs`
- [ ] Chercher tous les `#[test]` dans le fichier
- [ ] Pour chaque test GPU, vérifier qu'il a l'attribut `#[serial(gpu)]` de la crate `serial_test`
- [ ] Si `serial_test` n'est pas dans les dev-dependencies de Cargo.toml, l'ajouter: `serial_test = "3.0"`
- [ ] Ajouter `use serial_test::serial;` en haut du module de tests
- [ ] Ajouter `#[serial(gpu)]` à chaque test qui utilise le GPU ou `pollster::block_on`

## Timeouts GPU

- [ ] Chercher tous les appels à `pollster::block_on()` dans les tests GPU
- [ ] Envelopper chaque appel dans un timeout: `std::thread::scope(|s| { let handle = s.spawn(|| pollster::block_on(async_fn())); handle.join().expect("GPU test timed out") })`
- [ ] Alternativement, utiliser `tokio::time::timeout` si tokio est disponible dans les tests
- [ ] Définir un timeout raisonnable (ex: 30 secondes) pour éviter les deadlocks CI

## DistanceMetric compliance

- [ ] Chercher dans `gpu_backend.rs` tout calcul de distance hardcodé (dot product, cosine, euclidean)
- [ ] Vérifier que le code utilise `self.config.metric` ou paramètre `DistanceMetric`
- [ ] Si un calcul est hardcodé, le remplacer par `metric.calculate(a, b)` ou le shader correspondant
- [ ] Vérifier que les shaders WGSL supportent toutes les métriques de distance définies dans `DistanceMetric` enum
- [ ] Documenter les métriques supportées dans la doc du module GPU

## Tests de couverture GPU

- [ ] Créer test `test_gpu_cosine_similarity` qui vérifie le calcul cosine via GPU
- [ ] Créer test `test_gpu_euclidean_distance` qui vérifie le calcul euclidien via GPU
- [ ] Créer test `test_gpu_dot_product` qui vérifie le dot product via GPU
- [ ] Chaque test doit comparer le résultat GPU avec le résultat CPU (tolérance 1e-5)

## Validation

- [ ] Exécuter `cargo test -p velesdb-core gpu -- --test-threads=1` pour tester en séquentiel
- [ ] Vérifier qu'aucun test ne timeout ou deadlock
- [ ] Exécuter `cargo clippy -p velesdb-core -- -D warnings` sur le module GPU
