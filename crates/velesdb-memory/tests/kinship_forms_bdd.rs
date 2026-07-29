//! Behaviour: a kinship triple points the way the passage states it, whatever
//! FORM the passage uses to state it.
//!
//! The orientation pass shipped covering one form only — the possessive
//! ("X a une soeur, Y"). Three others reach it just as often and were left to
//! the prompt alone, which is to say unproven:
//!
//! * **alliances** — "X a un beau-frere, Y", "X has a godson, Y": the same
//!   possessive, with a noun the table did not list, so the pass walked past it.
//! * **the genitive** — "la soeur de X est Y", "X's sister is Y": the same
//!   carrier/holder reading, in the opposite word order.
//! * **the plural** — "X a deux soeurs, Y et Z": one construction naming SEVERAL
//!   carriers, of which only the first was ever oriented.
//!
//! Every form is checked in BOTH directions: the construction that must be
//! corrected, and the copule over the same nouns that must survive untouched.
//! The copule is the guard — without it a "fix" that simply mirrors everything
//! would pass every other assertion here.
//!
//! The extractor is a deterministic stub keyed on the sentence, so the whole
//! behaviour is proven with no model, no network and no flake.

use tempfile::TempDir;
use velesdb_memory::{
    ExtractError, ExtractedFact, ExtractedRelation, Extraction, Extractor, HashEmbedder,
    MemoryService, DEFAULT_DIMENSION,
};

