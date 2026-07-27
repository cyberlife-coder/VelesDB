//! Behaviour: the graph completes itself from plain sentences.
//!
//! `remember_extracted` used to build only a bipartite fact↔topic graph: it knew
//! a fact *mentioned* "axel lange", never that Julien is his father. These tests
//! drive the scenario that motivated the change, one sentence at a time, exactly
//! as a user would say them:
//!
//! 1. "Julien Lange est le pere d'Axel Lange"  → a typed entity→entity edge
//! 2. "Axel Lange a 15 ans"                    → a filterable numeric attribute
//! 3. "Axel Lange a une soeur, Lea Lange"      → a NEW entity, wired in
//!
//! The extractor is a deterministic stub keyed on the sentence, so the whole
//! behaviour is proven with no model, no network, and no flake.

use serde_json::{json, Value};
use tempfile::TempDir;
use velesdb_memory::{
    ExtractError, ExtractedAttribute, ExtractedFact, ExtractedRelation, Extraction, Extractor,
    HashEmbedder, MemoryService, DEFAULT_DIMENSION,
};

/// A canned graph extractor: it recognises the three sentences of the scenario
/// and returns exactly what a competent model would return for each.
struct FamilyExtractor;

impl Extractor for FamilyExtractor {
    fn extract(&self, text: &str) -> Result<Vec<ExtractedFact>, ExtractError> {
        Ok(self.extract_graph(text)?.facts)
    }

    fn extract_graph(&self, text: &str) -> Result<Extraction, ExtractError> {
        let fact = |text: &str, entities: &[&str]| ExtractedFact {
            text: text.to_string(),
            entities: entities.iter().map(|e| (*e).to_string()).collect(),
        };
        let relation = |subject: &str, predicate: &str, object: &str| ExtractedRelation {
            subject: subject.to_string(),
            predicate: predicate.to_string(),
            object: object.to_string(),
        };
        if text.contains("pere") {
            return Ok(Extraction {
                facts: vec![fact(
                    "Julien Lange est le pere d'Axel Lange.",
                    &["julien lange", "axel lange"],
                )],
                relations: vec![relation("julien lange", "pere de", "axel lange")],
                attributes: vec![],
            });
        }
        if text.contains("15 ans") {
            return Ok(Extraction {
                facts: vec![fact("Axel Lange a 15 ans.", &["axel lange"])],
                relations: vec![],
                attributes: vec![ExtractedAttribute {
                    entity: "axel lange".to_string(),
                    key: "age".to_string(),
                    // A JSON NUMBER on purpose: recall_where is type-strict.
                    value: json!(15),
                }],
            });
        }
        if text.contains("soeur") {
            return Ok(Extraction {
                facts: vec![fact(
                    "Axel Lange a une soeur, Lea Lange.",
                    &["axel lange", "lea lange"],
                )],
                relations: vec![relation("axel lange", "soeur de", "lea lange")],
                attributes: vec![],
            });
        }
        Ok(Extraction::default())
    }
}

/// An extractor that emits a reserved key and a self-loop — the hostile input
/// the wiring must survive without corrupting the hub.
struct HostileExtractor;

impl Extractor for HostileExtractor {
    fn extract(&self, text: &str) -> Result<Vec<ExtractedFact>, ExtractError> {
        Ok(self.extract_graph(text)?.facts)
    }

    fn extract_graph(&self, _text: &str) -> Result<Extraction, ExtractError> {
        Ok(Extraction {
            facts: vec![ExtractedFact {
                text: "Axel Lange lives in Nantes.".to_string(),
                entities: vec!["axel lange".to_string()],
            }],
            relations: vec![ExtractedRelation {
                subject: "axel lange".to_string(),
                predicate: "est".to_string(),
                object: "axel lange".to_string(),
            }],
            attributes: vec![
                ExtractedAttribute {
                    entity: "axel lange".to_string(),
                    key: "content".to_string(),
                    value: json!("hijacked"),
                },
                ExtractedAttribute {
                    entity: "axel lange".to_string(),
                    key: "_veles_hub".to_string(),
                    value: json!(false),
                },
                ExtractedAttribute {
                    entity: "axel lange".to_string(),
                    key: "ville".to_string(),
                    value: json!("Nantes"),
                },
            ],
        })
    }
}

