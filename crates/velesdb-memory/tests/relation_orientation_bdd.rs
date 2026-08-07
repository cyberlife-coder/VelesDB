//! Behaviour: a non-kinship triple keeps the exact orientation its extractor
//! stated, from extraction through storage to BOTH sides of the entity
//! profile (#1792).
//!
//! `orient_kinship` is the only pass that ever re-points a triple, and it
//! exits on any predicate that is not a kinship noun — so every other
//! relation must cross extraction → storage → `entity()` untouched. Until
//! now nothing asserted that end to end: an inversion introduced below the
//! [`Extractor`] trait (a swapped `relate()` argument, a crossed far-end pick
//! in the profile) would have shipped silently.
//!
//! Every case therefore asserts the FULL edge sets of both endpoints —
//! outgoing and incoming, exactly. A `contains`-style assertion cannot see a
//! mirrored symmetric edge ("est amie de" reads as well backwards as
//! forwards) or a doubled one; exact equality sees both.
//!
//! The oracle is deterministic throughout: the outline extractor, or a
//! scripted stub standing in for a generative backend. For the generative
//! path the invariant under test is NOT "did the model understand" — it is:
//! whatever triple the backend returned, the graph stores it with the same
//! orientation.

use tempfile::TempDir;
use velesdb_memory::{
    ExtractError, ExtractedFact, ExtractedRelation, Extraction, Extractor, HashEmbedder,
    MemoryService, OutlineExtractor, DEFAULT_DIMENSION,
};

