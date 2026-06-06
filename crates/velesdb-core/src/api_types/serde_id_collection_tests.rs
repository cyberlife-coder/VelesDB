//! Tests for collection-typed (`Vec<u64>`, `HashMap<String, u64>`) ID serde
//! helpers in [`super::serde_id`] guarding JS precision safety.

use super::serde_id;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;

const ABOVE_SAFE: u64 = (1_u64 << 53) + 1; // 9_007_199_254_740_993

#[derive(Serialize, Deserialize)]
struct Ids {
    #[serde(
        serialize_with = "serde_id::serialize_ids_as_strings",
        deserialize_with = "serde_id::deserialize_ids_from_string_or_number"
    )]
    ids: Vec<u64>,
}

#[derive(Serialize, Deserialize)]
struct IdMap {
    #[serde(
        serialize_with = "serde_id::serialize_id_map_as_strings",
        deserialize_with = "serde_id::deserialize_id_map_from_string_or_number"
    )]
    map: HashMap<String, u64>,
}

#[test]
fn serialize_ids_as_strings_above_max_safe_integer() {
    let value = Ids {
        ids: vec![42, ABOVE_SAFE],
    };
    let json = serde_json::to_value(&value).unwrap();
    assert_eq!(json["ids"], json!(["42", "9007199254740993"]));
}

#[test]
fn serialize_id_map_as_strings_above_max_safe_integer() {
    let value = IdMap {
        map: HashMap::from([("doc".to_string(), ABOVE_SAFE)]),
    };
    let json = serde_json::to_value(&value).unwrap();
    assert_eq!(json["map"]["doc"], json!("9007199254740993"));
}

#[test]
fn deserialize_ids_accepts_strings_and_numbers() {
    let from_strings: Ids = serde_json::from_value(json!({ "ids": ["1", "2"] })).unwrap();
    assert_eq!(from_strings.ids, vec![1, 2]);
    let from_numbers: Ids = serde_json::from_value(json!({ "ids": [1, 2] })).unwrap();
    assert_eq!(from_numbers.ids, vec![1, 2]);
}

#[test]
fn deserialize_id_map_accepts_string_and_number_values() {
    let from_string: IdMap = serde_json::from_value(json!({ "map": { "a": "1" } })).unwrap();
    assert_eq!(from_string.map.get("a"), Some(&1));
    let from_number: IdMap = serde_json::from_value(json!({ "map": { "a": 1 } })).unwrap();
    assert_eq!(from_number.map.get("a"), Some(&1));
}

#[test]
fn empty_collections_round_trip() {
    let ids = Ids { ids: vec![] };
    let ids_json = serde_json::to_value(&ids).unwrap();
    assert_eq!(ids_json["ids"], json!([]));
    let ids_back: Ids = serde_json::from_value(ids_json).unwrap();
    assert!(ids_back.ids.is_empty());

    let map = IdMap {
        map: HashMap::new(),
    };
    let map_json = serde_json::to_value(&map).unwrap();
    assert_eq!(map_json["map"], json!({}));
    let map_back: IdMap = serde_json::from_value(map_json).unwrap();
    assert!(map_back.map.is_empty());
}
