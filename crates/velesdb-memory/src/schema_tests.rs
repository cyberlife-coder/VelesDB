//! Tests for [`strip_int_formats`](super::strip_int_formats) and for the
//! input-only [`scalarize_slot_types`](super::scalarize_slot_types) pass.

use schemars::{schema_for, JsonSchema};
use serde_json::json;

use super::strip_int_formats;

#[test]
fn removes_rust_int_formats_but_keeps_standard_ones() {
    let mut schema: schemars::Schema = serde_json::from_value(json!({
        "type": "object",
        "properties": {
            "id": { "type": "integer", "format": "uint64", "minimum": 0 },
            "ids": { "type": "array", "items": { "type": "integer", "format": "uint" } },
            "when": { "type": "string", "format": "date-time" }
        }
    }))
    .expect("valid schema");

    strip_int_formats(&mut schema);

    let value = serde_json::to_value(&schema).expect("serializable");
    assert!(value["properties"]["id"].get("format").is_none());
    assert!(value["properties"]["ids"]["items"].get("format").is_none());
    // The integer constraint survives; only the non-standard format is dropped.
    assert_eq!(value["properties"]["id"]["type"], "integer");
    assert_eq!(value["properties"]["id"]["minimum"], 0);
    // Standard formats are preserved.
    assert_eq!(value["properties"]["when"]["format"], "date-time");
}

#[derive(JsonSchema)]
#[schemars(transform = strip_int_formats)]
#[allow(dead_code)]
struct Sample {
    id: u64,
    hop: usize,
}

#[test]
fn derived_schema_has_no_int_format() {
    let schema = schema_for!(Sample);
    let text = serde_json::to_string(&schema).expect("serializable");
    assert!(
        !text.contains("\"format\""),
        "derived schema still carries an int format: {text}"
    );
}

// --- scalarize_slot_types (l'entree, et l'entree seulement) ------------------

/// Fait passer un schema d'ENTREE par la scalarisation seule (sans inlining
/// ni stringification d'id), pour observer chaque regle isolement.
#[cfg(feature = "mcp")]
fn scalarized(schema: serde_json::Value) -> serde_json::Value {
    let serde_json::Value::Object(map) = schema else {
        panic!("test: le schema d'entree est un objet JSON");
    };
    let mut wire = super::WireInputSchema::adopt(map);
    super::scalarize_slot_types(&mut wire);
    serde_json::Value::Object(wire.0)
}

/// (a) `anyOf: [T, null]` → `T`, et les mots-cles freres du slot ecrasent
/// ceux de la branche promue.
#[test]
#[cfg(feature = "mcp")]
fn collapses_a_nullable_union_into_its_typed_branch() {
    let out = scalarized(json!({
        "type": "object",
        "properties": {
            "media": {
                "description": "la description du slot gagne",
                "default": null,
                "anyOf": [
                    {
                        "type": "object",
                        "description": "celle de la branche perd",
                        "properties": {"data": {"type": "string"}},
                        "required": ["data"]
                    },
                    {"type": "null"}
                ]
            }
        }
    }));

    let media = &out["properties"]["media"];
    assert_eq!(media["type"], json!("object"));
    assert_eq!(media["description"], json!("la description du slot gagne"));
    assert_eq!(media["required"], json!(["data"]));
    assert!(media.get("anyOf").is_none(), "l'union a disparu: {media}");
    assert!(
        media.get("default").is_none(),
        "(d) un `default: null` sur un slot devenu non-nullable est retire: {media}"
    );
}