/// A fresh service over a temp store. The [`TempDir`] must outlive the service.
fn service() -> (TempDir, MemoryService<HashEmbedder>) {
    let dir = tempfile::tempdir().expect("tempdir");
    let svc = MemoryService::open(dir.path(), HashEmbedder::new(DEFAULT_DIMENSION))
        .expect("open service");
    (dir, svc)
}

/// Feed the three scenario sentences, in the order a user would say them.
fn tell_the_story(svc: &MemoryService<HashEmbedder>) {
    for sentence in [
        "Julien Lange est le pere d'Axel Lange",
        "Axel Lange a 15 ans",
        "Axel Lange a une soeur, Lea Lange",
    ] {
        svc.remember_extracted(sentence, &FamilyExtractor, None)
            .expect("extract and remember");
    }
}

#[test]
fn a_stated_relationship_becomes_a_typed_edge() {
    let (_dir, svc) = service();
    tell_the_story(&svc);

    let julien = svc
        .entity_profile("Julien Lange")
        .expect("profile lookup")
        .expect("julien exists");
    let axel = svc
        .entity_profile("axel lange")
        .expect("profile lookup")
        .expect("axel exists");

    let edge = julien
        .relations
        .iter()
        .find(|r| r.predicate == "pere de")
        .expect("julien -[pere de]-> someone");
    assert_eq!(
        edge.target_id, axel.id,
        "the edge lands on Axel's own hub, not a parallel node"
    );
}

#[test]
fn an_entity_is_the_same_node_across_separate_sentences() {
    let (_dir, svc) = service();
    tell_the_story(&svc);

    // Axel is named in all three sentences. If entity resolution forked, the
    // age and the sister would sit on different nodes.
    let axel = svc
        .entity_profile("axel lange")
        .expect("profile lookup")
        .expect("axel exists");
    assert_eq!(axel.attributes.get("age"), Some(&json!(15)));
    assert!(
        axel.relations.iter().any(|r| r.predicate == "soeur de"),
        "the same Axel node carries both the age and the sister edge: {:?}",
        axel.relations
    );
}

#[test]
fn a_newly_mentioned_person_becomes_its_own_entity() {
    let (_dir, svc) = service();
    tell_the_story(&svc);

    let lea = svc
        .entity_profile("Lea Lange")
        .expect("profile lookup")
        .expect("lea was created from the sentence that introduced her");
    let axel = svc
        .entity_profile("axel lange")
        .expect("profile lookup")
        .expect("axel exists");
    assert_ne!(lea.id, axel.id, "Lea is a distinct node");

    let edge = axel
        .relations
        .iter()
        .find(|r| r.predicate == "soeur de")
        .expect("axel -[soeur de]-> someone");
    assert_eq!(edge.target_id, lea.id);
}

#[test]
fn a_numeric_attribute_keeps_its_json_type() {
    let (_dir, svc) = service();
    tell_the_story(&svc);

    let axel = svc
        .entity_profile("axel lange")
        .expect("profile lookup")
        .expect("axel exists");
    let age = axel.attributes.get("age").expect("age was stored");
    assert!(
        age.is_number(),
        "age must stay a JSON number — a string would silently never match a \
         numeric recall_where filter, got {age:?}"
    );
}

#[test]
fn learning_a_new_attribute_does_not_erase_the_previous_one() {
    let (_dir, svc) = service();
    tell_the_story(&svc);
    // A later, unrelated sentence about the same entity.
    svc.remember_extracted("Axel Lange lives in Nantes", &HostileExtractor, None)
        .expect("extract and remember");

    let axel = svc
        .entity_profile("axel lange")
        .expect("profile lookup")
        .expect("axel exists");
    assert_eq!(
        axel.attributes.get("age"),
        Some(&json!(15)),
        "the age learned earlier survives a later attribute write"
    );
    assert_eq!(axel.attributes.get("ville"), Some(&json!("Nantes")));
}

#[test]
fn a_reserved_key_can_never_be_written_through_an_attribute() {
    let (_dir, svc) = service();
    svc.remember_extracted("Axel Lange lives in Nantes", &HostileExtractor, None)
        .expect("extract and remember");

    let axel = svc
        .entity_profile("axel lange")
        .expect("profile lookup")
        .expect("axel exists");
    assert_eq!(
        axel.attributes.get("content"),
        None,
        "a model emitting `content` must not overwrite the hub's own content"
    );
    assert_eq!(axel.attributes.get("_veles_hub"), None);
    // The good attribute in the same batch still landed.
    assert_eq!(axel.attributes.get("ville"), Some(&json!("Nantes")));
}

