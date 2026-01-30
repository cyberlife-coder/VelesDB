# Phase 7 - Documentation & CI (50 min)

## Documentation episodic_memory.rs

- [ ] Ouvrir `crates/velesdb-core/src/agent/episodic_memory.rs`
- [ ] Ajouter `//! Episodic memory subsystem for storing time-bound events and experiences.` en haut du fichier
- [ ] Pour chaque `pub struct`, ajouter `/// Description of the struct` avant la définition
- [ ] Pour chaque `pub fn`, ajouter `/// Description`, `/// # Arguments`, `/// # Returns`, `/// # Errors` si applicable
- [ ] Pour chaque champ public, ajouter `/// Description du champ`

## Documentation semantic_memory.rs

- [ ] Ouvrir `crates/velesdb-core/src/agent/semantic_memory.rs`
- [ ] Ajouter `//! Semantic memory subsystem for storing long-term knowledge and facts.`
- [ ] Documenter toutes les structs publiques avec `///`
- [ ] Documenter toutes les méthodes publiques avec `///`

## Documentation procedural_memory.rs

- [ ] Ouvrir `crates/velesdb-core/src/agent/procedural_memory.rs`
- [ ] Ajouter `//! Procedural memory subsystem for storing skills, habits, and learned behaviors.`
- [ ] Documenter toutes les structs publiques avec `///`
- [ ] Documenter toutes les méthodes publiques avec `///`

## Documentation autres modules agent

- [ ] Ouvrir `crates/velesdb-core/src/agent/temporal_index.rs` et documenter structs/fonctions publiques
- [ ] Ouvrir `crates/velesdb-core/src/agent/ttl.rs` et documenter `MemoryTtl`, `TtlKey`, `Subsystem`
- [ ] Ouvrir `crates/velesdb-core/src/agent/snapshot.rs` et documenter `Snapshot`, `SnapshotMetadata`
- [ ] Ouvrir `crates/velesdb-core/src/agent/reinforcement.rs` et documenter `ReinforcementStrategy`, `CompositeStrategy`

## Suppression allow(missing_docs)

- [ ] Chercher `#![allow(missing_docs)]` dans tous les fichiers du module agent
- [ ] Supprimer chaque occurrence trouvée
- [ ] Exécuter `cargo doc --no-deps -p velesdb-core 2>&1 | grep "missing documentation"` pour lister les docs manquantes
- [ ] Corriger chaque warning de documentation manquante

## CI Miri job

- [ ] Ouvrir `.github/workflows/ci.yml`
- [ ] Ajouter un nouveau job `miri-check` après les tests normaux:
```yaml
miri-check:
  runs-on: ubuntu-latest
  steps:
    - uses: actions/checkout@v4
    - uses: dtolnay/rust-toolchain@nightly
      with:
        components: miri
    - name: Run Miri
      run: cargo +nightly miri test -p velesdb-core --test miri_unsafe
```
- [ ] Ajouter `needs: [test]` pour que Miri ne tourne qu'après les tests normaux

## CHANGELOG.md

- [ ] Ouvrir `CHANGELOG.md` à la racine du projet
- [ ] Ajouter une nouvelle section `## [Unreleased]` si elle n'existe pas
- [ ] Ajouter sous-section `### Changed`:
  - `- Refactored util module with checked_u32!, json helpers, and crc32`
  - `- Unified TemporalIndex rebuild API`
  - `- Improved SIMD dispatch with PDX layout and Teddy trigrams (arXiv-based)`
- [ ] Ajouter sous-section `### Fixed`:
  - `- TTL namespace collision between memory subsystems`
  - `- Consolidation ID collision with semantic entries`
  - `- Adaptive fetching now accumulates results correctly`
  - `- Snapshot restore is now atomic with rollback on failure`
  - `- CompositeStrategy normalizes weights and clamps results`
- [ ] Ajouter sous-section `### Added`:
  - `- Miri tests for unsafe code validation`
  - `- SIMD benchmarks with Criterion`

## Validation

- [ ] Exécuter `cargo doc --no-deps -p velesdb-core` et vérifier 0 warnings
- [ ] Ouvrir `target/doc/velesdb_core/index.html` dans un navigateur pour vérifier la doc
- [ ] Vérifier que la CI passe localement avec `act` ou en créant une PR de test