/// LE CAS NON-COLLAPSABLE : une branche sans `type` direct — un `$ref` que
/// le garde de cycle de l'inliner a laisse en place — ne doit PAS etre
/// promue. La promouvoir ferait du slot un « intypable » orphelin, c'est-a-
/// dire exactement le defaut que la passe repare.
#[test]
#[cfg(feature = "mcp")]
fn keeps_a_nullable_union_whose_branch_carries_no_direct_type() {
    let schema = json!({
        "type": "object",
        "properties": {
            "source": {
                "anyOf": [{"$ref": "#/$defs/SourceReference"}, {"type": "null"}]
            }
        }
    });

    let out = scalarized(schema.clone());

    assert_eq!(
        out["properties"]["source"], schema["properties"]["source"],
        "une branche non typee reste intacte"
    );
}

/// (b) `"type": ["X", "null"]` → `"X"`, les contraintes numeriques survivant
/// intactes.
#[test]
#[cfg(feature = "mcp")]
fn collapses_a_nullable_type_list() {
    let out = scalarized(json!({
        "type": "object",
        "properties": {
            "priority": {"type": ["integer", "null"], "minimum": 0, "maximum": 255, "default": null}
        }
    }));

    let priority = &out["properties"]["priority"];
    assert_eq!(priority["type"], json!("integer"));
    assert_eq!(priority["minimum"], json!(0));
    assert_eq!(priority["maximum"], json!(255));
    assert!(priority.get("default").is_none(), "{priority}");
}

/// (c) Un `oneOf` de `const` devient un `enum` unique, les descriptions de
/// branches repliees dans celle du slot — sans quoi l'effondrement perdrait
/// ce que chaque valeur signifie (`recall_where.filters[].op`).
#[test]
#[cfg(feature = "mcp")]
fn collapses_a_const_union_into_an_enum_and_folds_the_branch_descriptions() {
    let out = scalarized(json!({
        "type": "object",
        "properties": {
            "op": {
                "description": "Comparison operator.",
                "oneOf": [
                    {"const": "eq", "description": "`=`", "type": "string"},
                    {"const": "lt", "description": "`<`", "type": "string"}
                ]
            }
        }
    }));

    let op = &out["properties"]["op"];
    assert_eq!(op["type"], json!("string"));
    assert_eq!(op["enum"], json!(["eq", "lt"]));
    assert!(op.get("oneOf").is_none(), "l'union a disparu: {op}");
    let description = op["description"].as_str().expect("une description");
    assert!(
        description.starts_with("Comparison operator."),
        "{description}"
    );
    assert!(description.contains("\"eq\": `=`"), "{description}");
    assert!(description.contains("\"lt\": `<`"), "{description}");
}

/// Les deux regles se composent : `Option<Enum>` arrive en
/// `anyOf: [{oneOf: [const…]}, null]`, dont la premiere branche ne porte
/// aucun `type` direct. C'est la descente enfants-d'abord qui la rend
/// promouvable.
#[test]
#[cfg(feature = "mcp")]
fn collapses_a_nullable_const_union_through_its_branch() {
    let out = scalarized(json!({
        "type": "object",
        "properties": {
            "format": {
                "anyOf": [
                    {"oneOf": [{"const": "plain", "type": "string"}, {"const": "jsonl", "type": "string"}]},
                    {"type": "null"}
                ]
            }
        }
    }));

    let format = &out["properties"]["format"];
    assert_eq!(format["type"], json!("string"));
    assert_eq!(format["enum"], json!(["plain", "jsonl"]));
}

/// La marche d'arbre couvre les memes chemins que l'inliner : `items`
/// simple, `items` tuple, et `additionalProperties`.
#[test]
#[cfg(feature = "mcp")]
fn walks_items_tuple_items_and_additional_properties() {
    let out = scalarized(json!({
        "type": "object",
        "properties": {
            "facts": {"type": "array", "items": {"type": "object", "properties": {
                "text": {"type": ["string", "null"]}
            }}},
            "pair": {"type": "array", "items": [
                {"type": ["integer", "null"]},
                {"type": ["boolean", "null"]}
            ]},
            "models": {"type": "object", "additionalProperties": {
                "anyOf": [{"type": "object", "properties": {}}, {"type": "null"}]
            }}
        }
    }));

    assert_eq!(
        out["properties"]["facts"]["items"]["properties"]["text"]["type"],
        json!("string")
    );
    assert_eq!(
        out["properties"]["pair"]["items"][0]["type"],
        json!("integer")
    );
    assert_eq!(
        out["properties"]["pair"]["items"][1]["type"],
        json!("boolean")
    );
    assert_eq!(
        out["properties"]["models"]["additionalProperties"]["type"],
        json!("object")
    );
}

