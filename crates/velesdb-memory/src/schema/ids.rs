//! Announcing id properties as plain strings on the input surface.

use serde_json::{Map, Value};

/// Mots-cles numeriques qu'un slot annonce `string` ne contraint plus.
/// JSON Schema applique `minimum`/`maximum`/`multipleOf` aux nombres
/// uniquement : les laisser sur un slot devenu `string` n'interdit rien et
/// suggere au lecteur un type que le slot n'a plus.
const INERT_NUMERIC_KEYWORDS: [&str; 6] = [
    "minimum",
    "maximum",
    "exclusiveMinimum",
    "exclusiveMaximum",
    "multipleOf",
    "format",
];

/// Mots-cles d'union qu'un slot rendu scalaire ne doit plus porter : les
/// garder ferait exiger a un validateur la conjonction du type et de la
/// branche.
const UNION_KEYWORDS: [&str; 4] = ["anyOf", "oneOf", "allOf", "$ref"];

/// Le pendant ENTREE de [`walks::widen_id_properties`](super::walks::widen_id_properties) : chaque propriete nommee
/// dans `keys` est annoncee `type: "string"`, tout court.
///
/// `widen_id_properties` reste — il sert la SORTIE, ou un id traverse en
/// entier ou en chaine selon [`crate::context::model::CompilePolicy::ids_as_strings`]
/// et ou les deux formes doivent donc etre annoncees. En ENTREE, une union
/// est detruite : les harnais clients observes aplatissent
/// `type: ["integer", "string"]` en `{}`, et le slot redevient exactement
/// l'« intypable » que tout ce module existe pour empecher. Une seule forme
/// annoncee, donc, et c'est la chaine — la seule qui traverse un client JSON
/// a nombres flottants sans perdre les bits d'un id au-dela de 2^53.
///
/// Ce que le serveur ACCEPTE est inchange : `deserialize_id` prend toujours
/// l'entier comme la chaine. On restreint ce que le client LIT, jamais ce
/// qu'il peut ENVOYER.
///
/// **La contrepartie, et elle est obligatoire :** un id annonce `string` en
/// entree doit etre LISIBLE exactement quelque part en sortie, sinon un
/// client a nombres flottants n'a plus aucune forme utilisable des deux
/// cotes. C'est le role du jumeau `<nom>_str`
/// (cf. `crate::mcp::dto`) et, pour le couple
/// `save_working_context`/`load_working_context`, de la reponse emise
/// directement en chaines. Le test
/// `every_lossy_id_in_a_result_offers_its_exact_string_twin`
/// (`tests/mcp_schema_bdd.rs`) refuse la moitie sans l'autre.
pub(crate) fn stringify_id_properties(map: &mut Map<String, Value>, keys: &[&str]) {
    if keys.is_empty() {
        return;
    }
    if let Some(Value::Object(properties)) = map.get_mut("properties") {
        for (name, subschema) in properties.iter_mut() {
            if keys.contains(&name.as_str()) {
                stringify_id_schema(subschema);
            }
        }
    }
    for value in map.values_mut() {
        stringify_in_value(value, keys);
    }
}

fn stringify_in_value(value: &mut Value, keys: &[&str]) {
    match value {
        Value::Object(map) => stringify_id_properties(map, keys),
        Value::Array(items) => items
            .iter_mut()
            .for_each(|item| stringify_in_value(item, keys)),
        _ => {}
    }
}

/// Un tableau d'ids descend sur ses `items` ; tout le reste devient un
/// `string` nu, debarrasse des mots-cles devenus inertes.
fn stringify_id_schema(schema: &mut Value) {
    let Value::Object(map) = schema else {
        return;
    };
    if declares_array(map) {
        if let Some(items) = map.get_mut("items") {
            stringify_id_schema(items);
        }
        return;
    }
    map.insert("type".to_owned(), Value::String("string".to_owned()));
    for keyword in INERT_NUMERIC_KEYWORDS.iter().chain(UNION_KEYWORDS.iter()) {
        map.remove(*keyword);
    }
}

/// `"type": "array"`, ou une liste de formes qui en contient `array`.
fn declares_array(map: &Map<String, Value>) -> bool {
    match map.get("type") {
        Some(Value::String(kind)) => kind == "array",
        Some(Value::Array(kinds)) => kinds.iter().any(|kind| kind == "array"),
        _ => false,
    }
}
