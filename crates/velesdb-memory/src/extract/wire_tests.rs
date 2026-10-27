use super::*;
// `extraction_from_reply` lives in the reply reader: the schema and the reader
// are one contract, and the last test below is what keeps them from drifting.
use crate::extract::reply::extraction_from_reply;

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

    let reply = r#"{"facts": [{"fact": "Kaltar is fifteen.", "entities": ["kaltar"]}],
        "relations": [{"subject": "zephyrin", "predicate": "pere de", "object": "kaltar"}],
        "attributes": [{"entity": "kaltar", "key": "age", "value": 15}]}"#;
    let extraction = extraction_from_reply(reply).expect("a schema-shaped reply parses");
    assert_eq!(extraction.facts.len(), 1, "facts survive the reader");
    assert_eq!(extraction.relations.len(), 1);
    assert_eq!(extraction.attributes[0].value, serde_json::json!(15));
}