/// Une liste de formes qui n'est pas `[X, null]` reste intacte :
/// `recall_where.filters[].value` est polymorphe par conception, et
/// l'effondrer arbitrairement mentirait sur ce que le serveur compare.
#[test]
#[cfg(feature = "mcp")]
fn leaves_a_genuinely_polymorphic_type_list_alone() {
    let out = scalarized(json!({
        "type": "object",
        "properties": {
            "value": {"type": ["number", "string", "boolean"]}
        }
    }));

    assert_eq!(
        out["properties"]["value"]["type"],
        json!(["number", "string", "boolean"])
    );
}

// --- La SORTIE, et la regle qui ne doit jamais l'atteindre -------------------

#[cfg(feature = "mcp")]
#[derive(schemars::JsonSchema)]
#[allow(dead_code)]
struct NestedOut {
    label: String,
}

#[cfg(feature = "mcp")]
#[derive(schemars::JsonSchema)]
#[allow(dead_code)]
struct OptionalOut {
    /// Ce que `load_working_context.working` et
    /// `retrieve_context_source.media` sont : une valeur legitimement absente.
    nested: Option<NestedOut>,
    /// Et la forme d'id que `widen_id_properties` produit en sortie.
    memory_id: Option<u64>,
}

/// Le durcissement de SORTIE doit garder ce qu'un `null` legitime exige.
///
/// C'est le garde de la regle que la separation par types documente sans
/// pouvoir la faire respecter a l'interieur de `schema.rs` : `scalarize_in_map`
/// y est typee sur la carte brute, donc ajouter `scalarize_in_map(&mut self.0)`
/// a [`super::WireOutputSchema::harden`] compile. Rien, jusqu'ici, ne
/// l'aurait vu : la regle de sortie de `tests/mcp_schema_bdd.rs`
/// (`announces_some_type`) est satisfaite aussi bien par `T` que par
/// `anyOf: [T, null]`.
///
/// Ce qui casserait alors n'est pas un schema, c'est une REPONSE : les SDK
/// MCP valident `structuredContent` contre l'`outputSchema` annonce (spec
/// 2025-06-18), donc un `working: null` — la reponse documentee d'une session
/// jamais sauvegardee — serait rejete chez le client.
#[test]
#[cfg(feature = "mcp")]
fn output_hardening_keeps_a_nullable_union_intact() {
    let published = super::wire_safe_output_schema::<OptionalOut>();
    let schema = serde_json::to_value(&*published).expect("le schema se serialise");

    let nested = &schema["properties"]["nested"];
    assert!(
        admits_null(nested),
        "la sortie doit continuer d'admettre `null` sur un champ optionnel, got {nested}"
    );
    let memory_id = &schema["properties"]["memory_id"];
    assert!(
        admits_null(memory_id),
        "idem pour un id optionnel, got {memory_id}"
    );
}

/// `null` est-il une valeur admise par ce slot — directement, dans une liste
/// de formes, ou par une branche d'union ?
#[cfg(feature = "mcp")]
fn admits_null(slot: &serde_json::Value) -> bool {
    match slot.get("type") {
        Some(serde_json::Value::String(kind)) => kind == "null",
        Some(serde_json::Value::Array(names)) => names.iter().any(|name| name == "null"),
        _ => ["anyOf", "oneOf"].iter().any(|keyword| {
            matches!(slot.get(*keyword), Some(serde_json::Value::Array(branches))
                if branches.iter().any(admits_null))
        }),
    }
}
