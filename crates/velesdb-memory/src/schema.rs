//! JSON Schema post-processing shared by the domain model and the MCP DTOs.
//!
//! `schemars` annotates Rust integer types with a `format` keyword (`"uint64"`
//! for `u64`, `"uint"` for `usize`, …). Those values are not standard JSON
//! Schema formats, so strict MCP clients log `unknown format "uint64" ignored`
//! for every integer field. The `type: integer` (plus the `minimum: 0` schemars
//! already emits for unsigned types) carries the constraint on its own, so the
//! non-standard `format` is pure noise — this transform strips it.

use schemars::Schema;
use serde_json::{Map, Value};

/// A `schemars` container transform that recursively removes Rust integer
/// `format` keywords from a generated schema. Apply with
/// `#[schemars(transform = crate::schema::strip_int_formats)]`.
pub(crate) fn strip_int_formats(schema: &mut Schema) {
    if let Some(object) = schema.as_object_mut() {
        strip_in_map(object);
    }
}

fn strip_in_map(map: &mut Map<String, Value>) {
    let drop_format = matches!(map.get("format"), Some(Value::String(f)) if is_rust_int_format(f));
    if drop_format {
        map.remove("format");
    }
    for value in map.values_mut() {
        strip_in_value(value);
    }
}

fn strip_in_value(value: &mut Value) {
    match value {
        Value::Object(map) => strip_in_map(map),
        Value::Array(items) => items.iter_mut().for_each(strip_in_value),
        _ => {}
    }
}

/// Recursively widen every property named in `keys` (resolving the `items`
/// of array-typed ones) from `integer` to `["integer", "string"]`, across
/// the whole schema tree — `$defs` included.
///
/// The advertised-schema counterpart of the `context::wire` id contract:
/// under `CompilePolicy::ids_as_strings` a response id field crosses as a
/// decimal string, and `fragments[].id` accepts one on input — and the
/// official MCP SDKs validate `structuredContent` against the advertised
/// `outputSchema` (spec 2025-06-18), so a schema typing those fields
/// `integer` only would make every opted-in response fail validation for
/// exactly the clients the option exists for. Same shape of tree walk as
/// [`strip_int_formats`], but keyed: only the named properties widen.
///
/// `mcp`-gated: the advertised tool schemas are its only consumer.
#[cfg(feature = "mcp")]
pub(crate) fn widen_id_properties(map: &mut Map<String, Value>, keys: &[&str]) {
    if let Some(Value::Object(properties)) = map.get_mut("properties") {
        for (name, subschema) in properties.iter_mut() {
            if keys.contains(&name.as_str()) {
                widen_id_schema(subschema);
            }
        }
    }
    for value in map.values_mut() {
        widen_in_value(value, keys);
    }
}

#[cfg(feature = "mcp")]
fn widen_in_value(value: &mut Value, keys: &[&str]) {
    match value {
        Value::Object(map) => widen_id_properties(map, keys),
        Value::Array(items) => items.iter_mut().for_each(|item| widen_in_value(item, keys)),
        _ => {}
    }
}

/// Widen one id property's schema: `integer` → `["integer", "string"]`
/// (keeping any `null` of an optional field), recursing into `items` for an
/// array of ids. `minimum: 0` may stay — JSON Schema numeric keywords apply
/// to numbers only, so the string form is unaffected.
#[cfg(feature = "mcp")]
fn widen_id_schema(schema: &mut Value) {
    let Value::Object(map) = schema else {
        return;
    };
    match map.get("type").cloned() {
        Some(Value::String(kind)) if kind == "integer" => {
            map.insert(
                "type".to_owned(),
                Value::Array(vec![
                    Value::String("integer".to_owned()),
                    Value::String("string".to_owned()),
                ]),
            );
        }
        Some(Value::String(kind)) if kind == "array" => {
            if let Some(items) = map.get_mut("items") {
                widen_id_schema(items);
            }
        }
        Some(Value::Array(mut kinds)) => {
            let has_integer = kinds.iter().any(|kind| kind == "integer");
            let has_string = kinds.iter().any(|kind| kind == "string");
            if has_integer && !has_string {
                let after = kinds
                    .iter()
                    .position(|kind| kind == "integer")
                    .map_or(kinds.len(), |position| position + 1);
                kinds.insert(after, Value::String("string".to_owned()));
                map.insert("type".to_owned(), Value::Array(kinds));
            }
        }
        _ => {}
    }
}

