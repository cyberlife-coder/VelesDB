# Phase 2 - Factorisation Agent (65 min)

## TemporalIndex refactoring

- [ ] Ouvrir `crates/velesdb-core/src/agent/temporal_index.rs`
- [ ] Ajouter la méthode publique `pub fn rebuild_from_collection(&mut self, collection: &Collection) -> Result<(), AgentMemoryError>` qui itère sur tous les points de la collection et reconstruit l'index temporel
- [ ] Dans cette méthode, appeler `collection.iter_all()` ou équivalent pour obtenir tous les points avec leurs payloads
- [ ] Pour chaque point, extraire le timestamp via `util::json::timestamp(payload)` et appeler `self.insert(id, timestamp)`
- [ ] Ajouter la méthode `pub fn clear(&mut self)` qui vide l'index avant rebuild
- [ ] Ouvrir `crates/velesdb-core/src/agent/episodic_memory.rs`
- [ ] Chercher la méthode privée `rebuild_temporal_index` ou code similaire de reconstruction
- [ ] Remplacer ce code par un appel à `self.temporal_index.write().rebuild_from_collection(&self.collection)?`
- [ ] Supprimer le code dupliqué de reconstruction

## Snapshot restore refactoring

- [ ] Ouvrir `crates/velesdb-core/src/agent/snapshot.rs`
- [ ] Dans la méthode `restore()`, identifier le code qui reconstruit le TemporalIndex
- [ ] Remplacer par `temporal_index.write().rebuild_from_collection(&collection)?`
- [ ] Identifier le code qui remet les TTL en place après restore
- [ ] Ce code devrait utiliser `ttl.bulk_insert()` au lieu de boucler manuellement

## TTL bulk_insert

- [ ] Ouvrir `crates/velesdb-core/src/agent/ttl.rs`
- [ ] Vérifier si `MemoryTtl::bulk_insert(ids: &[u64], ttl_seconds: i64)` existe déjà
- [ ] Si non, ajouter: `pub fn bulk_insert(&mut self, ids: &[u64], ttl_seconds: i64) { let expiry = SystemTime::now() + Duration::from_secs(ttl_seconds as u64); for &id in ids { self.entries.insert(id, expiry); } }`
- [ ] Dans snapshot.rs, remplacer les boucles d'insertion TTL par `ttl.bulk_insert(&ids, ttl_seconds)`

## CRC32 déduplication

- [ ] Dans `crates/velesdb-core/src/agent/snapshot.rs`, chercher toute implémentation locale de CRC32
- [ ] Remplacer par `use crate::util::checksum::crc32;` et appeler `crc32(data)`
- [ ] Supprimer le code CRC32 dupliqué de snapshot.rs

## Validation

- [ ] Exécuter `cargo check -p velesdb-core` pour vérifier la compilation
- [ ] Exécuter `cargo test -p velesdb-core agent` pour valider tous les modules agent
- [ ] Exécuter `cargo test -p velesdb-core -- temporal_index` pour tester spécifiquement le TemporalIndex
- [ ] Exécuter `cargo test -p velesdb-core -- snapshot` pour tester le Snapshot
