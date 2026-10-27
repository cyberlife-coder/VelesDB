use super::*;

#[test]
fn graph_prompt_demands_numeric_values_and_the_three_sections() {
    let prompt = build_graph_prompt("Kaltar a 15 ans.");
    assert!(prompt.contains("Kaltar a 15 ans."));
    assert!(prompt.contains("\"relations\""));
    assert!(prompt.contains("\"attributes\""));
    assert!(prompt.contains("15, not \"15\""));
}

#[test]
fn prompt_carries_the_passage_and_json_contract() {
    let prompt = build_prompt("Alice adopted a dog in 2021.");
    assert!(prompt.contains("Alice adopted a dog in 2021."));
    assert!(prompt.contains("\"fact\": string"));
}

/// The graph prompt has to bound the predicate explicitly. Asking for "a
/// short label" was not enough: on real content the model answered
/// "est utilise pour la surveillance de fuites de donnees" — a restated
/// sentence, which makes the edge unreadable in `entity()`.
#[test]
fn graph_prompt_bounds_the_predicate_and_demands_edges() {
    let prompt = build_graph_prompt("Ahmia is an onion search engine.");
    assert!(prompt.contains("Ahmia is an onion search engine."));
    assert!(
        prompt.contains("at most 3 words"),
        "the predicate length must be a hard bound, not a suggestion"
    );
    assert!(
        prompt.contains("NEVER restate the sentence"),
        "the counter-example is what stops a restated sentence"
    );
    assert!(
        prompt.contains("at least one triple"),
        "an entity with attributes but no edge is a dead end — the prompt \
         must ask for the edge"
    );
}

/// The prompt must make the language rule SYMMETRIC (#1846). Its previous
/// form gave predicate examples in mixed languages with no rule tying the
/// answer to the passage: on an English passage, `default:fast` emitted
/// `frere de` (French) — and a graph holding both `works at` and
/// `travaille chez` for the same relation fragments it into two
/// predicates. Measured fix: with this rule the same model passes both
/// languages; a one-sided rule ("answer in French") merely inverted the
/// defect.
#[test]
fn graph_prompt_ties_every_output_to_the_passage_language() {
    let prompt = build_graph_prompt("Sarah Miller has a brother, Tom Miller.");
    assert!(
        prompt.contains("Identify the passage's language"),
        "the language rule must be an explicit first step, not an aside"
    );
    assert!(
        prompt.contains("match the PASSAGE, never the example"),
        "mixed-language examples are load-bearing — the rule must say they \
         are examples of SHAPE, not of language"
    );
}

/// The prompt must forbid the fake converse (#1846). Its previous form
/// said "add the converse ONLY if the passage states it too", and on an
/// English passage `default:fast` still emitted BOTH `tom brother of
/// sarah` AND `sarah brother of tom` — each the sibling of the other, a
/// contradiction `orient_kinship` repairs for kinship only: `works at` /
/// `manages` would ship inverted. The rule must name the failure, not
/// just permit its absence.
#[test]
fn graph_prompt_forbids_both_directions_of_one_predicate() {
    let prompt = build_graph_prompt("Sarah Miller has a brother, Tom Miller.");
    assert!(
        prompt.contains("emit exactly one"),
        "the one-per-predicate rule must be stated as a hard bound"
    );
    assert!(
        prompt.contains("keep both"),
        "the rule must carry its POSITIVE half too: two different \
         predicates stated separately are two facts — without it a model \
         collapses a real converse pair into one edge (measured 3/3 on \
         the bench's converse case)"
    );
    assert!(
        prompt.contains("is a contradiction, not a converse"),
        "the counter-example is what makes the rule unambiguous — the same \
         device the carrier rule below relies on"
    );
}

/// The prompt must state the possessive rule explicitly: asked only to
/// "state the triple in the direction the passage states it", the model
/// read the grammatical subject as the subject of the triple and mirrored
/// every possessive.
#[test]
fn graph_prompt_states_which_side_carries_the_relation() {
    let prompt = build_graph_prompt("Theo Durand a une soeur, Camille Durand.");
    assert!(
        prompt.contains("whoever CARRIES the relation"),
        "the rule must name the carrier, not just \"the direction\""
    );
    assert!(
        prompt.contains("never A/\"soeur de\"/B"),
        "the counter-example is what makes the rule unambiguous"
    );
}