/// Mots-cles numeriques qu'un slot annonce `string` ne contraint plus.
/// JSON Schema applique `minimum`/`maximum`/`multipleOf` aux nombres
/// uniquement : les laisser sur un slot devenu `string` n'interdit rien et
/// suggere au lecteur un type que le slot n'a plus.
#[cfg(feature = "mcp")]
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
#[cfg(feature = "mcp")]
const UNION_KEYWORDS: [&str; 4] = ["anyOf", "oneOf", "allOf", "$ref"];

/// Le pendant ENTREE de [`widen_id_properties`] : chaque propriete nommee
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
/// (cf. [`crate::mcp::dto`]) et, pour le couple
/// `save_working_context`/`load_working_context`, de la reponse emise
/// directement en chaines. Le test
/// `every_lossy_id_in_a_result_offers_its_exact_string_twin`
/// (`tests/mcp_schema_bdd.rs`) refuse la moitie sans l'autre.
#[cfg(feature = "mcp")]
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

#[cfg(feature = "mcp")]
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
#[cfg(feature = "mcp")]
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
#[cfg(feature = "mcp")]
fn declares_array(map: &Map<String, Value>) -> bool {
    match map.get("type") {
        Some(Value::String(kind)) => kind == "array",
        Some(Value::Array(kinds)) => kinds.iter().any(|kind| kind == "array"),
        _ => false,
    }
}

/// Cap on how deep [`inline_ref_only_properties`] chases nested `$ref`s.
/// The deepest real input schema needs 3 (`working` → `items` of
/// `ContextFact` → its `source`); 8 leaves headroom while keeping a
/// mutually-recursive type from expanding without bound even if the
/// [`InlineChain`] guard were ever bypassed.
#[cfg(feature = "mcp")]
const MAX_INLINE_DEPTH: usize = 8;

/// The `$defs` names currently being inlined, as a PATH stack (pushed on
/// descent, popped on the way out) — not a global visited-set: a global set
/// would leave the second and third `Vec<ContextFact>` of a
/// [`WorkingContext`](crate::context::WorkingContext) un-inlined just
/// because a sibling got there first. A name already on the stack means a
/// reference cycle, and its `$ref` is left intact.
#[cfg(feature = "mcp")]
type InlineChain = Vec<String>;

/// Inline every schema slot that is a bare `$ref` (or a single-element
/// `allOf` wrapping one) into the referenced `$defs` entry, RECURSIVELY and
/// through `items`, so each slot carries a DIRECT `type` keyword.
///
/// Real MCP client harnesses (observed 2026-07-24 with Claude Code) degrade
/// a `$ref`-only parameter to "untyped" and then serialize the argument as a
/// JSON-encoded string — `save_working_context`'s `working` object arrived
/// as `"{\"goal\": ...}"` and failed with `invalid type: string, expected
/// struct WorkingContext`. Same wire-contract class as the #1468 float-lossy
/// id fix: the advertised schema must be harness-proof, not merely
/// spec-correct.
///
/// Chasing only ONE level (the 2026-07-24 shape) left the identical hole one
/// step deeper, and it cost four round trips of deserialization errors on
/// 2026-07-26: `working.decisions` advertised `items: {"$ref":
/// "#/$defs/ContextDecisionRef"}`, so a `$defs`-blind harness saw "array of
/// anything" and never learned that `rule_id` (or `SourceReference`'s
/// `handle`) is required. Hence the same tree walk as
/// [`widen_id_properties`] — `properties`, `items`, then a generic descent —
/// bounded by [`MAX_INLINE_DEPTH`] and an [`InlineChain`] cycle guard.
///
/// Sibling keywords on the slot (e.g. `description`) override the inlined
/// definition's; a slot that already exposes a `type` is not inlined, but is
/// still descended into — that is exactly the `{"type": "array", "items":
/// {"$ref": …}}` case. `$defs` is left in place and is NOT expanded on the
/// spot: spec-correct clients still resolve through it, it stays a
/// resolvable target for any `$ref` a bound leaves behind, and expanding it
/// would only inflate the advertised schema.
///
/// `mcp`-gated: the advertised tool schemas are its only consumer.
#[cfg(feature = "mcp")]
pub(crate) fn inline_ref_only_properties(map: &mut Map<String, Value>) {
    let Some(Value::Object(defs)) = map.get("$defs").cloned() else {
        return;
    };
    let mut chain = InlineChain::new();
    inline_in_map(map, &defs, &mut chain, 0);
    prune_unreferenced_defs(map);
}

