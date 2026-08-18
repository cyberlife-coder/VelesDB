use super::*;

/// The graph call must carry the schema as Ollama's `format` (#1944).
///
/// Measured, not assumed: passing the extraction schema moved the 8 GB
/// tier from no eligible model to two, and took one model from zero valid
/// replies to all of them. Dropping this field would undo that silently —
/// every stub-backed test would stay green, because no stub reads it.
#[test]
fn the_graph_call_constrains_decoding_with_the_extraction_schema() {
    let body = generate_body("qwen3:14b", "passage", &extraction_schema());
    assert_eq!(
        body["format"],
        extraction_schema(),
        "the graph call must send the extraction schema verbatim"
    );
    assert_eq!(body["options"]["temperature"], 0, "greedy decoding kept");
}

/// The fact-only call carries the array shape its own prompt states — the
/// two prompts return different top-level types, so one schema cannot
/// serve both.
#[test]
fn the_fact_only_call_constrains_decoding_with_the_array_schema() {
    let body = generate_body("qwen3:14b", "passage", &fact_list_schema());
    assert_eq!(body["format"]["type"], "array");
    assert_eq!(body["format"]["items"], fact_item_schema());
}

/// The schema and the parser are one contract stated twice. A reply that
/// satisfies every `required` key must survive the reader — if the two
/// drift, constrained decoding starts guaranteeing a shape nothing reads.
#[test]
fn every_required_key_of_the_schema_is_a_key_the_parser_reads() {
    let schema = extraction_schema();
    let required: Vec<&str> = schema["required"]
        .as_array()
        .expect("the extraction schema declares required keys")
        .iter()
        .map(|key| key.as_str().expect("required keys are strings"))
        .collect();
    assert_eq!(required, ["facts", "relations", "attributes"]);

    // One line on purpose: `check_prod_unwraps.py` delimits this test
    // module by matching braces, and a multi-line JSON literal unbalances
    // that count mid-module — which silently reclassifies everything after
    // it as production code (#1918 documents the same fragility).
    let reply = r#"{"facts": [{"fact": "Kaltar is fifteen.", "entities": ["kaltar"]}], "relations": [{"subject": "zephyrin", "predicate": "pere de", "object": "kaltar"}], "attributes": [{"entity": "kaltar", "key": "age", "value": 15}]}"#;
    let extraction = extraction_from_reply(reply).expect("a schema-shaped reply parses");
    assert_eq!(extraction.facts.len(), 1, "facts survive the reader");
    assert_eq!(extraction.relations.len(), 1);
    assert_eq!(extraction.attributes[0].value, serde_json::json!(15));
}

/// Regression: the graph reply is an OBJECT whose first `[` belongs to the
/// nested `facts` field. Slicing with the array preference grabbed that
/// inner list and failed to read it as the whole extraction — a real model
/// reply was rejected wholesale while every stub-backed test stayed green.
#[test]
fn parses_a_graph_reply_whose_first_bracket_is_nested() {
    let reply = r#"{ "facts": [ { "fact": "Zephyrin is the father of Kaltar.", "entities": ["zephyrin", "kaltar"] } ], "relations": [ { "subject": "zephyrin", "predicate": "pere de", "object": "kaltar" } ], "attributes": [ { "entity": "kaltar", "key": "age", "value": 15 } ] }"#;
    let raw: RawExtraction = json_slice_object(reply).expect("the object is sliced whole");
    let extraction = raw.into_extraction();
    assert_eq!(extraction.facts.len(), 1);
    assert_eq!(extraction.relations.len(), 1);
    // Endpoints asserted by name: a subject↔object swap in
    // `into_relation` kept this test green when only the predicate was
    // checked (#1792).
    assert_eq!(extraction.relations[0].subject, "zephyrin");
    assert_eq!(extraction.relations[0].predicate, "pere de");
    assert_eq!(extraction.relations[0].object, "kaltar");
    assert_eq!(extraction.attributes.len(), 1);
    assert_eq!(extraction.attributes[0].value, serde_json::json!(15));
}

/// Prose (and a fenced block) around the object must not defeat slicing.
#[test]
fn parses_a_graph_reply_wrapped_in_prose_and_fences() {
    let reply = "Here you go:\n```json\n{\"facts\": [], \"relations\": [{\"subject\": \"a\", \"predicate\": \"knows\", \"object\": \"b\"}], \"attributes\": []}\n```";
    let raw: RawExtraction = json_slice_object(reply).expect("sliced past the fence");
    assert_eq!(raw.into_extraction().relations.len(), 1);
}

/// The fact-only path must keep preferring an array: prose carrying a stray
/// `{` before the list is exactly what that preference exists to survive.
#[test]
fn fact_only_slicing_still_prefers_the_array() {
    let reply = "Result {ok}: [{\"fact\": \"A ships B.\", \"entities\": [\"b\"]}]";
    let raw: Vec<RawFact> = json_slice(reply).expect("array sliced despite the stray brace");
    assert_eq!(raw.len(), 1);
}

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

#[test]
fn parses_facts_from_a_fenced_reply() {
    let reply = "Sure!\n```json\n[{\"fact\":\"Alice adopted a dog.\",\"entities\":[\"Adoption\",\"adoption\",\"\"]}]\n```";
    let facts: Vec<RawFact> = json_slice(reply).expect("slice json");
    let facts: Vec<ExtractedFact> = facts.into_iter().filter_map(RawFact::into_fact).collect();
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].text, "Alice adopted a dog.");
    // Trimmed, lowercased, deduplicated, blanks dropped.
    assert_eq!(facts[0].entities, vec!["adoption".to_string()]);
}

#[test]
fn drops_a_textless_fact() {
    let raw = RawFact {
        fact: "   ".to_string(),
        entities: vec!["x".to_string()],
    };
    assert!(raw.into_fact().is_none());
}

#[test]
fn parses_response_envelope() {
    let text = parse_generate_response(r#"{"response":"  [] "}"#).expect("parse");
    assert_eq!(text, "[]");
}

#[test]
fn rejects_response_without_field() {
    assert!(matches!(
        parse_generate_response(r#"{"oops":true}"#),
        Err(ExtractError::Backend(_))
    ));
}
