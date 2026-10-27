use super::*;

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