/// Drop every `$defs` entry no `$ref` can still reach.
///
/// Inlining copies a definition to each site that referenced it, so its
/// `$defs` entry usually becomes dead weight — measured at **55 % of the
/// published schema bytes** (83 KB of 148 KB across the 18 tools) once both
/// the input and output sides were inlined. Keeping it costs every client
/// that reads `tools/list`, and the whole point of inlining was that a
/// `$defs`-blind client never looks there.
///
/// Only *unreachable* entries go: 65 `$ref`s legitimately survive (a bound a
/// cycle guard stopped, an `anyOf` arm), and a definition still reachable —
/// directly or through another definition — is kept. The fixpoint below is
/// what makes that transitive: dropping one entry can orphan another.
#[cfg(feature = "mcp")]
fn prune_unreferenced_defs(map: &mut Map<String, Value>) {
    let Some(Value::Object(defs)) = map.get("$defs") else {
        return;
    };
    let names: Vec<String> = defs.keys().cloned().collect();
    let defs = defs.clone();

    let mut live: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    // Seed with everything referenced outside `$defs` itself.
    let mut outside = map.clone();
    outside.remove("$defs");
    collect_refs(&Value::Object(outside), &mut live);

    // Fixpoint: a live definition's own `$ref`s keep their targets alive.
    loop {
        let mut grown = false;
        for name in &names {
            if !live.contains(name) {
                continue;
            }
            if let Some(def) = defs.get(name) {
                let before = live.len();
                collect_refs(def, &mut live);
                grown |= live.len() != before;
            }
        }
        if !grown {
            break;
        }
    }

    if let Some(Value::Object(defs)) = map.get_mut("$defs") {
        defs.retain(|name, _| live.contains(name));
        if defs.is_empty() {
            map.remove("$defs");
        }
    }
}

/// Every `#/$defs/<name>` target reachable from `value`.
#[cfg(feature = "mcp")]
fn collect_refs(value: &Value, out: &mut std::collections::BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            if let Some(Value::String(target)) = map.get("$ref") {
                if let Some(name) = target.strip_prefix("#/$defs/") {
                    out.insert(name.to_owned());
                }
            }
            for sub in map.values() {
                collect_refs(sub, out);
            }
        }
        Value::Array(entries) => {
            for sub in entries {
                collect_refs(sub, out);
            }
        }
        _ => {}
    }
}

/// One schema node: inline its `properties` and `items` slots, then descend
/// generically into every other keyword — `$defs` excepted (see
/// [`inline_ref_only_properties`]).
#[cfg(feature = "mcp")]
fn inline_in_map(
    map: &mut Map<String, Value>,
    defs: &Map<String, Value>,
    chain: &mut InlineChain,
    depth: usize,
) {
    if depth >= MAX_INLINE_DEPTH {
        return;
    }
    inline_property_slots(map, defs, chain, depth);
    inline_item_slots(map, defs, chain, depth);
    inline_union_branches(map, defs, chain, depth);
    inline_additional_property_slot(map, defs, chain, depth);
    inline_remaining_keywords(map, defs, chain, depth);
}

