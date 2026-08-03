//! Behaviour: a kinship triple points the way the passage states it, whatever
//! WORDS the passage and the predicate use to say the same thing.
//!
//! `reorient` decides direction by asking whether the predicate names the same
//! kinship the passage named. It asked that question with `stem == noun` — a
//! comparison of SPELLING. Two labels that mean the same relation but are not
//! spelled alike therefore read as each other's converse, and the edge is
//! stored backwards.
//!
//! Two families of passage hit this, and both are ordinary:
//!
//! * **across languages** — `KINSHIP_NOUNS` is bilingual and `POSSESSIVE_MARKERS`
//!   contains `" has a "`, so an English passage meets a French predicate as soon
//!   as a model labels in one language what it read in the other;
//! * **within one language** — the table itself lists `epoux`/`mari`,
//!   `epouse`/`femme` and `gendre`/`beau-fils`, which are synonyms, not converses.
//!
//! Every case is paired with the CONVERSE over the same nouns, which must keep
//! being re-pointed. Without that pair a "fix" that simply stopped re-orienting
//! would pass every other assertion here.

use tempfile::TempDir;
use velesdb_memory::{
    ExtractError, ExtractedFact, ExtractedRelation, Extraction, Extractor, HashEmbedder,
    MemoryService, DEFAULT_DIMENSION,
};

/// One triple as an extractor backend spells it: subject, predicate, object.
type Triple = (&'static str, &'static str, &'static str);

/// A stub returning one canned triple for the sentence under test, so the
/// behaviour is proven with no model, no network and no flake.
struct Scripted {
    triple: Triple,
}

impl Extractor for Scripted {
    fn extract(&self, text: &str) -> Result<Vec<ExtractedFact>, ExtractError> {
        Ok(self.extract_graph(text)?.facts)
    }

    fn extract_graph(&self, text: &str) -> Result<Extraction, ExtractError> {
        let (subject, predicate, object) = self.triple;
        Ok(Extraction {
            facts: vec![ExtractedFact {
                text: text.to_string(),
                entities: vec![],
            }],
            relations: vec![ExtractedRelation {
                subject: subject.to_string(),
                predicate: predicate.to_string(),
                object: object.to_string(),
            }],
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

/// The hub id of `name`, which must already exist.
fn hub_id(svc: &MemoryService<HashEmbedder>, name: &str) -> u64 {
    svc.entity_profile(name)
        .expect("profile lookup")
        .unwrap_or_else(|| panic!("{name} has a hub"))
        .id
}

/// The predicates leaving `name`'s hub, paired with the hub they point AT — so
/// a direction is asserted as (who states it, about whom), never as "an edge
/// exists somewhere".
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

/// Remember `sentence` with `triple` extracted from it, then assert `carrier`
/// is the one `predicate` hangs on and `held` the one it points at.
fn assert_carries(
    sentence: &'static str,
    triple: Triple,
    carrier: &str,
    predicate: &str,
    held: &str,
) {
    let (_dir, svc) = service();
    svc.remember_extracted(sentence, &Scripted { triple }, None)
        .expect("extract and remember");
    let target = hub_id(&svc, held);
    assert!(
        outgoing(&svc, carrier).contains(&(predicate.to_string(), target)),
        "{sentence:?} + {triple:?}: expected {carrier} -[{predicate}]-> {held}, \
         got outgoing({carrier}) = {:?}",
        outgoing(&svc, carrier)
    );
}

// ---------------------------------------------------------------------------
// The defect: same relation, different words — the edge must NOT flip.
// ---------------------------------------------------------------------------

#[test]
fn an_english_passage_with_a_french_predicate_keeps_the_sister_on_the_sister() {
    // The exact case of #1754. `"soeur"` and `"sister"` name ONE relation.
    assert_carries(
        "Theo has a sister called Camille",
        ("Camille", "soeur de", "Theo"),
        "Camille",
        "soeur de",
        "Theo",
    );
}

#[test]
fn a_french_passage_with_an_english_predicate_keeps_the_brother_on_the_brother() {
    assert_carries(
        "Theo a un frere, Marc",
        ("Marc", "brother of", "Theo"),
        "Marc",
        "brother of",
        "Theo",
    );
}

#[test]
fn two_french_synonyms_for_one_relation_do_not_flip_the_edge() {
    // `epoux` and `mari` are listed as separate nouns but name ONE relation:
    // the defect is not confined to a language boundary.
    assert_carries(
        "Theo a un epoux, Marc",
        ("Marc", "mari de", "Theo"),
        "Marc",
        "mari de",
        "Theo",
    );
}

// ---------------------------------------------------------------------------
// The guard: genuine converses must KEEP being re-pointed.
// ---------------------------------------------------------------------------

#[test]
fn a_genuine_converse_across_languages_is_still_repointed() {
    // Camille is the sister, so the triple labelled `frere de` belongs to Theo.
    assert_carries(
        "Theo has a sister called Camille",
        ("Camille", "frere de", "Theo"),
        "Theo",
        "frere de",
        "Camille",
    );
}

#[test]
fn a_genuine_converse_within_one_language_is_still_repointed() {
    assert_carries(
        "Theo a une soeur, Camille",
        ("Camille", "frere de", "Theo"),
        "Theo",
        "frere de",
        "Camille",
    );
}

// ---------------------------------------------------------------------------
// The controls already measured on develop: monolingual cases stay correct.
// ---------------------------------------------------------------------------

#[test]
fn an_all_english_passage_is_unchanged() {
    assert_carries(
        "Theo has a sister called Camille",
        ("Camille", "sister of", "Theo"),
        "Camille",
        "sister of",
        "Theo",
    );
}

#[test]
fn an_all_french_passage_is_unchanged() {
    assert_carries(
        "Theo a une soeur, Camille",
        ("Camille", "soeur de", "Theo"),
        "Camille",
        "soeur de",
        "Theo",
    );
}
