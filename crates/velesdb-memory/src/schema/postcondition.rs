//! The post-condition of the single pass: no input slot is left untyped.

use serde_json::{Map, Value};

// --- La post-condition du point de passage unique ---------------------------

/// Les slots d'un schema d'ENTREE qui n'annoncent AUCUN type.
///
/// Verifiee sur place par `McpServer::combined_router`, a la construction du
/// serveur, plutot que confiee a un test lointain : les schemas sont
/// statiques, donc une violation est deterministe et vaut mieux tot.
///
/// Volontairement plus faible que la regle du test `mcp_schema_bdd.rs`
/// (« UN type scalaire »). Elle admet `type: ["number", "string", "boolean"]`
/// — `recall_where.filters[].value`, dont la comparaison est type-stricte par
/// conception et dont le doc-comment explique pourquoi le type est epele
/// plutot que laisse vide. Une exception motivee n'a pas sa place dans un
/// `panic!` de constructeur ; elle vit dans la liste d'exemptions du test,
/// qui a un controle de peremption. Ce qui est refuse ici est le defaut
/// lui-meme : le slot qu'un harnais rend `{}`.
pub(crate) fn untyped_input_slots(map: &Map<String, Value>) -> Vec<String> {
    let mut found = Vec::new();
    collect_untyped(&Value::Object(map.clone()), "$", &mut found);
    found
}

fn collect_untyped(node: &Value, path: &str, found: &mut Vec<String>) {
    let Value::Object(map) = node else {
        return;
    };
    if let Some(Value::Object(properties)) = map.get("properties") {
        for (name, slot) in properties {
            check_untyped_slot(slot, &format!("{path}.{name}"), found);
        }
    }
    collect_untyped_items(map, path, found);
    collect_untyped_branches(map, path, found);
    if let Some(extra @ Value::Object(_)) = map.get("additionalProperties") {
        check_untyped_slot(extra, &format!("{path}.additionalProperties"), found);
    }
}

fn collect_untyped_items(map: &Map<String, Value>, path: &str, found: &mut Vec<String>) {
    match map.get("items") {
        Some(Value::Array(entries)) => {
            for (index, entry) in entries.iter().enumerate() {
                check_untyped_slot(entry, &format!("{path}.items[{index}]"), found);
            }
        }
        Some(single) => check_untyped_slot(single, &format!("{path}.items"), found),
        None => {}
    }
}

fn collect_untyped_branches(map: &Map<String, Value>, path: &str, found: &mut Vec<String>) {
    for keyword in ["anyOf", "oneOf", "allOf", "prefixItems"] {
        let Some(Value::Array(entries)) = map.get(keyword) else {
            continue;
        };
        for (index, entry) in entries.iter().enumerate() {
            check_untyped_slot(entry, &format!("{path}.{keyword}[{index}]"), found);
        }
    }
}

fn check_untyped_slot(slot: &Value, path: &str, found: &mut Vec<String>) {
    let typed = match slot {
        Value::Object(map) => {
            map.contains_key("type") || map.contains_key("enum") || map.contains_key("const")
        }
        _ => false,
    };
    if !typed {
        found.push(format!("{path} = {slot}"));
    }
    collect_untyped(slot, path, found);
}