/// The keywords `inline_in_map` walks as argument paths, each with its own
/// pass — listed once here so the generic descent cannot walk them twice.
#[cfg(feature = "mcp")]
const SLOT_KEYWORDS: [&str; 7] = [
    "$defs",
    "properties",
    "items",
    "anyOf",
    "oneOf",
    "prefixItems",
    "additionalProperties",
];

#[cfg(feature = "mcp")]
fn inline_property_slots(
    map: &mut Map<String, Value>,
    defs: &Map<String, Value>,
    chain: &mut InlineChain,
    depth: usize,
) {
    if let Some(Value::Object(properties)) = map.get_mut("properties") {
        for slot in properties.values_mut() {
            inline_slot(slot, defs, chain, depth);
        }
    }
}

#[cfg(feature = "mcp")]
fn inline_item_slots(
    map: &mut Map<String, Value>,
    defs: &Map<String, Value>,
    chain: &mut InlineChain,
    depth: usize,
) {
    match map.get_mut("items") {
        // Tuple form: `items` is an array of per-position schemas.
        Some(Value::Array(entries)) => {
            for entry in entries {
                inline_slot(entry, defs, chain, depth);
            }
        }
        Some(single) => inline_slot(single, defs, chain, depth),
        None => {}
    }
}

/// Union branches are argument paths too.
///
/// `Option<T>` is the common case: schemars renders it
/// `anyOf: [{"$ref": …}, {"type": "null"}]`, and a `$defs`-blind harness that
/// cannot resolve the first branch degrades the WHOLE slot to "anything" —
/// which is how `source`, `goal` and `memory_id` reached callers as a bare
/// `{}` (2026-07-28). Treating each branch as a slot is what makes an
/// optional field as self-describing as a required one.
///
/// `allOf` stays out on purpose: [`ref_only_target`] already reads its
/// single-element form as a wrapper around the slot itself, and inlining it
/// here as a branch would fight that.
#[cfg(feature = "mcp")]
fn inline_union_branches(
    map: &mut Map<String, Value>,
    defs: &Map<String, Value>,
    chain: &mut InlineChain,
    depth: usize,
) {
    for keyword in ["anyOf", "oneOf", "prefixItems"] {
        if let Some(Value::Array(branches)) = map.get_mut(keyword) {
            for branch in branches {
                inline_slot(branch, defs, chain, depth);
            }
        }
    }
}

/// The value schema of an open map (`BTreeMap<String, T>`) is an argument
/// path like any other.
///
/// Found on 2026-07-29 by the post-condition of `combined_router`, not by a
/// test: the generic descent below walked THROUGH `additionalProperties`
/// (inlining what was inside it) but never inlined the node ITSELF, so
/// `compile_context`'s `policy.pricing.models` shipped
/// `additionalProperties: {"$ref": "#/$defs/ModelPricing"}` — the exact
/// unresolvable-`$ref` slot this whole pass exists to remove, hiding in the
/// one keyword no pass claimed. `additionalProperties` is often the boolean
/// `true`; [`inline_slot`] ignores a non-object silently.
#[cfg(feature = "mcp")]
fn inline_additional_property_slot(
    map: &mut Map<String, Value>,
    defs: &Map<String, Value>,
    chain: &mut InlineChain,
    depth: usize,
) {
    if let Some(extra @ Value::Object(_)) = map.get_mut("additionalProperties") {
        inline_slot(extra, defs, chain, depth);
    }
}

/// Generic descent for everything the dedicated passes did not claim.
#[cfg(feature = "mcp")]
fn inline_remaining_keywords(
    map: &mut Map<String, Value>,
    defs: &Map<String, Value>,
    chain: &mut InlineChain,
    depth: usize,
) {
    for (key, value) in map.iter_mut() {
        if !SLOT_KEYWORDS.contains(&key.as_str()) {
            inline_in_value(value, defs, chain, depth);
        }
    }
}

