//! Collapsing every input slot to one scalar type.

use super::wire::WireInputSchema;
use serde_json::{Map, Value};

// --- La scalarisation des slots d'ENTREE ------------------------------------

/// Effondre chaque slot d'un schema d'ENTREE en UN type scalaire.
///
/// La preuve etablie le 2026-07-29, en interrogeant le serveur en JSON-RPC
/// brut : le serveur emettait deja un schema d'entree CORRECT — une union
/// `["integer", "string"]` sur les slots d'id, un `anyOf: [T, null]` sur les
/// champs optionnels — et c'est le HARNAIS qui aplatit toute union en `{}`.
/// On ne corrige pas les harnais ; on cesse d'emettre une union sur une
/// ENTREE.
///
/// Attention en verifiant : `docs/reference/mcp-tools.json` est regenere
/// APRES cette passe, il enregistre donc l'etat d'ARRIVEE et ne peut pas
/// porter la trace de l'union de depart. Ce que l'artefact montre encore,
/// c'est la meme union LA OU ELLE SURVIT — la SORTIE, que cette passe ne
/// touche pas : `compile_context.decisions[].fragment_id` y vaut toujours
/// `["integer", "string"]`, et `load_working_context.working` un
/// `anyOf: [WorkingContext, null]`. C'est cette forme-la, sur l'entree,
/// qu'on a cesse de publier.
///
/// Quatre formes s'effondrent :
/// - `anyOf`/`oneOf` `[T, {"type": "null"}]` → `T`, **seulement** si `T`
///   porte deja un `type` direct : un `$ref` laisse en place par le garde de
///   cycle d'[`inline_ref_only_properties`](super::inline::inline_ref_only_properties) deviendrait sinon un slot
///   orphelin, c'est-a-dire exactement le defaut qu'on repare ;
/// - `"type": ["X", "null"]` → `"X"` ;
/// - un `oneOf` de `const` → `{"type": "string", "enum": [...]}`, les
///   descriptions de branches repliees dans celle du slot (ce qui sauve
///   `recall_where.filters[].op` et les enums de politique) ;
/// - un `"default": null` devenu inerte disparait.
///
/// N'accepte QUE [`WireInputSchema`] : appliquee a une sortie, elle ferait
/// echouer la validation de `structuredContent` chez le client (voir le
/// commentaire de section plus bas). S'applique APRES
/// [`inline_ref_only_properties`](super::inline::inline_ref_only_properties) — une branche doit deja etre inlinee pour
/// porter un type promouvable.
pub(crate) fn scalarize_slot_types(schema: &mut WireInputSchema) {
    scalarize_in_map(schema.map_mut());
}

/// Les mots-cles que [`scalarize_in_map`] parcourt comme chemins d'argument,
/// chacun avec sa propre passe — listes ici une seule fois pour que la
/// descente generique ne les reparcoure pas.
const SCALARIZE_SLOT_KEYWORDS: [&str; 6] = [
    "properties",
    "items",
    "anyOf",
    "oneOf",
    "prefixItems",
    "additionalProperties",
];

/// Mots-cles dont la valeur est une DONNEE et non un sous-schema. Descendre
/// dedans reviendrait a reecrire une valeur d'exemple parce qu'elle porte
/// une cle qui ressemble a un mot-cle de schema.
const DATA_KEYWORDS: [&str; 4] = ["enum", "const", "default", "examples"];

fn scalarize_in_map(map: &mut Map<String, Value>) {
    scalarize_property_slots(map);
    scalarize_item_slots(map);
    scalarize_union_branches(map);
    scalarize_additional_properties(map);
    scalarize_remaining_keywords(map);
}

fn scalarize_property_slots(map: &mut Map<String, Value>) {
    if let Some(Value::Object(properties)) = map.get_mut("properties") {
        for slot in properties.values_mut() {
            scalarize_slot(slot);
        }
    }
}

fn scalarize_item_slots(map: &mut Map<String, Value>) {
    match map.get_mut("items") {
        // Forme tuple : `items` est un tableau de schemas par position.
        Some(Value::Array(entries)) => entries.iter_mut().for_each(scalarize_slot),
        Some(single) => scalarize_slot(single),
        None => {}
    }
}

fn scalarize_union_branches(map: &mut Map<String, Value>) {
    for keyword in ["anyOf", "oneOf", "prefixItems"] {
        if let Some(Value::Array(branches)) = map.get_mut(keyword) {
            branches.iter_mut().for_each(scalarize_slot);
        }
    }
}

fn scalarize_additional_properties(map: &mut Map<String, Value>) {
    if let Some(extra @ Value::Object(_)) = map.get_mut("additionalProperties") {
        scalarize_slot(extra);
    }
}

fn scalarize_remaining_keywords(map: &mut Map<String, Value>) {
    for (key, value) in map.iter_mut() {
        let name = key.as_str();
        if !SCALARIZE_SLOT_KEYWORDS.contains(&name) && !DATA_KEYWORDS.contains(&name) {
            scalarize_in_value(value);
        }
    }
}

fn scalarize_in_value(value: &mut Value) {
    match value {
        Value::Object(map) => scalarize_in_map(map),
        Value::Array(items) => items.iter_mut().for_each(scalarize_in_value),
        _ => {}
    }
}

/// Un slot : ses enfants D'ABORD, lui-meme ensuite.
///
/// L'ordre n'est pas cosmetique. Une branche doit etre effondree avant
/// d'etre promue : `Option<SegmentFormat>` arrive en
/// `anyOf: [{oneOf: [const…]}, null]`, dont la premiere branche ne porte
/// aucun `type` direct. C'est la regle (c) appliquee a la branche qui lui en
/// donne un, et donc la regle (a) qui devient applicable au slot.
fn scalarize_slot(slot: &mut Value) {
    let Value::Object(map) = slot else {
        return;
    };
    scalarize_in_map(map);
    collapse_nullable_union(map);
    collapse_const_union(map);
    collapse_nullable_type_list(map);
    drop_inert_null_default(map);
}