#[test]
fn a_self_loop_is_never_wired() {
    let (_dir, svc) = service();
    svc.remember_extracted("Axel Lange lives in Nantes", &HostileExtractor, None)
        .expect("extract and remember");

    let axel = svc
        .entity_profile("axel lange")
        .expect("profile lookup")
        .expect("axel exists");
    assert!(
        !axel.relations.iter().any(|r| r.target_id == axel.id),
        "an entity must never point at itself: {:?}",
        axel.relations
    );
}

#[test]
fn an_unknown_entity_has_no_profile() {
    let (_dir, svc) = service();
    tell_the_story(&svc);
    assert!(svc
        .entity_profile("Marie Curie")
        .expect("profile lookup")
        .is_none());
    assert!(svc.entity_profile("   ").expect("profile lookup").is_none());
}

#[test]
fn a_fact_only_extractor_still_works_unchanged() {
    /// A backend written against the OLD contract: no `extract_graph` override.
    struct LegacyExtractor;
    impl Extractor for LegacyExtractor {
        fn extract(&self, _text: &str) -> Result<Vec<ExtractedFact>, ExtractError> {
            Ok(vec![ExtractedFact {
                text: "Alice ships the parser in Rust.".to_string(),
                entities: vec!["rust".to_string()],
            }])
        }
    }

    let (_dir, svc) = service();
    let ids = svc
        .remember_extracted("Alice works in Rust.", &LegacyExtractor, None)
        .expect("extract and remember");
    assert_eq!(
        ids.len(),
        1,
        "the fact-only default path still stores facts"
    );

    let rust = svc
        .entity_profile("rust")
        .expect("profile lookup")
        .expect("the topic hub still gets built");
    assert!(
        rust.attributes.is_empty(),
        "no attributes are invented for a legacy backend"
    );
}

/// An `Arc`-held extractor must not silently lose its relations: the MCP server
/// and every binding hold exactly that shape.
#[test]
fn relations_survive_an_arc_held_extractor() {
    let (_dir, svc) = service();
    let shared: std::sync::Arc<dyn Extractor + Send + Sync> = std::sync::Arc::new(FamilyExtractor);
    svc.remember_extracted("Julien Lange est le pere d'Axel Lange", &shared, None)
        .expect("extract and remember");

    let julien = svc
        .entity_profile("julien lange")
        .expect("profile lookup")
        .expect("julien exists");
    assert!(
        julien.relations.iter().any(|r| r.predicate == "pere de"),
        "Arc forwarding must not fall back to the fact-only default: {:?}",
        julien.relations
    );
}

/// Guard the type-strictness contract end to end: the stored value must be the
/// same JSON shape a `recall_where` numeric filter compares against.
#[test]
fn a_stored_age_matches_a_numeric_comparison() {
    let (_dir, svc) = service();
    tell_the_story(&svc);
    let axel = svc
        .entity_profile("axel lange")
        .expect("profile lookup")
        .expect("axel exists");
    let age: Value = axel.attributes.get("age").cloned().expect("age");
    let as_i64 = age.as_i64().expect("age reads back as an integer");
    assert_eq!(as_i64, 15, "a numeric filter compares against this value");
}

// --- Autograph: the same wiring, triggered by a plain `remember` -------------

/// Counts how many times the extractor was actually invoked, so the tests can
/// prove `remember_extracted` does NOT double-extract when autograph is on.
#[derive(Default)]
struct CountingExtractor {
    calls: std::sync::atomic::AtomicUsize,
}

impl Extractor for CountingExtractor {
    fn extract(&self, text: &str) -> Result<Vec<ExtractedFact>, ExtractError> {
        Ok(self.extract_graph(text)?.facts)
    }

    fn extract_graph(&self, text: &str) -> Result<Extraction, ExtractError> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        FamilyExtractor.extract_graph(text)
    }
}

/// A backend that is always down — the availability case autograph must
/// survive without losing the caller's fact.
struct OfflineExtractor;

impl Extractor for OfflineExtractor {
    fn extract(&self, _text: &str) -> Result<Vec<ExtractedFact>, ExtractError> {
        Err(ExtractError::Backend("model offline".to_string()))
    }
}