#[cfg(feature = "mcp")]
fn inline_in_value(
    value: &mut Value,
    defs: &Map<String, Value>,
    chain: &mut InlineChain,
    depth: usize,
) {
    match value {
        Value::Object(map) => inline_in_map(map, defs, chain, depth),
        Value::Array(items) => items
            .iter_mut()
            .for_each(|item| inline_in_value(item, defs, chain, depth)),
        _ => {}
    }
}

/// Inline one slot when it is `$ref`-only and its target is not already
/// being inlined higher up the path, then recurse into the resulting node
/// either way.
#[cfg(feature = "mcp")]
fn inline_slot(slot: &mut Value, defs: &Map<String, Value>, chain: &mut InlineChain, depth: usize) {
    let Value::Object(prop) = slot else {
        return;
    };
    if !prop.contains_key("type") {
        if let Some(name) = ref_only_target(prop) {
            if !chain.contains(&name) {
                if let Some(Value::Object(definition)) = defs.get(&name) {
                    let mut merged = definition.clone();
                    for (key, value) in prop.iter() {
                        if key != "$ref" && key != "allOf" {
                            merged.insert(key.clone(), value.clone());
                        }
                    }
                    *prop = merged;
                    chain.push(name);
                    inline_in_map(prop, defs, chain, depth + 1);
                    chain.pop();
                    return;
                }
            }
        }
    }
    inline_in_map(prop, defs, chain, depth + 1);
}

/// Resolves the `#/$defs/<Name>` target of a `$ref`-only property schema:
/// either a direct `$ref` keyword or a single-element `allOf` wrapping one.
#[cfg(feature = "mcp")]
fn ref_only_target(prop: &Map<String, Value>) -> Option<String> {
    let reference = match (prop.get("$ref"), prop.get("allOf")) {
        (Some(Value::String(r)), _) => r.clone(),
        (None, Some(Value::Array(items))) if items.len() == 1 => match &items[0] {
            Value::Object(inner) => match inner.get("$ref") {
                Some(Value::String(r)) => r.clone(),
                _ => return None,
            },
            _ => return None,
        },
        _ => return None,
    };
    reference.strip_prefix("#/$defs/").map(str::to_owned)
}

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
///   cycle d'[`inline_ref_only_properties`] deviendrait sinon un slot
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
/// [`inline_ref_only_properties`] — une branche doit deja etre inlinee pour
/// porter un type promouvable.
#[cfg(feature = "mcp")]
pub(crate) fn scalarize_slot_types(schema: &mut WireInputSchema) {
    scalarize_in_map(&mut schema.0);
}

/// Les mots-cles que [`scalarize_in_map`] parcourt comme chemins d'argument,
/// chacun avec sa propre passe — listes ici une seule fois pour que la
/// descente generique ne les reparcoure pas.
#[cfg(feature = "mcp")]
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
#[cfg(feature = "mcp")]
const DATA_KEYWORDS: [&str; 4] = ["enum", "const", "default", "examples"];

#[cfg(feature = "mcp")]
fn scalarize_in_map(map: &mut Map<String, Value>) {
    scalarize_property_slots(map);
    scalarize_item_slots(map);
    scalarize_union_branches(map);
    scalarize_additional_properties(map);
    scalarize_remaining_keywords(map);
}

#[cfg(feature = "mcp")]
fn scalarize_property_slots(map: &mut Map<String, Value>) {
    if let Some(Value::Object(properties)) = map.get_mut("properties") {
        for slot in properties.values_mut() {
            scalarize_slot(slot);
        }
    }
}

#[cfg(feature = "mcp")]
fn scalarize_item_slots(map: &mut Map<String, Value>) {
    match map.get_mut("items") {
        // Forme tuple : `items` est un tableau de schemas par position.
        Some(Value::Array(entries)) => entries.iter_mut().for_each(scalarize_slot),
        Some(single) => scalarize_slot(single),
        None => {}
    }
}

#[cfg(feature = "mcp")]
fn scalarize_union_branches(map: &mut Map<String, Value>) {
    for keyword in ["anyOf", "oneOf", "prefixItems"] {
        if let Some(Value::Array(branches)) = map.get_mut(keyword) {
            branches.iter_mut().for_each(scalarize_slot);
        }
    }
}