/// (a) `anyOf`/`oneOf` `[T, {"type": "null"}]` → `T`.
fn collapse_nullable_union(map: &mut Map<String, Value>) {
    for keyword in ["anyOf", "oneOf"] {
        if let Some(promoted) = nullable_union_branch(map, keyword) {
            promote_branch(map, keyword, promoted);
            return;
        }
    }
}

/// La branche non-nulle d'une union binaire nullable, si elle porte deja un
/// `type` scalaire direct. Sinon `None` — un `$ref` survivant ne doit jamais
/// devenir un slot orphelin.
fn nullable_union_branch(map: &Map<String, Value>, keyword: &str) -> Option<Map<String, Value>> {
    let Value::Array(branches) = map.get(keyword)? else {
        return None;
    };
    if branches.len() != 2 {
        return None;
    }
    let kept = match (is_null_branch(&branches[0]), is_null_branch(&branches[1])) {
        (false, true) => &branches[0],
        (true, false) => &branches[1],
        _ => return None,
    };
    let Value::Object(kept) = kept else {
        return None;
    };
    matches!(kept.get("type"), Some(Value::String(_))).then(|| kept.clone())
}

fn is_null_branch(branch: &Value) -> bool {
    let Value::Object(map) = branch else {
        return false;
    };
    matches!(map.get("type"), Some(Value::String(kind)) if kind == "null")
}

/// Les mots-cles freres du slot (`description`, `title`, …) ecrasent ceux de
/// la branche promue — exactement ce que fait deja `inline_slot`.
fn promote_branch(map: &mut Map<String, Value>, keyword: &str, mut promoted: Map<String, Value>) {
    for (key, value) in map.iter() {
        if key != keyword {
            promoted.insert(key.clone(), value.clone());
        }
    }
    *map = promoted;
}

/// (c) Une union dont CHAQUE branche est un `const` chaine devient un
/// `enum` unique. C'est la forme que `schemars` donne a une enumeration
/// unitaire, et celle qu'un harnais aplatit en `{}` alors qu'elle enumere
/// pourtant ses propres valeurs.
fn collapse_const_union(map: &mut Map<String, Value>) {
    for keyword in ["oneOf", "anyOf"] {
        let Some(branches) = const_branches(map, keyword) else {
            continue;
        };
        let description = folded_description(map.get("description"), &branches);
        let values: Vec<Value> = branches.into_iter().map(|(value, _)| value).collect();
        map.remove(keyword);
        map.insert("type".to_owned(), Value::String("string".to_owned()));
        map.insert("enum".to_owned(), Value::Array(values));
        if let Some(text) = description {
            map.insert("description".to_owned(), Value::String(text));
        }
        return;
    }
}

/// Les `(valeur, description)` d'une union de `const` chaines, ou `None` des
/// qu'une branche n'est pas de cette forme.
fn const_branches(map: &Map<String, Value>, keyword: &str) -> Option<Vec<(Value, Option<String>)>> {
    let Value::Array(branches) = map.get(keyword)? else {
        return None;
    };
    if branches.is_empty() {
        return None;
    }
    let mut collected = Vec::with_capacity(branches.len());
    for branch in branches {
        let Some(Value::String(literal)) = branch.get("const") else {
            return None;
        };
        collected.push((
            Value::String(literal.clone()),
            text_keyword(branch, "description"),
        ));
    }
    Some(collected)
}

fn text_keyword(node: &Value, keyword: &str) -> Option<String> {
    match node.get(keyword) {
        Some(Value::String(text)) => Some(text.clone()),
        _ => None,
    }
}

/// La description du slot, suivie d'une ligne par branche qui en portait une
/// — sans quoi l'effondrement perdrait ce que chaque valeur signifie.
fn folded_description(own: Option<&Value>, branches: &[(Value, Option<String>)]) -> Option<String> {
    let mut lines: Vec<String> = branches
        .iter()
        .filter_map(|(value, text)| text.as_ref().map(|text| format!("- {value}: {text}")))
        .collect();
    let own = match own {
        Some(Value::String(text)) => Some(text.clone()),
        _ => None,
    };
    if lines.is_empty() {
        return own;
    }
    if let Some(text) = own {
        lines.insert(0, text);
    }
    Some(lines.join("\n"))
}

/// (b) `"type": ["X", "null"]` → `"X"`.
fn collapse_nullable_type_list(map: &mut Map<String, Value>) {
    if let Some(kept) = nullable_type_pair(map) {
        map.insert("type".to_owned(), Value::String(kept));
    }
}

fn nullable_type_pair(map: &Map<String, Value>) -> Option<String> {
    let Value::Array(kinds) = map.get("type")? else {
        return None;
    };
    let named: Vec<&str> = kinds.iter().filter_map(Value::as_str).collect();
    if named.len() != 2 || !named.contains(&"null") {
        return None;
    }
    named
        .iter()
        .find(|kind| **kind != "null")
        .map(|kind| (*kind).to_owned())
}

/// (d) Un `"default": null` sur un slot devenu non-nullable n'annonce plus
/// rien : il contredit le type qui vient d'etre pose.
fn drop_inert_null_default(map: &mut Map<String, Value>) {
    let scalar = matches!(map.get("type"), Some(Value::String(kind)) if kind != "null");
    if scalar && map.get("default") == Some(&Value::Null) {
        map.remove("default");
    }
}