fn autograph_service(
    extractor: std::sync::Arc<dyn Extractor + Send + Sync>,
) -> (TempDir, MemoryService<HashEmbedder>) {
    let dir = tempfile::tempdir().expect("tempdir");
    let svc = MemoryService::open(dir.path(), HashEmbedder::new(DEFAULT_DIMENSION))
        .expect("open service")
        .with_autograph(extractor);
    (dir, svc)
}

#[test]
fn a_plain_remember_builds_the_graph_when_autograph_is_on() {
    let (_dir, svc) = autograph_service(std::sync::Arc::new(FamilyExtractor));
    svc.remember("Julien Lange est le pere d'Axel Lange", &[], None)
        .expect("remember");

    let julien = svc
        .entity_profile("julien lange")
        .expect("lookup")
        .expect("julien exists");
    assert!(
        julien.relations.iter().any(|r| r.predicate == "pere de"),
        "a plain remember wired the typed edge: {:?}",
        julien.relations
    );
}

#[test]
fn autograph_is_off_unless_asked_for() {
    let (_dir, svc) = service();
    svc.remember("Julien Lange est le pere d'Axel Lange", &[], None)
        .expect("remember");
    assert!(
        svc.entity_profile("julien lange")
            .expect("lookup")
            .is_none(),
        "no extractor was attached, so nothing may be wired"
    );
}

#[test]
fn the_callers_fact_is_stored_verbatim_and_is_the_only_memory() {
    let (_dir, svc) = autograph_service(std::sync::Arc::new(FamilyExtractor));
    let id = svc
        .remember("Julien Lange est le pere d'Axel Lange", &[], None)
        .expect("remember");

    let hits = svc.recall("Julien Lange", 10, None).expect("recall");
    let mine: Vec<_> = hits.iter().filter(|h| h.id == id).collect();
    assert_eq!(mine.len(), 1, "exactly one caller-visible memory");
    assert_eq!(
        mine[0].content, "Julien Lange est le pere d'Axel Lange",
        "stored verbatim — autograph adds structure, it never rewrites the fact"
    );
    assert_eq!(
        hits.len(),
        1,
        "the extracted sentences are NOT stored as extra memories: {:?}",
        hits.iter().map(|h| &h.content).collect::<Vec<_>>()
    );
}

/// The availability contract: the fact is already durable when extraction
/// runs, so a model that is down must cost the enrichment and nothing else.
#[test]
fn a_dead_extractor_never_costs_the_caller_their_fact() {
    let (_dir, svc) = autograph_service(std::sync::Arc::new(OfflineExtractor));
    let id = svc
        .remember("Julien Lange est le pere d'Axel Lange", &[], None)
        .expect("remember must succeed even though the extractor is down");

    let hits = svc.recall("Julien Lange", 5, None).expect("recall");
    assert!(
        hits.iter().any(|h| h.id == id),
        "the fact is stored and recallable"
    );
    assert!(
        svc.entity_profile("julien lange")
            .expect("lookup")
            .is_none(),
        "degrades to a plain remember — no half-built graph"
    );
}

/// `remember_extracted` already extracted the passage; running autograph on
/// each stored fact would re-derive what was just computed, once per fact.
#[test]
fn remember_extracted_does_not_extract_twice_under_autograph() {
    let counting = std::sync::Arc::new(CountingExtractor::default());
    let (_dir, svc) = autograph_service(counting.clone());

    svc.remember_extracted("Julien Lange est le pere d'Axel Lange", &counting, None)
        .expect("extract and remember");

    assert_eq!(
        counting.calls.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "exactly one generation for the passage, not one more per stored fact"
    );
}

#[test]
fn autograph_accumulates_across_separate_remembers() {
    let (_dir, svc) = autograph_service(std::sync::Arc::new(FamilyExtractor));
    for sentence in [
        "Julien Lange est le pere d'Axel Lange",
        "Axel Lange a 15 ans",
        "Axel Lange a une soeur, Lea Lange",
    ] {
        svc.remember(sentence, &[], None).expect("remember");
    }
    let axel = svc
        .entity_profile("axel lange")
        .expect("lookup")
        .expect("axel exists");
    assert_eq!(axel.attributes.get("age"), Some(&json!(15)));
    assert!(axel.relations.iter().any(|r| r.predicate == "soeur de"));
}
