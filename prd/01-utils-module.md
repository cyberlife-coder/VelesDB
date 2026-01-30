# Phase 1 - Module Utilitaire (55 min)

## Création du module util

- [ ] Créer le dossier `crates/velesdb-core/src/util/`
- [ ] Créer `crates/velesdb-core/src/util/mod.rs` contenant: `pub mod convert; pub mod json; pub mod checksum; pub use convert::checked_u32; pub use json::{timestamp, get_str, get_f32}; pub use checksum::crc32;`
- [ ] Créer `crates/velesdb-core/src/util/convert.rs` avec la macro `#[macro_export] macro_rules! checked_u32 { ($value:expr, $context:expr) => {{ let v: u64 = $value; assert!(v <= u32::MAX as u64, "{} {} exceeds u32::MAX", $context, v); v as u32 }}; }`
- [ ] Ajouter `pub use checked_u32;` après la macro dans convert.rs
- [ ] Créer `crates/velesdb-core/src/util/json.rs` avec `pub fn timestamp(payload: &serde_json::Value) -> Option<i64> { payload.get("timestamp").and_then(serde_json::Value::as_i64) }`
- [ ] Ajouter `pub fn get_str<'a>(payload: &'a serde_json::Value, key: &str) -> Option<&'a str> { payload.get(key).and_then(serde_json::Value::as_str) }` dans json.rs
- [ ] Ajouter `pub fn get_f32(payload: &serde_json::Value, key: &str) -> Option<f32> { payload.get(key).and_then(serde_json::Value::as_f64).map(|v| v as f32) }` dans json.rs
- [ ] Créer `crates/velesdb-core/src/util/checksum.rs` avec la constante `const CRC32_TABLE: [u32; 256]` pré-calculée (polynôme IEEE 0xEDB88320)
- [ ] Ajouter `pub fn crc32(data: &[u8]) -> u32 { let mut crc = 0xFFFFFFFF_u32; for &byte in data { let idx = ((crc ^ u32::from(byte)) & 0xFF) as usize; crc = (crc >> 8) ^ CRC32_TABLE[idx]; } !crc }` dans checksum.rs
- [ ] Ajouter `pub mod util;` dans `crates/velesdb-core/src/lib.rs` après les autres déclarations de modules

## Tests unitaires

- [ ] Ajouter `#[cfg(test)] mod tests` dans convert.rs avec test `test_checked_u32_valid` vérifiant `checked_u32!(100u64, "test") == 100u32`
- [ ] Ajouter test `#[should_panic(expected = "exceeds u32::MAX")] fn test_checked_u32_overflow()` dans convert.rs
- [ ] Ajouter `#[cfg(test)] mod tests` dans json.rs avec test vérifiant `timestamp(&json!({"timestamp": 1234567890})) == Some(1234567890)`
- [ ] Ajouter test pour `get_str` et `get_f32` dans json.rs
- [ ] Ajouter `#[cfg(test)] mod tests` dans checksum.rs avec test vérifiant `crc32(b"hello") == 0x3610A686`

## Remplacements dans le codebase

- [ ] Dans `crates/velesdb-core/src/index/bm25.rs`, chercher `assert!(u32::try_from(id).is_ok()` et remplacer par `let id_u32 = checked_u32!(id, "BM25 document ID");`
- [ ] Dans `crates/velesdb-core/src/index/bm25.rs`, supprimer le `let id_u32 = id as u32;` redondant après le remplacement
- [ ] Dans `crates/velesdb-core/src/quantization.rs`, chercher les patterns similaires d'assertion u64->u32 et remplacer par `checked_u32!`
- [ ] Dans `crates/velesdb-core/src/agent/episodic_memory.rs`, ajouter `use crate::util::json::timestamp;` en haut du fichier
- [ ] Dans episodic_memory.rs, remplacer tous les `payload.get("timestamp").and_then(serde_json::Value::as_i64)` par `timestamp(payload)`
- [ ] Dans `crates/velesdb-core/src/agent/snapshot.rs`, ajouter `use crate::util::json::timestamp;` et remplacer les appels
- [ ] Dans `crates/velesdb-core/src/agent/ttl.rs`, remplacer les appels timestamp si présents

## Validation

- [ ] Exécuter `cargo check -p velesdb-core` pour vérifier la compilation
- [ ] Exécuter `cargo test -p velesdb-core util` pour tester le nouveau module
- [ ] Exécuter `cargo test -p velesdb-core` pour vérifier qu'aucune régression n'a été introduite
- [ ] Exécuter `cargo clippy -p velesdb-core -- -D warnings` pour vérifier le code
