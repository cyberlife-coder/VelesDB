//! `SemanticMemory.query` in the browser: the facts nearest an embedding,
//! best first, each with its id, score and content.
//!
//! Run with `wasm-pack test --node`.

#![cfg(target_arch = "wasm32")]

use serde::Deserialize;
use wasm_bindgen_test::*;

use velesdb_wasm::SemanticMemory;

#[derive(Debug, Deserialize)]
struct Fact {
    /// The fact's id: its exact decimal string.
    id: String,
    score: f32,
    content: String,
}

fn memory() -> SemanticMemory {
    let mut memory = SemanticMemory::new(3).expect("test: memory");
    memory
        .store(1, "Paris is the capital of France", &[1.0, 0.0, 0.0])
        .expect("test: store");
    memory
        .store(2, "Berlin is the capital of Germany", &[0.0, 1.0, 0.0])
        .expect("test: store");
    memory
        .store(3, "Lyon is a city in France", &[0.9, 0.1, 0.0])
        .expect("test: store");
    memory
}

#[wasm_bindgen_test]
fn query_returns_the_nearest_facts_with_their_content() {
    let found = memory().query(&[1.0, 0.0, 0.0], 2).expect("query runs");
    let facts: Vec<Fact> = serde_wasm_bindgen::from_value(found).expect("an array of facts");

    let ids: Vec<&str> = facts.iter().map(|f| f.id.as_str()).collect();
    assert_eq!(ids, vec!["1", "3"], "nearest first, top_k honoured");
    assert_eq!(facts[0].content, "Paris is the capital of France");
    assert_eq!(facts[1].content, "Lyon is a city in France");
    assert!(facts[0].score > facts[1].score);
}

#[wasm_bindgen_test]
fn query_with_a_wrong_dimension_is_refused() {
    assert!(memory().query(&[1.0, 0.0], 2).is_err());
}
