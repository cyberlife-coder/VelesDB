# Phase 6 - Devin Review Bugs (135 min)

## Bug 1: TTL Namespace Collision (R250-272)

- [ ] Ouvrir `crates/velesdb-core/src/agent/ttl.rs`
- [ ] Ajouter `#[derive(Clone, Copy, PartialEq, Eq, Hash)] pub enum Subsystem { Semantic, Episodic, Procedural }`
- [ ] Ajouter `#[derive(Clone, Copy, PartialEq, Eq, Hash)] pub struct TtlKey { pub subsystem: Subsystem, pub id: u64 }`
- [ ] Modifier la définition de `MemoryTtl` pour changer `entries: HashMap<u64, SystemTime>` en `entries: HashMap<TtlKey, SystemTime>`
- [ ] Modifier toutes les méthodes de `MemoryTtl` pour accepter `TtlKey` au lieu de `u64`
- [ ] Ouvrir `crates/velesdb-core/src/agent/memory.rs`
- [ ] Modifier `set_semantic_ttl(id, ttl)` pour appeler `ttl.set(TtlKey { subsystem: Subsystem::Semantic, id }, ttl)`
- [ ] Modifier `set_episodic_ttl(id, ttl)` pour utiliser `Subsystem::Episodic`
- [ ] Modifier `set_procedural_ttl(id, ttl)` pour utiliser `Subsystem::Procedural`
- [ ] Créer test `test_ttl_no_cross_subsystem_collision` vérifiant que ID=1 semantic != ID=1 episodic

## Bug 2: Consolidation ID Collision (R273-285)

- [ ] Ouvrir `crates/velesdb-core/src/agent/error.rs`
- [ ] Ajouter variant `IdGenerationFailed(String)` à l'enum `AgentMemoryError`
- [ ] Implémenter Display et Error pour le nouveau variant
- [ ] Ouvrir `crates/velesdb-core/src/agent/memory.rs`
- [ ] Ajouter méthode privée `fn generate_unique_semantic_id(&self) -> Result<u64, AgentMemoryError>`:
  - `let base = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0);`
  - Boucle `for i in 0..100` essayant `base.wrapping_add(i)`
  - Si `!self.semantic.exists(candidate)?` retourner `Ok(candidate)`
  - Sinon `Err(AgentMemoryError::IdGenerationFailed(...))`
- [ ] Modifier `consolidate_old_episodes` pour vérifier collision avant store:
  - `let store_id = if self.semantic.exists(id)? { self.generate_unique_semantic_id()? } else { id };`
- [ ] Créer test `test_consolidation_no_overwrite` qui crée une entrée sémantique puis consolide un épisode avec même ID

## Bug 3: Adaptive Fetching Inefficiency (R158-189)

- [ ] Ouvrir `crates/velesdb-core/src/agent/episodic_memory.rs`
- [ ] Trouver la méthode `recent()` ou `get_recent()`
- [ ] Chercher le pattern `events = points.into_iter()...collect()` dans une boucle while
- [ ] Remplacer par:
  - Avant la boucle: `let mut events = Vec::with_capacity(limit);`
  - Dans la boucle: `events.extend(new_events.into_iter().filter(...));`
  - Ajouter: `if events.len() >= limit { break; }`
  - Après la boucle: `events.truncate(limit);`
- [ ] Appliquer le même fix à `older_than()` si le pattern existe
- [ ] Créer test avec 90% d'entrées expirées pour vérifier performance

## Bug 4: Temporal Index Misleading Comment (R63-65)

- [ ] Ouvrir `crates/velesdb-core/src/agent/memory.rs`
- [ ] Chercher `#[allow(dead_code)]` sur le champ `temporal_index`
- [ ] Supprimer l'attribut `#[allow(dead_code)]`
- [ ] Ajouter commentaire: `/// Shared temporal index for episodic events. Arc+RwLock enables multiple memory subsystems to access it.`
- [ ] Vérifier que le champ est bien utilisé quelque part (sinon le code est vraiment mort)

## Bug 5: Snapshot Restore Atomicity (R305-337)

- [ ] Ouvrir `crates/velesdb-core/src/agent/error.rs`
- [ ] Ajouter variant `CriticalRestoreFailure { backup_path: PathBuf, original_error: Box<AgentMemoryError> }`
- [ ] Ouvrir `crates/velesdb-core/src/agent/memory.rs` ou `snapshot.rs`
- [ ] Extraire le contenu de `restore()` dans `fn restore_inner(&mut self, snapshot_path: &Path) -> Result<(), AgentMemoryError>`
- [ ] Réécrire `restore()`:
  - `let backup = self.create_snapshot("backup_before_restore")?;`
  - `match self.restore_inner(snapshot_path) { Ok(_) => { std::fs::remove_file(&backup)?; Ok(()) } Err(e) => { self.restore_inner(&backup)?; Err(AgentMemoryError::CriticalRestoreFailure { ... }) } }`
- [ ] Créer test `test_restore_rollback_on_failure` qui simule une erreur et vérifie le rollback

## Bug 6: TTL Consolidation Transfer (R286-295)

- [ ] Ouvrir `crates/velesdb-core/src/agent/memory.rs`
- [ ] Trouver `consolidate_old_episodes`
- [ ] Ajouter documentation `///`:
  - `/// Consolidates old episodic events into semantic memory.`
  - `/// Note: TTL is intentionally NOT transferred. Episodic memories have time-limited relevance,`
  - `/// while semantic knowledge (consolidated facts) should persist indefinitely.`
  - `/// This follows the cognitive model where short-term memories fade but consolidated knowledge remains.`
- [ ] Créer test `test_consolidation_no_ttl_transfer` vérifiant que le TTL n'est pas copié

## Bug 7: CompositeStrategy Averaging (R404-429)

- [ ] Ouvrir `crates/velesdb-core/src/agent/reinforcement.rs`
- [ ] Trouver `impl ReinforcementStrategy for CompositeStrategy`
- [ ] Modifier `update_confidence`:
  - Au début: `if self.strategies.is_empty() { return current.clamp(0.0, 1.0); }`
  - Calculer: `let total_weight: f32 = self.strategies.iter().map(|(_, w)| w).sum();`
  - Si `total_weight <= f32::EPSILON { return current.clamp(0.0, 1.0); }`
  - Pour chaque stratégie: `result.clamp(0.0, 1.0) * weight`
  - Final: `(weighted_sum / total_weight).clamp(0.0, 1.0)`
- [ ] Créer test `test_composite_normalizes_weights` avec poids (0.5, 0.5, 0.5)
- [ ] Créer test `test_composite_clamps_result` avec stratégie retournant 1.5

## Validation

- [ ] Exécuter `cargo check -p velesdb-core` pour vérifier compilation
- [ ] Exécuter `cargo test -p velesdb-core agent` pour tester tous les modules agent
- [ ] Exécuter `cargo test -p velesdb-core -- ttl` pour tester spécifiquement TTL
- [ ] Exécuter `cargo test -p velesdb-core -- consolidat` pour tester consolidation
- [ ] Exécuter `cargo clippy -p velesdb-core -- -D warnings`