#[cfg(feature = "mcp")]
fn scalarize_additional_properties(map: &mut Map<String, Value>) {
    if let Some(extra @ Value::Object(_)) = map.get_mut("additionalProperties") {
        scalarize_slot(extra);
    }
}

#[cfg(feature = "mcp")]
fn scalarize_remaining_keywords(map: &mut Map<String, Value>) {
    for (key, value) in map.iter_mut() {
        let name = key.as_str();
        if !SCALARIZE_SLOT_KEYWORDS.contains(&name) && !DATA_KEYWORDS.contains(&name) {
            scalarize_in_value(value);
        }
    }
}

#[cfg(feature = "mcp")]
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
#[cfg(feature = "mcp")]
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
#[cfg(feature = "mcp")]
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
#[cfg(feature = "mcp")]
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

#[cfg(feature = "mcp")]
fn is_null_branch(branch: &Value) -> bool {
    let Value::Object(map) = branch else {
        return false;
    };
    matches!(map.get("type"), Some(Value::String(kind)) if kind == "null")
}

/// Les mots-cles freres du slot (`description`, `title`, …) ecrasent ceux de
/// la branche promue — exactement ce que fait deja `inline_slot`.
#[cfg(feature = "mcp")]
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
#[cfg(feature = "mcp")]
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
#[cfg(feature = "mcp")]
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

#[cfg(feature = "mcp")]
fn text_keyword(node: &Value, keyword: &str) -> Option<String> {
    match node.get(keyword) {
        Some(Value::String(text)) => Some(text.clone()),
        _ => None,
    }
}

/// La description du slot, suivie d'une ligne par branche qui en portait une
/// — sans quoi l'effondrement perdrait ce que chaque valeur signifie.
#[cfg(feature = "mcp")]
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
#[cfg(feature = "mcp")]
fn collapse_nullable_type_list(map: &mut Map<String, Value>) {
    if let Some(kept) = nullable_type_pair(map) {
        map.insert("type".to_owned(), Value::String(kept));
    }
}

#[cfg(feature = "mcp")]
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
#[cfg(feature = "mcp")]
fn drop_inert_null_default(map: &mut Map<String, Value>) {
    let scalar = matches!(map.get("type"), Some(Value::String(kind)) if kind != "null");
    if scalar && map.get("default") == Some(&Value::Null) {
        map.remove("default");
    }
}

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
#[cfg(feature = "mcp")]
pub(crate) fn untyped_input_slots(map: &Map<String, Value>) -> Vec<String> {
    let mut found = Vec::new();
    collect_untyped(&Value::Object(map.clone()), "$", &mut found);
    found
}

#[cfg(feature = "mcp")]
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

#[cfg(feature = "mcp")]
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

#[cfg(feature = "mcp")]
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

#[cfg(feature = "mcp")]
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

fn is_rust_int_format(format: &str) -> bool {
    matches!(
        format,
        "uint"
            | "uint8"
            | "uint16"
            | "uint32"
            | "uint64"
            | "uint128"
            | "int"
            | "int8"
            | "int16"
            | "int32"
            | "int64"
            | "int128"
    )
}

#[cfg(test)]
#[path = "schema_tests.rs"]
mod tests;

/// Les champs d'id qui traversent le fil sous forme entiere OU chaine
/// decimale.
///
/// Seule declaration (#1685) : [`crate::context::wire::ID_KEYS`] la
/// reexporte telle quelle plutot que de la redeclarer. Vit ici et non dans
/// `context::wire` parce que `schema.rs` n'est jamais gate par une feature
/// (voir `mod schema;` dans `lib.rs`), alors que le module `context` entier
/// l'est sur `context` — et les outils de `mcp.rs` en ont besoin meme quand
/// `context` est absente, ce qui est le cas de `--features http`. Gatee sur
/// `any(mcp, context)` et non laissee inconditionnelle : un build
/// `--features persistence` seule (verifie en isolation par la CI, voir
/// `ci.yml`) n'a besoin d'aucune des deux, et `-D warnings` transformerait
/// la constante alors inutilisee en echec de build.
#[cfg(any(feature = "mcp", feature = "context"))]
pub const WIRE_ID_KEYS: &[&str] = &["fragment_id", "content_hash", "memory_id", "fragment_ids"];