/// One triple as an extractor backend spells it: subject, predicate, object.
type Triple = (&'static str, &'static str, &'static str);

/// One side of an entity's profile: each edge as (predicate, far-end hub id).
type EdgeSet = Vec<(String, u64)>;

/// A stub returning a fixed set of triples for any passage — the generative
/// stand-in. The triples are the test's whole input: no branch of the
/// pipeline under test can influence what the "backend" produced, so any
/// orientation difference between input and stored graph is the pipeline's.
struct ScriptedExtractor {
    triples: &'static [Triple],
}

impl Extractor for ScriptedExtractor {
    fn extract(&self, text: &str) -> Result<Vec<ExtractedFact>, ExtractError> {
        Ok(self.extract_graph(text)?.facts)
    }

    fn extract_graph(&self, text: &str) -> Result<Extraction, ExtractError> {
        Ok(Extraction {
            facts: vec![ExtractedFact {
                text: text.to_string(),
                entities: vec![],
            }],
            relations: self
                .triples
                .iter()
                .map(|(subject, predicate, object)| ExtractedRelation {
                    subject: (*subject).to_string(),
                    predicate: (*predicate).to_string(),
                    object: (*object).to_string(),
                })
                .collect(),
            attributes: vec![],
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

/// Remember `passage` through the real outline extractor.
fn outline_remember(passage: &str) -> (TempDir, MemoryService<HashEmbedder>) {
    let (dir, svc) = service();
    svc.remember_extracted(passage, &OutlineExtractor, None)
        .expect("outline remember");
    (dir, svc)
}

/// Remember `passage` through a stub that returns exactly `triples`.
fn scripted_remember(
    passage: &str,
    triples: &'static [Triple],
) -> (TempDir, MemoryService<HashEmbedder>) {
    let (dir, svc) = service();
    svc.remember_extracted(passage, &ScriptedExtractor { triples }, None)
        .expect("scripted remember");
    (dir, svc)
}

/// The hub id of `name`, which must already exist.
fn hub_id(svc: &MemoryService<HashEmbedder>, name: &str) -> u64 {
    svc.entity_profile(name)
        .expect("profile lookup")
        .unwrap_or_else(|| panic!("{name} has a hub"))
        .id
}

/// The (predicate, far-end hub id) pairs `name` sees — outgoing then
/// incoming, each side sorted so exactness does not depend on storage order.
fn edges_seen(svc: &MemoryService<HashEmbedder>, name: &str) -> (EdgeSet, EdgeSet) {
    let profile = svc
        .entity_profile(name)
        .expect("profile lookup")
        .unwrap_or_else(|| panic!("{name} has a hub"));
    let mut outgoing: EdgeSet = profile
        .relations
        .into_iter()
        .map(|r| (r.predicate, r.target_id))
        .collect();
    let mut incoming: EdgeSet = profile
        .relations_in
        .into_iter()
        .map(|r| (r.predicate, r.target_id))
        .collect();
    outgoing.sort();
    incoming.sort();
    (outgoing, incoming)
}

/// Assert `name` sees EXACTLY `expected_out` leaving it and `expected_in`
/// pointing at it — full sets, both directions, far ends resolved to hub ids.
/// Exactness is the point of this file: an inverted edge shows up as a wrong
/// side, a mirrored one as an extra element, and neither can hide.
fn assert_entity_sees(
    svc: &MemoryService<HashEmbedder>,
    name: &str,
    expected_out: &[(&str, &str)],
    expected_in: &[(&str, &str)],
) {
    let resolve = |pairs: &[(&str, &str)]| {
        let mut resolved: EdgeSet = pairs
            .iter()
            .map(|(predicate, far)| ((*predicate).to_string(), hub_id(svc, far)))
            .collect();
        resolved.sort();
        resolved
    };
    let (seen_out, seen_in) = edges_seen(svc, name);
    assert_eq!(seen_out, resolve(expected_out), "outgoing edges of {name}");
    assert_eq!(seen_in, resolve(expected_in), "incoming edges of {name}");
}

// --- Outline extractor: the graph is exactly the one the caller wrote -------

#[test]
fn an_outlined_asymmetric_edge_is_stored_the_way_the_line_states_it() {
    let (_dir, svc) = outline_remember("edge: Alice Martin | travaille chez | Wiscale");
    assert_entity_sees(&svc, "alice martin", &[("travaille chez", "wiscale")], &[]);
    assert_entity_sees(&svc, "wiscale", &[], &[("travaille chez", "alice martin")]);
}

#[test]
fn an_outlined_accented_predicate_survives_verbatim_and_oriented() {
    let (_dir, svc) = outline_remember("edge: Alice Martin | a fondé | Wiscale");
    assert_entity_sees(&svc, "alice martin", &[("a fondé", "wiscale")], &[]);
    assert_entity_sees(&svc, "wiscale", &[], &[("a fondé", "alice martin")]);
}

#[test]
fn outlined_converse_predicates_each_keep_their_own_direction() {
    let (_dir, svc) = outline_remember(
        "edge: Wiscale | emploie | Alice Martin\n\
         edge: Alice Martin | travaille pour | Wiscale",
    );
    assert_entity_sees(
        &svc,
        "alice martin",
        &[("travaille pour", "wiscale")],
        &[("emploie", "wiscale")],
    );
    assert_entity_sees(
        &svc,
        "wiscale",
        &[("emploie", "alice martin")],
        &[("travaille pour", "alice martin")],
    );
}

#[test]
fn an_outlined_symmetric_predicate_is_stored_only_in_the_stated_direction() {
    let (_dir, svc) = outline_remember("edge: Alice Martin | est amie de | Bob Durand");
    assert_entity_sees(&svc, "alice martin", &[("est amie de", "bob durand")], &[]);
    assert_entity_sees(&svc, "bob durand", &[], &[("est amie de", "alice martin")]);
}

// --- Generative stand-in: stored exactly as the backend returned it ---------

#[test]
fn a_generative_triple_is_stored_exactly_as_the_backend_returned_it() {
    let (_dir, svc) = scripted_remember(
        "Alice Martin travaille chez Wiscale depuis 2019.",
        &[("alice martin", "travaille chez", "wiscale")],
    );
    assert_entity_sees(&svc, "alice martin", &[("travaille chez", "wiscale")], &[]);
    assert_entity_sees(&svc, "wiscale", &[], &[("travaille chez", "alice martin")]);
}

#[test]
fn a_generative_english_triple_is_stored_exactly_as_returned() {
    let (_dir, svc) = scripted_remember(
        "Alice Martin founded Wiscale in 2019.",
        &[("alice martin", "founded", "wiscale")],
    );
    assert_entity_sees(&svc, "alice martin", &[("founded", "wiscale")], &[]);
    assert_entity_sees(&svc, "wiscale", &[], &[("founded", "alice martin")]);
}

#[test]
fn a_generative_symmetric_triple_is_not_mirrored() {
    let (_dir, svc) = scripted_remember(
        "Alice Martin est amie de Camille Roy.",
        &[("alice martin", "amie de", "camille roy")],
    );
    assert_entity_sees(&svc, "alice martin", &[("amie de", "camille roy")], &[]);
    assert_entity_sees(&svc, "camille roy", &[], &[("amie de", "alice martin")]);
}

// --- The kinship pass's boundary: non-kinship edges stay untouched ----------

/// The exact guard #1792 asks for: a passage that ACTIVATES the kinship pass
/// (a possessive between two people) while the same pair also carries a
/// non-kinship edge. The kinship triple must be re-pointed; the non-kinship
/// one must keep the direction the backend stated, byte for byte.
#[test]
fn the_kinship_pass_leaves_a_non_kinship_edge_on_the_same_pair_untouched() {
    let (_dir, svc) = scripted_remember(
        "Marie Dupont a un frère, Paul Martin. Paul Martin travaille avec Marie Dupont.",
        &[
            // Mirrored by the "model", as a possessive so often is — the pass
            // must re-point it onto Paul.
            ("marie dupont", "frere de", "paul martin"),
            // Stated the right way already — the pass must walk past it.
            ("paul martin", "travaille avec", "marie dupont"),
        ],
    );
    assert_entity_sees(
        &svc,
        "paul martin",
        &[
            ("frere de", "marie dupont"),
            ("travaille avec", "marie dupont"),
        ],
        &[],
    );
    assert_entity_sees(
        &svc,
        "marie dupont",
        &[],
        &[
            ("frere de", "paul martin"),
            ("travaille avec", "paul martin"),
        ],
    );
}

#[test]
fn converse_triples_across_languages_are_not_conflated() {
    let (_dir, svc) = scripted_remember(
        "Wiscale emploie Alice Martin. Alice Martin works for Wiscale.",
        &[
            ("wiscale", "emploie", "alice martin"),
            ("alice martin", "works for", "wiscale"),
        ],
    );
    assert_entity_sees(
        &svc,
        "alice martin",
        &[("works for", "wiscale")],
        &[("emploie", "wiscale")],
    );
    assert_entity_sees(
        &svc,
        "wiscale",
        &[("emploie", "alice martin")],
        &[("works for", "alice martin")],
    );
}
