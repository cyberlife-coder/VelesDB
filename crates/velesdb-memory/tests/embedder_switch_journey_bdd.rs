//! Behaviour: switching embedders NEVER costs the user their memories — the
//! whole journey, across the three layers that each own a piece of it.
//!
//! The end-user audit scenario this pins: start on the default `hash`
//! embedder, accumulate weeks of facts, discover semantic recall, flip
//! `VELESDB_MEMORY_EMBEDDER` — and every stored memory silently becomes
//! unfindable, because vectors from two models are not comparable. Each
//! layer's own suite proves its piece (provenance refuses:
//! `embedding_provenance` tests; the rebuild re-embeds: `migration::tests`;
//! recall works: service tests) — but nothing proved the JOURNEY: that the
//! refusal names the recovery, the recovery accepts exactly this store, and
//! what comes out the other side still answers the user's question. A chain
//! whose links are each tested can still be unwalkable; this file walks it.
//!
//! Layer boundaries deliberately crossed, nothing mocked but the second
//! embedder (a `HashEmbedder` at a different width — the migration engine
//! cannot tell it from a real remote model, and nothing here may contact a
//! network).

#![cfg(feature = "persistence")]

use velesdb_memory::embedding_provenance::{self, EmbeddingProvenance};
use velesdb_memory::migration::{self, TargetContract};
use velesdb_memory::{HashEmbedder, MemoryService, DEFAULT_DIMENSION};

const OLD_MODEL: &str = "hash";
const NEW_MODEL: &str = "bge-mock";
const NEW_DIMENSION: usize = 1024;

#[test]
fn a_hash_store_survives_the_switch_to_a_semantic_embedder() {
    let root = tempfile::tempdir().expect("scratch root");
    let store_dir = root.path().join("store");

    // ACT 1 — life on the default: facts accumulate under `hash`, and the
    // daemon's startup path records what filled the store (#1751).
    let fact_id;
    {
        let service = MemoryService::open(&store_dir, HashEmbedder::new(DEFAULT_DIMENSION))
            .expect("open the store under the default embedder");
        fact_id = service
            .remember(
                "le timeout API est fixe a 8 secondes a cause de l'incident INC-42",
                &[],
                None,
            )
            .expect("remember a fact worth keeping");
        service
            .remember("le port du serveur est 6333", &[], None)
            .expect("remember a second fact");
    }
    embedding_provenance::write(
        &store_dir,
        &EmbeddingProvenance::new(OLD_MODEL, DEFAULT_DIMENSION),
    )
    .expect("record the filling model, as the daemon does on first open");

    // ACT 2 — the flip. The daemon's pre-open check must REFUSE (silently
    // serving nonsense is the audit's failure mode), and the refusal must
    // name the recovery command — the user's next step lives in this string.
    let recorded = embedding_provenance::read(&store_dir)
        .expect("read the record back")
        .expect("the record exists");
    let refusal = embedding_provenance::check(Some(&recorded), NEW_MODEL, NEW_DIMENSION)
        .expect_err("two different models must be refused, never served");
    assert!(
        refusal.contains("migrate-embeddings"),
        "the refusal must name the recovery command — a dead end here is the \
         difference between a migration and a lost store: {refusal}"
    );

    // ACT 3 — the recovery the refusal named: re-embed everything into a
    // destination and switch over, journaled. The new embedder produces
    // 1024-wide vectors; the engine cannot tell this stand-in from a real
    // remote model.
    let new_embedder = HashEmbedder::new(NEW_DIMENSION);
    let destination = root.path().join("rebuilt");
    let outcome = migration::migrate(
        &store_dir,
        root.path(),
        &TargetContract::automatic(NEW_MODEL, NEW_DIMENSION),
        &destination,
        &new_embedder,
        256,
    )
    .expect("the named recovery must accept exactly the store the refusal described");
    assert!(
        outcome.executed.is_some(),
        "a fresh migration performs the rebuild — a no-op here would mean the \
         journal claimed work this run never did"
    );
    assert_eq!(
        outcome.switched.activated,
        store_dir.canonicalize().expect("the store path exists"),
        "the switch activates the rebuilt store AT THE SOURCE'S OWN PATH — \
         the user's configuration keeps pointing where it always did"
    );

    // ACT 4 — the other side. The store now opens under the NEW identity…
    let migrated = embedding_provenance::read(&store_dir)
        .expect("read the migrated record")
        .expect("the migrated store carries a record");
    assert_eq!(
        (migrated.model.as_str(), migrated.dimension),
        (NEW_MODEL, NEW_DIMENSION),
        "the store's recorded identity is the one the user configured"
    );
    embedding_provenance::check(Some(&migrated), NEW_MODEL, NEW_DIMENSION)
        .expect("the daemon's next startup accepts the migrated store");

    // …and the memories are still THERE and still FINDABLE: same ids, same
    // content, recalled through the new embedder's own vectors.
    let service = MemoryService::open(&store_dir, new_embedder)
        .expect("open the migrated store under the new embedder");
    let recalled = service
        .recall(
            "le timeout API est fixe a 8 secondes a cause de l'incident INC-42",
            5,
            None,
        )
        .expect("recall over the migrated store");
    assert!(
        recalled
            .iter()
            .any(|m| m.id == fact_id && m.content.contains("INC-42")),
        "the fact stored under the OLD embedder is findable under the NEW one, \
         under its ORIGINAL id — this line is the product promise: switching \
         embedders never costs the user their memories"
    );
}