// --- Les deux surfaces du fil, separees par le TYPE -------------------------
//
// Entree et sortie partageaient jusqu'ici la meme representation (un
// `Map<String, Value>` nu), donc rien n'empechait d'appliquer a une sortie
// une passe reservee a l'entree. Ce n'est pas une nuance de style :
// [`scalarize_slot_types`] effondre `anyOf: [T, null]` en `T`, et les SDK MCP
// valident `structuredContent` contre l'`outputSchema` annonce (spec
// 2025-06-18) — retirer la branche `null` d'une sortie ferait donc echouer,
// chez le client, les reponses legitimes du serveur lui-meme.
//
// Les deux surfaces sont donc deux types distincts, sans conversion de l'un
// vers l'autre.
//
// Portee exacte de la barriere, parce qu'une garantie surestimee est pire
// qu'une garantie absente. Elle tient sur ce qui SORT de ce module : hors
// d'ici, aucune fonction n'accepte une carte nue pour lui appliquer un
// durcissement d'ENTREE — les deux constructeurs de `WireInputSchema` sont
// prives au module, et le seul point de passage qui reprend un schema deja
// publie ([`reharden_tool_input`]) prend l'outil entier et ne touche que son
// `input_schema` : il n'y a pas de parametre par lequel passer une sortie.
// (Il a existe, sous la forme d'un `pub(crate) fn(&JsonObject)`, et
// `rmcp::model::Tool` type ses deux schemas avec le MEME
// `Arc<JsonObject>` : rien n'aurait signale l'erreur.)
//
// Elle ne tient PAS a l'interieur de `schema.rs` : `scalarize_in_map` est
// typee sur la carte brute et `WireOutputSchema::harden` vit dans le meme
// module. Ce que le type empeche, c'est l'accident a distance ; ici, seule
// la lecture protege — d'ou le garde executable
// `output_hardening_keeps_a_nullable_union_intact` dans `schema_tests.rs`,
// qui echoue si la scalarisation atteint un jour la sortie.

/// Un schema d'ENTREE en construction : ce que le client LIT pour fabriquer
/// ses arguments. Il doit survivre au harnais qui le rend, pas seulement etre
/// valide au sens JSON Schema.
#[cfg(feature = "mcp")]
pub(crate) struct WireInputSchema(Map<String, Value>);

/// Un schema de SORTIE en construction : ce que le SDK client valide contre
/// la reponse. Il doit rester fidele a ce que le serveur emet reellement,
/// unions nullables comprises.
#[cfg(feature = "mcp")]
pub(crate) struct WireOutputSchema(Map<String, Value>);

#[cfg(feature = "mcp")]
impl WireInputSchema {
    /// Le schema derive par `schemars` pour les parametres de l'outil, avant
    /// tout durcissement.
    fn derived<T: schemars::JsonSchema + std::any::Any>() -> Self {
        let schema = rmcp::handler::server::tool::schema_for_input::<
            rmcp::handler::server::wrapper::Parameters<T>,
        >()
        .unwrap_or_else(|e| {
            panic!(
                "Invalid input schema for {}: {e}",
                std::any::type_name::<T>()
            )
        });
        Self((*schema).clone())
    }

    /// Reprend un schema d'entree DEJA publie par une route, pour lui
    /// repasser le durcissement universel. Volontairement prive au module :
    /// c'est le seul constructeur qui accepte une carte venue de l'exterieur,
    /// et son unique appelant est [`rehardened_input_schema`], dont le nom
    /// dit la surface.
    fn adopt(map: Map<String, Value>) -> Self {
        Self(map)
    }