/// One triple as an extractor backend spells it: subject, predicate, object.
type Triple = (&'static str, &'static str, &'static str);

/// A canned extractor's whole input: each sentence, with the triples a model
/// returns for it. Named, because the bare tuple type is unreadable inline.
type Script = &'static [(&'static str, &'static [Triple])];

/// The empty script entry, so a sentence the script does not know yields no
/// triples rather than panicking.
const NO_TRIPLES: &[Triple] = &[];

/// One triple, spelled the way an extractor backend returns it.
fn relation(subject: &str, predicate: &str, object: &str) -> ExtractedRelation {
    ExtractedRelation {
        subject: subject.to_string(),
        predicate: predicate.to_string(),
        object: object.to_string(),
    }
}

/// A stub returning, for each sentence, the triples a competent model returns
/// for it — mirrored where a model mirrors them, correct where it gets them
/// right. The map is the test's whole input: no branch of the pass under test
/// can influence what it produces.
struct ScriptedExtractor {
    script: Script,
}

impl Extractor for ScriptedExtractor {
    fn extract(&self, text: &str) -> Result<Vec<ExtractedFact>, ExtractError> {
        Ok(self.extract_graph(text)?.facts)
    }

    fn extract_graph(&self, text: &str) -> Result<Extraction, ExtractError> {
        let triples = self
            .script
            .iter()
            .find(|(sentence, _)| *sentence == text)
            .map_or(NO_TRIPLES, |(_, triples)| *triples);
        Ok(Extraction {
            facts: vec![ExtractedFact {
                text: text.to_string(),
                entities: vec![],
            }],
            relations: triples.iter().map(|(s, p, o)| relation(s, p, o)).collect(),
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

/// The predicates leaving `name`'s hub, so a direction is asserted as the pair
/// (who states it, about whom) rather than "an edge exists somewhere".
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

/// Remember `sentence` through `script`, then assert `carrier` is the one the
/// `predicate` hangs on and `held` the one it points at.
fn assert_carries(
    script: Script,
    sentence: &'static str,
    carrier: &str,
    predicate: &str,
    held: &str,
) {
    let (_dir, svc) = service();
    svc.remember_extracted(sentence, &ScriptedExtractor { script }, None)
        .expect("extract and remember");
    let target = hub_id(&svc, held);
    assert!(
        outgoing(&svc, carrier).contains(&(predicate.to_string(), target)),
        "{sentence:?}: expected {carrier} -[{predicate}]-> {held}, got {:?}",
        outgoing(&svc, carrier)
    );
    assert!(
        !outgoing(&svc, held)
            .iter()
            .any(|(label, _)| label == predicate),
        "{sentence:?}: {held} must not also carry {predicate}, got {:?}",
        outgoing(&svc, held)
    );
}

// --- Alliances: the possessive, with a noun the table did not list -----------

const ALLIANCES: Script = &[
    // A possessive: mirrored by the model, so the pass must re-point it.
    (
        "Marie Dupont a un beau-frère, Paul Martin.",
        &[("marie dupont", "beau-frere de", "paul martin")],
    ),
    (
        "Marie Dupont a un petit-fils, Hugo Martin.",
        &[("marie dupont", "petit-fils de", "hugo martin")],
    ),
    (
        "Marie Dupont has a brother-in-law, Paul Martin.",
        &[("marie dupont", "brother-in-law of", "paul martin")],
    ),
    (
        "Marie Dupont has a godson, Hugo Martin.",
        &[("marie dupont", "godson of", "hugo martin")],
    ),
    // A copule: already right, and must survive the pass untouched.
    (
        "Paul Martin est le beau-frère de Marie Dupont.",
        &[("paul martin", "beau-frere de", "marie dupont")],
    ),
    (
        "Hugo Martin is the godson of Marie Dupont.",
        &[("hugo martin", "godson of", "marie dupont")],
    ),
];

#[test]
fn a_possessive_naming_an_in_law_binds_it_to_the_person_it_introduces() {
    assert_carries(
        ALLIANCES,
        "Marie Dupont a un beau-frère, Paul Martin.",
        "paul martin",
        "beau-frere de",
        "marie dupont",
    );
}

#[test]
fn a_possessive_naming_a_grandchild_binds_it_to_the_person_it_introduces() {
    assert_carries(
        ALLIANCES,
        "Marie Dupont a un petit-fils, Hugo Martin.",
        "hugo martin",
        "petit-fils de",
        "marie dupont",
    );
}

/// The hyphenated English compound is the trap: read as the bare noun it
/// contains, `"brother-in-law"` would be oriented as `"brother"` — the pass
/// would then treat the sentence's own label as the CONVERSE of itself and
/// point the edge exactly the wrong way.
#[test]
fn a_hyphenated_english_compound_is_not_read_as_the_noun_it_contains() {
    assert_carries(
        ALLIANCES,
        "Marie Dupont has a brother-in-law, Paul Martin.",
        "paul martin",
        "brother-in-law of",
        "marie dupont",
    );
}

#[test]
fn a_possessive_naming_a_godchild_binds_it_to_the_person_it_introduces() {
    assert_carries(
        ALLIANCES,
        "Marie Dupont has a godson, Hugo Martin.",
        "hugo martin",
        "godson of",
        "marie dupont",
    );
}

/// The other direction, and the only guard against a "fix" that flips
/// everything: a copule already hangs the relation on its grammatical subject.
#[test]
fn a_copule_over_an_alliance_keeps_the_relation_on_its_subject() {
    assert_carries(
        ALLIANCES,
        "Paul Martin est le beau-frère de Marie Dupont.",
        "paul martin",
        "beau-frere de",
        "marie dupont",
    );
    assert_carries(
        ALLIANCES,
        "Hugo Martin is the godson of Marie Dupont.",
        "hugo martin",
        "godson of",
        "marie dupont",
    );
}

// --- The genitive: the same reading, the opposite word order -----------------
//
// A possessive names the holder first and the carrier last ("Theo a une soeur,
// Camille"). A genitive does the reverse — "la soeur DE Theo est Camille",
// "Theo's sister is Camille" — and a model mirrors it just as reliably. The
// carrier/holder reading is identical; only the surface order differs.
//
// The copule below is what stops the rule from over-reaching. "Camille est la
// soeur de Theo" contains the very same "<noun> de <holder>" fragment, and it
// is ALREADY right: what separates the two is that the genitive's copula
// follows the holder's name, while the copule's precedes the noun.

const GENITIVES: Script = &[
    // A genitive: mirrored by the model, so the pass must re-point it.
    (
        "La sœur de Theo Durand est Camille Durand.",
        &[("theo durand", "soeur de", "camille durand")],
    ),
    // The label is English because the sentence is. The extraction contract
    // asks for a predicate "in the passage's own language", and this pass reads
    // the sentence's noun and the triple's label as ONE language: a French
    // label on an English sentence is taken for the converse relation, and the
    // edge comes out backwards. Same-language is the contract, not a shortcut.
    (
        "Theo Durand's sister is Camille Durand.",
        &[("theo durand", "sister of", "camille durand")],
    ),
    (
        "The brother of Marie Dupont is Paul Dupont.",
        &[("marie dupont", "brother of", "paul dupont")],
    ),
    // A copule carrying the same fragment: already right, must not move.
    (
        "Camille Durand est la sœur de Theo Durand.",
        &[("camille durand", "soeur de", "theo durand")],
    ),
    (
        "Bruno Durand est le père de Theo Durand.",
        &[("bruno durand", "pere de", "theo durand")],
    ),
];

#[test]
fn a_french_genitive_binds_the_relation_to_the_person_the_copula_names() {
    assert_carries(
        GENITIVES,
        "La sœur de Theo Durand est Camille Durand.",
        "camille durand",
        "soeur de",
        "theo durand",
    );
}

#[test]
fn an_english_saxon_genitive_binds_the_relation_to_the_person_the_copula_names() {
    assert_carries(
        GENITIVES,
        "Theo Durand's sister is Camille Durand.",
        "camille durand",
        "sister of",
        "theo durand",
    );
}

#[test]
fn an_english_of_genitive_binds_the_relation_to_the_person_the_copula_names() {
    assert_carries(
        GENITIVES,
        "The brother of Marie Dupont is Paul Dupont.",
        "paul dupont",
        "brother of",
        "marie dupont",
    );
}

/// The guard that keeps the genitive rule from swallowing the copule: the same
/// "<noun> de <holder>" fragment appears in both, and here the triple is
/// already right. A rule that fired on the fragment alone would mirror it.
#[test]
fn a_copule_ending_on_the_same_fragment_is_left_alone() {
    assert_carries(
        GENITIVES,
        "Camille Durand est la sœur de Theo Durand.",
        "camille durand",
        "soeur de",
        "theo durand",
    );
    assert_carries(
        GENITIVES,
        "Bruno Durand est le père de Theo Durand.",
        "bruno durand",
        "pere de",
        "theo durand",
    );
}

// --- The plural: one construction, SEVERAL carriers --------------------------
//
// "Theo a deux soeurs, Camille et Lea" states two relations, not one. The pass
// read the first name after the noun and stopped, so Lea's triple — mirrored
// exactly like Camille's — was left mirrored. This is a change of FORM: the
// same construction now yields as many oriented pairs as it names.
//
// The enumeration therefore has to be BOUNDED. Walking every name after the
// noun would let the NEXT sentence contribute a carrier, and re-pointing
// "Bruno est le père de Theo" as if Bruno were a sister of Theo is a far worse
// outcome than the missing edge it replaces. Hence the second scenario.

const PLURALS: Script = &[
    (
        "Marie Dupont a une sœur, Camille Dupont, et Bruno Dupont est le père de Marie Dupont.",
        &[
            ("marie dupont", "soeur de", "camille dupont"),
            ("bruno dupont", "pere de", "marie dupont"),
        ],
    ),
    (
        "Le père de Theo Durand est Bruno Durand et Marie Dupont est la mère de Theo Durand.",
        &[
            ("theo durand", "pere de", "bruno durand"),
            ("marie dupont", "mere de", "theo durand"),
        ],
    ),
    (
        "Theo Durand a deux sœurs, Camille Durand et Lea Durand.",
        &[
            ("theo durand", "soeur de", "camille durand"),
            ("theo durand", "soeur de", "lea durand"),
        ],
    ),
    (
        "Theo Durand a une sœur, Camille Durand. Bruno Durand est le père de Theo Durand.",
        &[
            ("theo durand", "soeur de", "camille durand"),
            ("bruno durand", "pere de", "theo durand"),
        ],
    ),
];

#[test]
fn a_plural_possessive_orients_every_carrier_it_names() {
    for carrier in ["camille durand", "lea durand"] {
        assert_carries(
            PLURALS,
            "Theo Durand a deux sœurs, Camille Durand et Lea Durand.",
            carrier,
            "soeur de",
            "theo durand",
        );
    }
}

/// The enumeration must stop at the sentence it belongs to. Here the following
/// sentence names Bruno, who is Theo's father and no one's sister: an unbounded
/// walk would take him for a carrier and turn a correct edge inside out.
#[test]
fn an_enumeration_never_reaches_into_the_next_sentence() {
    const PASSAGE: &str =
        "Theo Durand a une sœur, Camille Durand. Bruno Durand est le père de Theo Durand.";
    let (_dir, svc) = service();
    svc.remember_extracted(PASSAGE, &ScriptedExtractor { script: PLURALS }, None)
        .expect("extract and remember");

    let theo = hub_id(&svc, "theo durand");
    assert!(
        outgoing(&svc, "camille durand").contains(&("soeur de".to_string(), theo)),
        "the possessive is still oriented: camille -[soeur de]-> theo, got {:?}",
        outgoing(&svc, "camille durand")
    );
    assert!(
        outgoing(&svc, "bruno durand").contains(&("pere de".to_string(), theo)),
        "the next sentence's copule must survive untouched: bruno -[pere de]-> theo, got {:?}",
        outgoing(&svc, "bruno durand")
    );
    assert!(
        outgoing(&svc, "theo durand").is_empty(),
        "Theo states nothing here — he is the holder in both, got {:?}",
        outgoing(&svc, "theo durand")
    );
}

/// A sentence can hold TWO clauses, and `", et "` is how French joins them —
/// the very string the enumeration walk strips to find another item. Bounding
/// the walk at the sentence is therefore not enough: the coordinated clause
/// hands it a name, and an edge that was ALREADY CORRECT comes out inverted.
///
/// That is strictly worse than the missing edge the pass was written to add:
/// "Bruno est le père de Marie" turning into "Marie est le père de Bruno" is
/// a confident falsehood the graph did not hold before.
#[test]
fn a_coordinated_clause_is_not_an_enumeration_item() {
    const PASSAGE: &str =
        "Marie Dupont a une sœur, Camille Dupont, et Bruno Dupont est le père de Marie Dupont.";
    let (_dir, svc) = service();
    svc.remember_extracted(PASSAGE, &ScriptedExtractor { script: PLURALS }, None)
        .expect("extract and remember");

    let marie = hub_id(&svc, "marie dupont");
    assert!(
        outgoing(&svc, "camille dupont").contains(&("soeur de".to_string(), marie)),
        "the possessive is still oriented: camille -[soeur de]-> marie, got {:?}",
        outgoing(&svc, "camille dupont")
    );
    assert!(
        outgoing(&svc, "bruno dupont").contains(&("pere de".to_string(), marie)),
        "the coordinated clause is a CLAUSE, not an item: bruno -[pere de]-> marie \
         must survive untouched, got {:?}",
        outgoing(&svc, "bruno dupont")
    );
    assert!(
        !outgoing(&svc, "marie dupont")
            .iter()
            .any(|(predicate, _)| predicate == "pere de"),
        "Marie is nobody's father — an inverted edge here is worse than no edge, got {:?}",
        outgoing(&svc, "marie dupont")
    );
}

/// Same boundary, genitive side: the walk runs after the carriers a genitive
/// names, and `" et "` joins the next clause just as readily.
#[test]
fn a_coordinated_clause_after_a_genitive_is_not_an_item() {
    const PASSAGE: &str =
        "Le père de Theo Durand est Bruno Durand et Marie Dupont est la mère de Theo Durand.";
    let (_dir, svc) = service();
    svc.remember_extracted(PASSAGE, &ScriptedExtractor { script: PLURALS }, None)
        .expect("extract and remember");

    let theo = hub_id(&svc, "theo durand");
    assert!(
        outgoing(&svc, "marie dupont").contains(&("mere de".to_string(), theo)),
        "the second clause states its own edge: marie -[mere de]-> theo, got {:?}",
        outgoing(&svc, "marie dupont")
    );
}
