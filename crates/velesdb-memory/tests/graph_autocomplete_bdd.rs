//! Behaviour: the graph completes itself from plain sentences.
//!
//! `remember_extracted` used to build only a bipartite fact↔topic graph: it knew
//! a fact *mentioned* "theo durand", never that Bruno is his father. These tests
//! drive the scenario that motivated the change, one sentence at a time, exactly
//! as a user would say them:
//!
//! 1. "Bruno Durand est le pere d'Theo Durand"  → a typed entity→entity edge
//! 2. "Theo Durand a 15 ans"                    → a filterable numeric attribute
//! 3. "Theo Durand a une soeur, Camille Durand"      → a NEW entity, wired in
//!
//! The extractor is a deterministic stub keyed on the sentence, so the whole
//! behaviour is proven with no model, no network, and no flake.

#![cfg(feature = "persistence")]

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
                    "Bruno Durand est le pere d'Theo Durand.",
                    &["bruno durand", "theo durand"],
                )],
                relations: vec![relation("bruno durand", "pere de", "theo durand")],
                attributes: vec![],
            });
        }
        if text.contains("15 ans") {
            return Ok(Extraction {
                facts: vec![fact("Theo Durand a 15 ans.", &["theo durand"])],
                relations: vec![],
                attributes: vec![ExtractedAttribute {
                    entity: "theo durand".to_string(),
                    key: "age".to_string(),
                    // A JSON NUMBER on purpose: recall_where is type-strict.
                    value: json!(15),
                }],
            });
        }
        if text.contains("soeur") {
            return Ok(Extraction {
                facts: vec![fact(
                    "Theo Durand a une soeur, Camille Durand.",
                    &["theo durand", "camille durand"],
                )],
                // Both triples, mirrored, exactly as a real model returns them
                // for a possessive — see the orientation tests further down.
                relations: vec![
                    relation("theo durand", "soeur de", "camille durand"),
                    relation("camille durand", "frere de", "theo durand"),
                ],
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
                text: "Theo Durand lives in Nantes.".to_string(),
                entities: vec!["theo durand".to_string()],
            }],
            relations: vec![ExtractedRelation {
                subject: "theo durand".to_string(),
                predicate: "est".to_string(),
                object: "theo durand".to_string(),
            }],
            attributes: vec![
                ExtractedAttribute {
                    entity: "theo durand".to_string(),
                    key: "content".to_string(),
                    value: json!("hijacked"),
                },
                ExtractedAttribute {
                    entity: "theo durand".to_string(),
                    key: "_veles_hub".to_string(),
                    value: json!(false),
                },
                ExtractedAttribute {
                    entity: "theo durand".to_string(),
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
        "Bruno Durand est le pere d'Theo Durand",
        "Theo Durand a 15 ans",
        "Theo Durand a une soeur, Camille Durand",
    ] {
        svc.remember_extracted(sentence, &FamilyExtractor, None)
            .expect("extract and remember");
    }
}

#[test]
fn a_stated_relationship_becomes_a_typed_edge() {
    let (_dir, svc) = service();
    tell_the_story(&svc);

    let bruno = svc
        .entity_profile("Bruno Durand")
        .expect("profile lookup")
        .expect("bruno exists");
    let theo = svc
        .entity_profile("theo durand")
        .expect("profile lookup")
        .expect("theo exists");

    let edge = bruno
        .relations
        .iter()
        .find(|r| r.predicate == "pere de")
        .expect("bruno -[pere de]-> someone");
    assert_eq!(
        edge.target_id, theo.id,
        "the edge lands on Theo's own hub, not a parallel node"
    );
}

#[test]
fn an_entity_is_the_same_node_across_separate_sentences() {
    let (_dir, svc) = service();
    tell_the_story(&svc);

    // Theo is named in all three sentences. If entity resolution forked, the
    // age and the sister would sit on different nodes.
    let theo = svc
        .entity_profile("theo durand")
        .expect("profile lookup")
        .expect("theo exists");
    assert_eq!(theo.attributes.get("age"), Some(&json!(15)));
    assert!(
        theo.relations.iter().any(|r| r.predicate == "frere de"),
        "the same Theo node carries both the age and the sibling edge: {:?}",
        theo.relations
    );
}

#[test]
fn a_newly_mentioned_person_becomes_its_own_entity() {
    let (_dir, svc) = service();
    tell_the_story(&svc);

    let camille = svc
        .entity_profile("Camille Durand")
        .expect("profile lookup")
        .expect("camille was created from the sentence that introduced her");
    let theo = svc
        .entity_profile("theo durand")
        .expect("profile lookup")
        .expect("theo exists");
    assert_ne!(camille.id, theo.id, "Camille is a distinct node");

    let edge = camille
        .relations
        .iter()
        .find(|r| r.predicate == "soeur de")
        .expect("camille -[soeur de]-> someone");
    assert_eq!(edge.target_id, theo.id);
}

#[test]
fn a_numeric_attribute_keeps_its_json_type() {
    let (_dir, svc) = service();
    tell_the_story(&svc);

    let theo = svc
        .entity_profile("theo durand")
        .expect("profile lookup")
        .expect("theo exists");
    let age = theo.attributes.get("age").expect("age was stored");
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
    svc.remember_extracted("Theo Durand lives in Nantes", &HostileExtractor, None)
        .expect("extract and remember");

    let theo = svc
        .entity_profile("theo durand")
        .expect("profile lookup")
        .expect("theo exists");
    assert_eq!(
        theo.attributes.get("age"),
        Some(&json!(15)),
        "the age learned earlier survives a later attribute write"
    );
    assert_eq!(theo.attributes.get("ville"), Some(&json!("Nantes")));
}

#[test]
fn a_reserved_key_can_never_be_written_through_an_attribute() {
    let (_dir, svc) = service();
    svc.remember_extracted("Theo Durand lives in Nantes", &HostileExtractor, None)
        .expect("extract and remember");

    let theo = svc
        .entity_profile("theo durand")
        .expect("profile lookup")
        .expect("theo exists");
    assert_eq!(
        theo.attributes.get("content"),
        None,
        "a model emitting `content` must not overwrite the hub's own content"
    );
    assert_eq!(theo.attributes.get("_veles_hub"), None);
    // The good attribute in the same batch still landed.
    assert_eq!(theo.attributes.get("ville"), Some(&json!("Nantes")));
}

#[test]
fn a_self_loop_is_never_wired() {
    let (_dir, svc) = service();
    svc.remember_extracted("Theo Durand lives in Nantes", &HostileExtractor, None)
        .expect("extract and remember");

    let theo = svc
        .entity_profile("theo durand")
        .expect("profile lookup")
        .expect("theo exists");
    assert!(
        !theo.relations.iter().any(|r| r.target_id == theo.id),
        "an entity must never point at itself: {:?}",
        theo.relations
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
        .expect("extract and remember")
        .ids;
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
    svc.remember_extracted("Bruno Durand est le pere d'Theo Durand", &shared, None)
        .expect("extract and remember");

    let bruno = svc
        .entity_profile("bruno durand")
        .expect("profile lookup")
        .expect("bruno exists");
    assert!(
        bruno.relations.iter().any(|r| r.predicate == "pere de"),
        "Arc forwarding must not fall back to the fact-only default: {:?}",
        bruno.relations
    );
}

/// Guard the type-strictness contract end to end: the stored value must be the
/// same JSON shape a `recall_where` numeric filter compares against.
#[test]
fn a_stored_age_matches_a_numeric_comparison() {
    let (_dir, svc) = service();
    tell_the_story(&svc);
    let theo = svc
        .entity_profile("theo durand")
        .expect("profile lookup")
        .expect("theo exists");
    let age: Value = theo.attributes.get("age").cloned().expect("age");
    let as_i64 = age.as_i64().expect("age reads back as an integer");
    assert_eq!(as_i64, 15, "a numeric filter compares against this value");
}

// --- Direction of a kinship triple (issue #1653) -----------------------------
//
// A copule ("X est le pere de Y") states the relation on its own grammatical
// subject, so subject-of-sentence and subject-of-triple coincide. A possessive
// ("X a une soeur, Y") states it on the OTHER one: it is Y who is X's sister.
// The daemon took the grammatical subject either way, which does not merely
// lose an edge — it makes `entity("Camille Durand")` answer, confidently, that Camille
// is Theo's *brother*.

/// The triples the 0.11.4 daemon actually returned for the three constructions,
/// verbatim — accents, ligature and all. The copule is right; both possessive
/// sentences come back mirrored.
struct MeasuredExtractor;

impl MeasuredExtractor {
    const COPULE: &'static str = "Bruno Durand est le père d'Theo Durand.";
    const SISTER: &'static str = "Theo Durand a une sœur, Camille Durand.";
    const BROTHER: &'static str = "Marie Dupont a un frère, Paul Dupont.";
}

impl Extractor for MeasuredExtractor {
    fn extract(&self, text: &str) -> Result<Vec<ExtractedFact>, ExtractError> {
        Ok(self.extract_graph(text)?.facts)
    }

    fn extract_graph(&self, text: &str) -> Result<Extraction, ExtractError> {
        let relation = |subject: &str, predicate: &str, object: &str| ExtractedRelation {
            subject: subject.to_string(),
            predicate: predicate.to_string(),
            object: object.to_string(),
        };
        let relations = match text {
            Self::COPULE => vec![relation("bruno durand", "pere de", "theo durand")],
            Self::SISTER => vec![
                relation("theo durand", "soeur de", "camille durand"),
                relation("camille durand", "frere de", "theo durand"),
            ],
            Self::BROTHER => vec![relation("marie dupont", "frere de", "paul dupont")],
            _ => vec![],
        };
        Ok(Extraction {
            facts: vec![ExtractedFact {
                text: text.to_string(),
                entities: vec![],
            }],
            relations,
            attributes: vec![],
        })
    }
}

/// The predicates leaving `name`'s hub, so a direction can be asserted as the
/// pair (who states it, about whom) rather than "an edge exists somewhere".
fn outgoing(svc: &MemoryService<HashEmbedder>, name: &str) -> Vec<(String, u64)> {
    svc.entity_profile(name)
        .expect("profile lookup")
        .map(|profile| {
            profile
                .relations
                .into_iter()
                .map(|r| (r.predicate, r.target_id))
                .collect()
        })
        .unwrap_or_default()
}

/// The hub id of `name`, which must already exist.
fn hub_id(svc: &MemoryService<HashEmbedder>, name: &str) -> u64 {
    svc.entity_profile(name)
        .expect("profile lookup")
        .unwrap_or_else(|| panic!("{name} has a hub"))
        .id
}

/// The nominal case, and the only guard against a "fix" that flips everything:
/// a copule already binds the relation to the grammatical subject.
#[test]
fn a_copule_keeps_the_relation_on_the_grammatical_subject() {
    let (_dir, svc) = service();
    svc.remember_extracted(MeasuredExtractor::COPULE, &MeasuredExtractor, None)
        .expect("extract and remember");

    let theo = hub_id(&svc, "theo durand");
    assert!(
        outgoing(&svc, "bruno durand").contains(&("pere de".to_string(), theo)),
        "bruno -[pere de]-> theo must survive untouched, got {:?}",
        outgoing(&svc, "bruno durand")
    );
    assert!(
        outgoing(&svc, "theo durand").is_empty(),
        "the child states nothing about the father here: {:?}",
        outgoing(&svc, "theo durand")
    );
}

#[test]
fn a_possessive_binds_the_relation_to_the_person_it_introduces() {
    let (_dir, svc) = service();
    svc.remember_extracted(MeasuredExtractor::SISTER, &MeasuredExtractor, None)
        .expect("extract and remember");

    let theo = hub_id(&svc, "theo durand");
    let camille = hub_id(&svc, "camille durand");
    assert!(
        outgoing(&svc, "camille durand").contains(&("soeur de".to_string(), theo)),
        "Camille is the one introduced as the sister: camille -[soeur de]-> theo, got {:?}",
        outgoing(&svc, "camille durand")
    );
    assert!(
        outgoing(&svc, "theo durand").contains(&("frere de".to_string(), camille)),
        "and Theo is her brother: theo -[frere de]-> camille, got {:?}",
        outgoing(&svc, "theo durand")
    );
    assert!(
        !outgoing(&svc, "camille durand")
            .iter()
            .any(|(predicate, _)| predicate == "frere de"),
        "Camille must never be reported as anyone's brother: {:?}",
        outgoing(&svc, "camille durand")
    );
}

/// The same construction with a single triple and the masculine noun: the
/// converse edge is not what carries the fix.
#[test]
fn a_possessive_with_one_triple_is_oriented_too() {
    let (_dir, svc) = service();
    svc.remember_extracted(MeasuredExtractor::BROTHER, &MeasuredExtractor, None)
        .expect("extract and remember");

    let marie = hub_id(&svc, "marie dupont");
    assert!(
        outgoing(&svc, "paul dupont").contains(&("frere de".to_string(), marie)),
        "Paul is the brother: paul -[frere de]-> marie, got {:?}",
        outgoing(&svc, "paul dupont")
    );
    assert!(
        outgoing(&svc, "marie dupont").is_empty(),
        "Marie is not her own brother's brother: {:?}",
        outgoing(&svc, "marie dupont")
    );
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
    svc.remember("Bruno Durand est le pere d'Theo Durand", &[], None)
        .expect("remember");

    let bruno = svc
        .entity_profile("bruno durand")
        .expect("lookup")
        .expect("bruno exists");
    assert!(
        bruno.relations.iter().any(|r| r.predicate == "pere de"),
        "a plain remember wired the typed edge: {:?}",
        bruno.relations
    );
}

#[test]
fn autograph_is_off_unless_asked_for() {
    let (_dir, svc) = service();
    svc.remember("Bruno Durand est le pere d'Theo Durand", &[], None)
        .expect("remember");
    assert!(
        svc.entity_profile("bruno durand")
            .expect("lookup")
            .is_none(),
        "no extractor was attached, so nothing may be wired"
    );
}

#[test]
fn the_callers_fact_is_stored_verbatim_and_is_the_only_memory() {
    let (_dir, svc) = autograph_service(std::sync::Arc::new(FamilyExtractor));
    let id = svc
        .remember("Bruno Durand est le pere d'Theo Durand", &[], None)
        .expect("remember");

    let hits = svc.recall("Bruno Durand", 10, None).expect("recall");
    let mine: Vec<_> = hits.iter().filter(|h| h.id == id).collect();
    assert_eq!(mine.len(), 1, "exactly one caller-visible memory");
    assert_eq!(
        mine[0].content, "Bruno Durand est le pere d'Theo Durand",
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
        .remember("Bruno Durand est le pere d'Theo Durand", &[], None)
        .expect("remember must succeed even though the extractor is down");

    let hits = svc.recall("Bruno Durand", 5, None).expect("recall");
    assert!(
        hits.iter().any(|h| h.id == id),
        "the fact is stored and recallable"
    );
    assert!(
        svc.entity_profile("bruno durand")
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

    svc.remember_extracted("Bruno Durand est le pere d'Theo Durand", &counting, None)
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
        "Bruno Durand est le pere d'Theo Durand",
        "Theo Durand a 15 ans",
        "Theo Durand a une soeur, Camille Durand",
    ] {
        svc.remember(sentence, &[], None).expect("remember");
    }
    let theo = svc
        .entity_profile("theo durand")
        .expect("lookup")
        .expect("theo exists");
    assert_eq!(theo.attributes.get("age"), Some(&json!(15)));
    assert!(theo.relations.iter().any(|r| r.predicate == "frere de"));
}

/// The plain-`remember` path is where the defect was measured, so it gets the
/// same orientation guarantee as `remember_extracted` — the correction cannot
/// live in one write path only.
#[test]
fn autograph_orients_a_possessive_the_same_way() {
    let (_dir, svc) = autograph_service(std::sync::Arc::new(MeasuredExtractor));
    svc.remember(MeasuredExtractor::SISTER, &[], None)
        .expect("remember");

    let theo = hub_id(&svc, "theo durand");
    assert!(
        outgoing(&svc, "camille durand").contains(&("soeur de".to_string(), theo)),
        "autograph orients it too: camille -[soeur de]-> theo, got {:?}",
        outgoing(&svc, "camille durand")
    );
}