    /// L'ordre compte. `stringify_id_properties` d'abord, pour que les copies
    /// faites par l'inliner heritent du type deja pose ; l'inliner ensuite,
    /// pour que chaque branche porte un `type` direct ; la scalarisation en
    /// dernier, parce qu'elle ne promeut qu'une branche deja typee.
    fn harden(mut self, id_keys: &[&str]) -> Self {
        stringify_id_properties(&mut self.0, id_keys);
        inline_ref_only_properties(&mut self.0);
        scalarize_slot_types(&mut self);
        self
    }

    fn publish(self) -> std::sync::Arc<rmcp::model::JsonObject> {
        std::sync::Arc::new(self.0)
    }
}

#[cfg(feature = "mcp")]
impl WireOutputSchema {
    fn derived<T: schemars::JsonSchema + std::any::Any>() -> Self {
        let schema = rmcp::handler::server::tool::schema_for_output::<T>().unwrap_or_else(|e| {
            panic!(
                "Invalid output schema for {}: {e}",
                std::any::type_name::<T>()
            )
        });
        Self((*schema).clone())
    }

    /// Elargissement des ids puis inlining. Pas de scalarisation : voir le
    /// commentaire de section.
    fn harden(mut self) -> Self {
        widen_id_properties(&mut self.0, WIRE_ID_KEYS);
        inline_ref_only_properties(&mut self.0);
        self
    }

    fn publish(self) -> std::sync::Arc<rmcp::model::JsonObject> {
        std::sync::Arc::new(self.0)
    }
}

/// Le schema d'ENTREE annonce d'un outil.
///
/// `id_keys` nomme les proprietes que CET outil accepte en chaine decimale
/// (cf. [`stringify_id_properties`]) — la tolerance est per-outil, pas
/// globale : `explain_compilation.fragment_id` est un `u64` STRICT et ne doit
/// jamais etre annonce `string`.
///
/// Constructeur unique des deux qui existaient : celui de `mcp.rs` (jeu de
/// cles par outil) et celui de `mcp/context_tools.rs` (cle `"id"` figee).
#[cfg(feature = "mcp")]
pub(crate) fn wire_safe_input_schema<T: schemars::JsonSchema + std::any::Any>(
    id_keys: &[&str],
) -> std::sync::Arc<rmcp::model::JsonObject> {
    WireInputSchema::derived::<T>().harden(id_keys).publish()
}

/// Repasse le durcissement universel (inlining + scalarisation) sur le
/// schema d'ENTREE d'un outil deja construit.
///
/// C'est le point de passage unique de `McpServer::combined_router` : une
/// route qui n'a declare aucun `input_schema` recoit celui derive par rmcp,
/// que rien ne post-traitait. Sans cles d'id — la tolerance d'un id est une
/// connaissance de l'outil, elle reste dans son attribut.
///
/// Prend l'outil ENTIER, et non son `input_schema` : `rmcp::model::Tool`
/// type `input_schema` et `output_schema` avec le meme `Arc<JsonObject>`,
/// donc une signature `fn(&JsonObject)` aurait accepte un schema de sortie
/// sans le moindre diagnostic — et scalariser une sortie fait echouer, chez
/// le client, la validation des reponses legitimes du serveur. Ici, la
/// sortie n'est pas atteignable : elle n'est pas un parametre.
#[cfg(feature = "mcp")]
pub(crate) fn reharden_tool_input(tool: &mut rmcp::model::Tool) {
    tool.input_schema = WireInputSchema::adopt((*tool.input_schema).clone())
        .harden(&[])
        .publish();
}

/// Le schema de sortie ANNONCE d'un outil : ids elargis, puis `$ref` inlines
/// et `$defs` inatteignables elagues.
///
/// Vit ici plutot que dans `mcp/context_tools.rs` parce que les outils de
/// `mcp.rs` l'appellent aussi : le laisser dans un module gate sur `context`
/// cassait `--features http`, qui active `mcp` sans `context`.
#[cfg(feature = "mcp")]
pub(crate) fn wire_safe_output_schema<T: schemars::JsonSchema + std::any::Any>(
) -> std::sync::Arc<rmcp::model::JsonObject> {
    WireOutputSchema::derived::<T>().harden().publish()
}
