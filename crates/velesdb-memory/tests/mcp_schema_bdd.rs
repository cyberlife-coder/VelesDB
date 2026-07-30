//! The advertised MCP **input** schemas must be usable by a client that does
//! NOT dereference `$defs` — every array parameter has to carry a DIRECT
//! `type` on its `items`, not a bare `$ref`.
//!
//! Field report (2026-07-26): calling `save_working_context` for real took
//! four round trips of deserialization errors, because `working`'s nested
//! arrays (`verified_facts`, `decisions`, `exact_evidence`) advertised
//! `items: {"$ref": "#/$defs/…"}`. Harnesses that do not resolve `$defs`
//! degrade that to "untyped array of anything", so `ContextFact`,
//! `ContextDecisionRef` and `SourceReference` — and their REQUIRED fields
//! (`rule_id`, `handle`) — were invisible to the caller. Same failure class
//! as the `$ref`-only `working` parameter fixed on 2026-07-24, one level
//! deeper.
//!
//! The schemas are read exactly as published: an in-memory MCP client over
//! `tokio::io::duplex` (the idiom of rmcp's own upstream tests) drives the
//! real `McpServer`, so these tests see the same schema bytes a Claude Code /
//! Windsurf harness receives.
//!
//! Output schemas are covered too, since the same defect turned out to be
//! there and to bite harder: the official MCP SDKs validate a tool's
//! `structuredContent` against its advertised `outputSchema`, so an
//! unresolvable `$ref` is a RESULT the client may reject — worse than an
//! unparseable argument, where the server at least got to answer.

#![cfg(all(feature = "mcp", feature = "context", feature = "persistence"))]

use std::collections::BTreeSet;

use rmcp::model::CallToolRequestParams;
use rmcp::service::RunningService;
use rmcp::{RoleClient, ServiceExt};
use serde_json::{json, Map, Value};
use tempfile::TempDir;
use velesdb_memory::mcp::McpServer;
use velesdb_memory::{DynEmbedder, HashEmbedder, MemoryService, DEFAULT_DIMENSION};

/// A `u64` id well past 2^53 — the range every real `fragment_id` lives in
/// (they are FNV-1a 64 content hashes), and exactly the range a float-lossy
/// JSON client must relay as a decimal string.
const BIG_FRAGMENT_ID: &str = "12297829382473034410";

/// Boot the real `McpServer` over an in-memory duplex pipe and complete the
/// MCP handshake. The `TempDir` is returned so the caller keeps the store
/// alive for the test's duration.
async fn connected() -> (TempDir, RunningService<RoleClient, ()>) {
    let store_dir = tempfile::tempdir().expect("create scratch store dir");
    let embedder: DynEmbedder = Box::new(HashEmbedder::new(DEFAULT_DIMENSION));
    let service =
        MemoryService::open(store_dir.path(), embedder).expect("open scratch memory store");
    let (server_side, client_side) = tokio::io::duplex(1 << 20);
    tokio::spawn(async move {
        if let Ok(running) = McpServer::new(service).serve(server_side).await {
            let _ = running.waiting().await;
        }
    });
    let client = ().serve(client_side).await.expect("MCP initialize handshake over duplex");
    (store_dir, client)
}

fn as_args(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        other => panic!("expected a JSON object, got {other:?}"),
    }
}

/// Collect every `items` schema reachable from a tool's input schema WITHOUT
/// resolving `$defs` — i.e. exactly what a non-dereferencing harness sees —
/// and report the ones that carry no direct `type` keyword.
///
/// `$defs` itself is deliberately not walked: it is the reference pool kept
/// for spec-correct clients, not an argument path. Any `$ref` still sitting
/// on a reachable `items` shows up here as "untyped", which is the point.
fn untyped_items(schema: &Value, surface: Surface) -> Vec<String> {
    let mut found = Vec::new();
    walk_schema(schema, "$", surface, &mut found);
    found
}

/// Which direction of the wire a schema governs. The two are NOT held to the
/// same rule, and conflating them is itself a defect this file has to prevent.
///
/// An **input** slot is read by a client harness that must decide what bytes
/// to send, and real harnesses flatten a union into `{}` — the same class of
/// degradation as the `$defs` blindness this file opens on, one construct
/// further. So an input slot gets exactly one scalar `type`.
///
/// An **output** slot is the opposite: the official MCP SDKs VALIDATE a tool's
/// `structuredContent` against it. A nullable result legitimately serializes
/// to `null`, so stripping the `null` branch there would make the server's own
/// answers fail validation. Outputs therefore keep the looser rule — announce
/// *some* type — and the strict one must never reach them.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Surface {
    Input,
    Output,
}

impl Surface {
    fn accepts(self, slot: &Value) -> bool {
        match self {
            Surface::Input => describes_its_own_type(slot),
            Surface::Output => announces_some_type(slot),
        }
    }
}

fn walk_schema(node: &Value, path: &str, surface: Surface, found: &mut Vec<String>) {
    let Value::Object(map) = node else {
        return;
    };
    if let Some(items) = map.get("items") {
        let items_path = format!("{path}.items");
        match items {
            Value::Array(entries) => {
                for (index, entry) in entries.iter().enumerate() {
                    check_items(entry, &format!("{items_path}[{index}]"), surface, found);
                }
            }
            other => check_items(other, &items_path, surface, found),
        }
    }
    if let Some(Value::Object(properties)) = map.get("properties") {
        for (name, sub) in properties {
            walk_schema(sub, &format!("{path}.{name}"), surface, found);
        }
    }
    for keyword in ["anyOf", "oneOf", "allOf", "prefixItems"] {
        if let Some(Value::Array(entries)) = map.get(keyword) {
            for (index, entry) in entries.iter().enumerate() {
                walk_schema(entry, &format!("{path}.{keyword}[{index}]"), surface, found);
            }
        }
    }
    if let Some(extra @ Value::Object(_)) = map.get("additionalProperties") {
        walk_schema(
            extra,
            &format!("{path}.additionalProperties"),
            surface,
            found,
        );
    }
}

/// One `items` slot: it must announce its type per its surface's rule, then it
/// is walked like any other schema node.
fn check_items(items: &Value, path: &str, surface: Surface, found: &mut Vec<String>) {
    if !surface.accepts(items) {
        found.push(format!("{path} = {items}"));
    }
    walk_schema(items, path, surface, found);
}

/// The OUTPUT rule: a slot answers "what comes back here?" with a direct
/// `type`, a union whose every branch does, or a closed set of literals.
/// Unions are fine — and necessary — on this side; see [`Surface`].
fn announces_some_type(slot: &Value) -> bool {
    let Value::Object(map) = slot else {
        return false;
    };
    if map.contains_key("type") || map.contains_key("enum") || map.contains_key("const") {
        return true;
    }
    for keyword in ["anyOf", "oneOf"] {
        if let Some(Value::Array(branches)) = map.get(keyword) {
            if !branches.is_empty() && branches.iter().all(announces_some_type) {
                return true;
            }
        }
    }
    false
}

/// Every reachable **property** slot, not just every `items` slot.
///
/// The `items` rule above was written after the 2026-07-26 field report and
/// it holds — but it is narrower than the defect it was meant to close.
/// On 2026-07-28 a real call was rejected again, this time on properties:
/// `save_working_context`'s `source`, `goal` and `memory_id` were advertised
/// as `{}` — the empty schema, which says "anything". The caller sent a
/// string, the server wanted a `SourceReference`, and the schema had said
/// nothing to prevent it. Third occurrence of one family, third shape.
///
/// So the invariant here is deliberately shape-agnostic: it does not look
/// for `$ref`, `allOf`, `anyOf` or any other construct the next regression
/// might use. It asks the only question that matters to a caller — *can I
/// tell what may go in this slot?* — and a slot answers yes when it carries
/// a direct `type`, or an `anyOf`/`oneOf` whose branches each do, or an
/// `enum`/`const` that enumerates its own values.
fn untyped_properties(schema: &Value) -> Vec<Finding> {
    let mut found = Vec::new();
    walk_properties(schema, "$", &mut found);
    found
}

/// One offending slot: its argument path, and what it actually advertises.
/// The path is kept apart from the rendering so an exemption can be keyed on
/// it exactly, instead of on a substring of a message.
struct Finding {
    path: String,
    slot: String,
}

fn walk_properties(node: &Value, path: &str, found: &mut Vec<Finding>) {
    let Value::Object(map) = node else {
        return;
    };
    if let Some(Value::Object(properties)) = map.get("properties") {
        for (name, slot) in properties {
            let slot_path = format!("{path}.{name}");
            if !describes_its_own_type(slot) {
                found.push(Finding {
                    path: slot_path.clone(),
                    slot: slot.to_string(),
                });
            }
            walk_properties(slot, &slot_path, found);
        }
    }
    if let Some(items) = map.get("items") {
        match items {
            Value::Array(entries) => {
                for (index, entry) in entries.iter().enumerate() {
                    walk_properties(entry, &format!("{path}.items[{index}]"), found);
                }
            }
            single => walk_properties(single, &format!("{path}.items"), found),
        }
    }
    for keyword in ["anyOf", "oneOf", "allOf", "prefixItems"] {
        if let Some(Value::Array(entries)) = map.get(keyword) {
            for (index, entry) in entries.iter().enumerate() {
                walk_properties(entry, &format!("{path}.{keyword}[{index}]"), found);
            }
        }
    }
    if let Some(extra @ Value::Object(_)) = map.get("additionalProperties") {
        walk_properties(extra, &format!("{path}.additionalProperties"), found);
    }
}

/// A slot that legitimately advertises more than one scalar type, with the
/// reason it may.
///
/// Same discipline as `EXEMPTIONS` in `binding_parity_bdd.rs`: silence is not
/// a decision, so every survivor of the rule above is written down WITH its
/// justification — and [`no_polymorphic_exemption_is_stale`] deletes the
/// entry the day it stops describing anything, because a stale exemption is
/// a hole in the guard that looks like a decision.
struct PolymorphicSlot {
    tool: &'static str,
    path: &'static str,
    reason: &'static str,
}

const POLYMORPHIC_SLOTS: &[PolymorphicSlot] = &[PolymorphicSlot {
    tool: "recall_where",
    path: "$.filters.items.value",
    reason: "the ColumnStore comparison is TYPE-STRICT with no coercion, so the JSON type sent \
             here is part of the query: 20260601 (number) never matches a fact stored as \
             \"20260601\" (string) — same value, no match, and no error. Collapsing this slot to \
             one type would make the schema state a restriction the server does not apply, on the \
             single field where sending the wrong type fails SILENTLY. Its own doc-comment \
             already spells the type out rather than leaving the empty schema serde_json::Value \
             would produce; a harness that flattens it back to {} leaves the caller exactly where \
             the untyped form would have.",
}];

fn polymorphic_exemption(tool: &str, path: &str) -> Option<&'static PolymorphicSlot> {
    POLYMORPHIC_SLOTS
        .iter()
        .find(|slot| slot.tool == tool && slot.path == path)
}

/// A slot is self-describing when a caller can name what goes in it WITHOUT
/// resolving anything and WITHOUT choosing between alternatives: exactly one
/// scalar `type`, or a closed set of literal values.
///
/// The 2026-07-29 tightening. The previous version of this function also
/// accepted `anyOf`/`oneOf` whose every branch was itself self-describing, and
/// a `type` holding an array of names. Both are valid JSON Schema, and both
/// are what real client harnesses destroy: the server emitted
/// `type: ["integer","string"]` on its id parameters and
/// `anyOf: [SourceReference, null]` on `…verified_facts[].source`, while the
/// harness that consumed them showed `{}` for both. So the invariant can no
/// longer be "valid JSON Schema" — it has to be "a shape that survives the
/// consumer". One scalar type, nothing to flatten.
///
/// Do not go looking for those two unions in
/// `docs/reference/mcp-tools.json`: that artifact is regenerated AFTER the
/// scalarization pass, so it records the state of arrival, never the state of
/// departure. What it still shows is the same union WHERE IT SURVIVES — the
/// output side, which this rule must never reach: `compile_context`'s
/// `decisions[].fragment_id` is still `["integer","string"]` there, and
/// `load_working_context`'s `working` still an `anyOf` with a `null` branch.
/// Nor can the artifact record what a HARNESS rendered: it is the server's
/// own emission, captured over raw JSON-RPC precisely so no client library
/// sits between the bytes and the assertion.
fn describes_its_own_type(slot: &Value) -> bool {
    let Value::Object(map) = slot else {
        return false;
    };
    if map.contains_key("enum") || map.contains_key("const") {
        return true;
    }
    matches!(map.get("type"), Some(Value::String(_)))
}

/// The 2026-07-28 regression: an advertised `{}` is a promise that anything
/// fits, and the server keeps none of it.
#[tokio::test]
async fn every_tool_input_schema_types_every_reachable_property() {
    let (_store, client) = connected().await;
    let tools = client.list_all_tools().await.expect("list tools");
    assert!(!tools.is_empty(), "the server advertises at least one tool");
    let mut offenders: BTreeSet<String> = BTreeSet::new();
    for tool in &tools {
        let schema = Value::Object((*tool.input_schema).clone());
        for finding in untyped_properties(&schema) {
            if polymorphic_exemption(&tool.name, &finding.path).is_some() {
                continue;
            }
            offenders.insert(format!(
                "{}: {} = {}",
                tool.name, finding.path, finding.slot
            ));
        }
    }
    assert!(
        offenders.is_empty(),
        "{} property slot(s) advertise no single scalar type — a caller reading the schema \
         cannot know what to send, and the server rejects what it guesses. Collapse the slot \
         in `crate::schema::scalarize_slot_types`, or declare it in `POLYMORPHIC_SLOTS` in \
         this file WITH the reason it must stay polymorphic: {offenders:#?}",
        offenders.len()
    );
    client.cancel().await.expect("close the MCP session");
}

/// An exemption that stopped describing a real slot must be deleted, not
/// left to rot — the twin of `no_exemption_is_stale` in
/// `binding_parity_bdd.rs`. Without this, `POLYMORPHIC_SLOTS` would quietly
/// become a blanket that covers paths nobody has published for months.
#[tokio::test]
async fn no_polymorphic_exemption_is_stale() {
    let (_store, client) = connected().await;
    let tools = client.list_all_tools().await.expect("list tools");
    let mut offending: BTreeSet<(String, String)> = BTreeSet::new();
    for tool in &tools {
        let schema = Value::Object((*tool.input_schema).clone());
        for finding in untyped_properties(&schema) {
            offending.insert((tool.name.to_string(), finding.path));
        }
    }

    let stale: Vec<String> = POLYMORPHIC_SLOTS
        .iter()
        .filter(|slot| !offending.contains(&(slot.tool.to_string(), slot.path.to_string())))
        .map(|slot| {
            format!(
                "  {} {} no longer needs an exemption (it claimed: {})",
                slot.tool, slot.path, slot.reason
            )
        })
        .collect();
    assert!(
        stale.is_empty(),
        "{} stale entr(y/ies) in POLYMORPHIC_SLOTS — the slot now announces one scalar type, \
         or moved, or stopped existing:\n{}",
        stale.len(),
        stale.join("\n")
    );
    client.cancel().await.expect("close the MCP session");
}

/// The regression that cost four round trips: `save_working_context`'s
/// nested arrays must be self-describing.
#[tokio::test]
async fn save_working_context_input_schema_types_every_reachable_items() {
    let (_store, client) = connected().await;
    let tools = client.list_all_tools().await.expect("list tools");
    let tool = tools
        .iter()
        .find(|tool| tool.name == "save_working_context")
        .expect("save_working_context is advertised");
    let schema = Value::Object((*tool.input_schema).clone());
    let untyped = untyped_items(&schema, Surface::Input);
    assert!(
        untyped.is_empty(),
        "save_working_context advertises {} untyped `items` (a $defs-blind harness sees an \
         array of anything): {untyped:#?}",
        untyped.len()
    );
    client.cancel().await.expect("close the MCP session");
}

/// The generic form of the same rule, over EVERY advertised tool — so the
/// next tool that takes an array of objects is covered by construction, not
/// by someone remembering to add a case here.
#[tokio::test]
async fn every_tool_input_schema_types_every_reachable_items() {
    let (_store, client) = connected().await;
    let tools = client.list_all_tools().await.expect("list tools");
    assert!(!tools.is_empty(), "the server advertises at least one tool");
    let mut offenders: BTreeSet<String> = BTreeSet::new();
    for tool in &tools {
        let schema = Value::Object((*tool.input_schema).clone());
        for finding in untyped_items(&schema, Surface::Input) {
            offenders.insert(format!("{}: {finding}", tool.name));
        }
    }
    assert!(
        offenders.is_empty(),
        "{} untyped `items` across the advertised input schemas: {offenders:#?}",
        offenders.len()
    );
    client.cancel().await.expect("close the MCP session");
}

/// Same rule on the OUTPUT side, which the input fix left behind.
///
/// It is not cosmetic: the official MCP SDKs validate a tool's
/// `structuredContent` against its advertised `outputSchema`. A `$ref` a
/// client cannot resolve is a result it may reject outright — a harsher
/// failure than the input case, where the server at least got to answer.
#[tokio::test]
async fn every_tool_output_schema_types_every_reachable_items() {
    let (_store, client) = connected().await;
    let tools = client.list_all_tools().await.expect("list tools");
    let mut checked = 0usize;
    let mut offenders: BTreeSet<String> = BTreeSet::new();
    for tool in &tools {
        let Some(output) = tool.output_schema.as_ref() else {
            continue;
        };
        checked += 1;
        let schema = Value::Object((**output).clone());
        for finding in untyped_items(&schema, Surface::Output) {
            offenders.insert(format!("{}: {finding}", tool.name));
        }
    }
    assert!(checked > 0, "at least one tool advertises an output schema");
    assert!(
        offenders.is_empty(),
        "{} untyped `items` across {checked} advertised output schemas: {offenders:#?}",
        offenders.len()
    );
    client.cancel().await.expect("close the MCP session");
}

/// A full external payload — the shape a client would build from the schema
/// alone — including the two collections no existing fixture ever fills:
/// `decisions` (a `ContextDecisionRef`, `rule_id` required) and
/// `exact_evidence` (a `SourceReference`, `handle` required). `fragment_id`
/// is sent as a decimal STRING past 2^53, the form a float-lossy client must
/// use to relay an id without rounding it.
/// The payload an external client sends: every array field populated, so the
/// round-trip actually exercises `ContextDecisionRef` and `SourceReference`
/// rather than skipping them the way the in-crate fixtures do.
fn external_working_payload() -> serde_json::Value {
    json!({
        "goal": "ship the schema fix",
        "active_constraints": [
            { "text": "never break the published wire shape" }
        ],
        "verified_facts": [
            {
                "text": "the inliner only went one level deep",
                "source": {
                    "fragment_id": BIG_FRAGMENT_ID,
                    "handle": "ctx://source/schema-rs",
                    "memory_id": 42
                }
            }
        ],
        "open_hypotheses": [
            { "text": "output schemas have the same latent hole" }
        ],
        "decisions": [
            { "fragment_id": BIG_FRAGMENT_ID, "rule_id": "preserve.code_fence" }
        ],
        "exact_evidence": [
            { "fragment_id": BIG_FRAGMENT_ID, "handle": "ctx://source/evidence" }
        ],
        "pending_actions": ["re-run the gate"]
    })
}

#[tokio::test]
async fn save_working_context_round_trips_external_payload_with_decisions_and_evidence() {
    let (_store, client) = connected().await;
    let working = external_working_payload();
    let saved = client
        .call_tool(
            CallToolRequestParams::new("save_working_context").with_arguments(as_args(json!({
                "project": "velesdb",
                "session": "b6-schema",
                "working": working,
            }))),
        )
        .await
        .expect("save_working_context accepts a complete external payload");
    assert_ne!(
        saved.is_error,
        Some(true),
        "save_working_context reported a tool error: {:?}",
        saved.content
    );

    let loaded = client
        .call_tool(
            CallToolRequestParams::new("load_working_context").with_arguments(as_args(json!({
                "project": "velesdb",
                "session": "b6-schema",
            }))),
        )
        .await
        .expect("load_working_context call");
    let structured = loaded
        .structured_content
        .expect("load_working_context returns structured content");
    assert_eq!(structured["found"], json!(true));
    let round_tripped = &structured["working"];

    assert_payload_survived_the_round_trip(round_tripped);

    client.cancel().await.expect("close the MCP session");
}

/// Every field of [`external_working_payload`] must come back byte-identical.
/// The `fragment_id` assertions are the point of the test: sent as a decimal
/// string past 2^53, they must return AS THAT SAME DECIMAL STRING.
///
/// It asserted `as_u64()` until 2026-07-29, and that was the defect wearing a
/// green test. `as_u64()` passes because this client is Rust, where
/// `serde_json` decodes a `u64` exactly — the float-lossy client the whole id
/// contract exists for cannot, and it is the one that resumes a session.
/// Requiring the string form here is what makes `save_working_context`'s
/// input (`string`) and `load_working_context`'s answer the SAME bytes.
fn assert_payload_survived_the_round_trip(round_tripped: &serde_json::Value) {
    assert_eq!(round_tripped["goal"], json!("ship the schema fix"));
    assert_eq!(
        round_tripped["decisions"][0]["fragment_id"],
        json!(BIG_FRAGMENT_ID),
        "the decision's fragment_id must come back as the exact decimal string that was sent"
    );
    assert_eq!(
        round_tripped["decisions"][0]["rule_id"],
        json!("preserve.code_fence")
    );
    assert_eq!(
        round_tripped["exact_evidence"][0]["fragment_id"],
        json!(BIG_FRAGMENT_ID)
    );
    assert_eq!(
        round_tripped["exact_evidence"][0]["handle"],
        json!("ctx://source/evidence")
    );
    assert_eq!(
        round_tripped["verified_facts"][0]["source"]["fragment_id"],
        json!(BIG_FRAGMENT_ID)
    );
    assert_eq!(
        round_tripped["verified_facts"][0]["source"]["handle"],
        json!("ctx://source/schema-rs")
    );
    assert_eq!(
        round_tripped["verified_facts"][0]["source"]["memory_id"],
        json!("42")
    );
    assert_eq!(
        round_tripped["active_constraints"][0]["text"],
        json!("never break the published wire shape")
    );
    assert_eq!(
        round_tripped["open_hypotheses"][0]["text"],
        json!("output schemas have the same latent hole")
    );
    assert_eq!(round_tripped["pending_actions"], json!(["re-run the gate"]));
}

/// Pruning unreferenced `$defs` must never orphan a surviving `$ref`.
///
/// Inlining copies definitions to their use sites, leaving most `$defs`
/// entries dead — 55 % of the published bytes, measured. Dropping them is
/// worth 40 KB across the 18 tools, but a definition still reachable through
/// a cycle guard's leftover `$ref`, or through another definition, MUST
/// survive. This walks every advertised schema and resolves every `$ref`
/// against what actually shipped.
#[tokio::test]
async fn no_advertised_ref_points_at_a_pruned_definition() {
    let (_store, client) = connected().await;
    let tools = client.list_all_tools().await.expect("list tools");
    let mut dangling: BTreeSet<String> = BTreeSet::new();
    for tool in &tools {
        for (kind, schema) in [
            ("input", Some(Value::Object((*tool.input_schema).clone()))),
            (
                "output",
                tool.output_schema
                    .as_ref()
                    .map(|s| Value::Object((**s).clone())),
            ),
        ] {
            let Some(schema) = schema else { continue };
            let available: BTreeSet<String> = schema
                .get("$defs")
                .and_then(Value::as_object)
                .map(|defs| defs.keys().cloned().collect())
                .unwrap_or_default();
            let mut wanted = BTreeSet::new();
            collect_ref_targets(&schema, &mut wanted);
            for name in wanted.difference(&available) {
                dangling.insert(format!("{} ({kind}): #/$defs/{name}", tool.name));
            }
        }
    }
    assert!(
        dangling.is_empty(),
        "{} `$ref`(s) point at a definition that was pruned away: {dangling:#?}",
        dangling.len()
    );
    client.cancel().await.expect("close the MCP session");
}

fn collect_ref_targets(value: &Value, out: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            if let Some(Value::String(target)) = map.get("$ref") {
                if let Some(name) = target.strip_prefix("#/$defs/") {
                    out.insert(name.to_owned());
                }
            }
            for sub in map.values() {
                collect_ref_targets(sub, out);
            }
        }
        Value::Array(entries) => {
            for sub in entries {
                collect_ref_targets(sub, out);
            }
        }
        _ => {}
    }
}

/// Every tool must ADVERTISE an output schema, not merely have a typed one.
///
/// Ten of the nineteen tools declared none at all, so rmcp derived one that
/// escaped every post-processing pass — the inliner included. A client
/// validating `structuredContent` against a `$ref` it cannot resolve may
/// reject a perfectly good result.
#[tokio::test]
async fn every_tool_advertises_an_output_schema() {
    let (_store, client) = connected().await;
    let tools = client.list_all_tools().await.expect("list tools");
    let missing: BTreeSet<String> = tools
        .iter()
        .filter(|tool| tool.output_schema.is_none())
        .map(|tool| tool.name.to_string())
        .collect();
    assert!(
        missing.is_empty(),
        "{} tool(s) advertise no output schema: {missing:#?}",
        missing.len()
    );
    client.cancel().await.expect("close the MCP session");
}

/// `required` must be the WHOLE truth about what a call needs.
///
/// The two rules above make each slot self-describing; this one makes the
/// *set* of slots honest — a distinct property, and one nothing checked.
///
/// It is a guard, not a repair. It was written on 2026-07-28 after a
/// scenario campaign appeared to show four tools rejecting a call that
/// carried exactly their declared-required fields. They do not: the server's
/// `required` was right all along — `compile_context` does advertise
/// `query`. The truncated `required` came from the client rendering the
/// schema, and the campaign trusted that rendering instead of the wire. The
/// finding was retracted; this test is what proved it wrong.
///
/// It earns its place anyway, because the property it pins had no coverage
/// and the two sibling rules cannot express it: they describe individual
/// slots, never which ones a call must carry. It does what a first-time
/// caller does — build the minimal call the schema describes, send it, and
/// require that the answer is not a complaint about a field the schema never
/// asked for. Business-rule rejections ("fact text must not be empty",
/// "memory 1 does not exist") are expected and fine; the dummy values are
/// deliberately trivial. Only `missing field` means the schema lied.
fn minimal_arguments(schema: &Value) -> Value {
    let Value::Object(map) = schema else {
        return Value::Null;
    };
    let mut out = Map::new();
    let (Some(Value::Array(required)), Some(Value::Object(properties))) =
        (map.get("required"), map.get("properties"))
    else {
        return Value::Object(out);
    };
    for name in required.iter().filter_map(Value::as_str) {
        let slot = properties.get(name).unwrap_or(&Value::Null);
        out.insert(name.to_string(), dummy_for(slot));
    }
    Value::Object(out)
}

/// The first value an enumerated slot lists, if it enumerates any.
fn first_enum_value(map: &Map<String, Value>) -> Option<Value> {
    match map.get("enum") {
        Some(Value::Array(values)) => values.first().cloned(),
        _ => None,
    }
}

/// The cheapest value the slot's advertised type admits. Nested objects are
/// built from their own `required`, so a lie one level down is caught too —
/// `explain_compilation`'s `request` is exactly that case.
///
/// A string slot is filled with `"1"` rather than `""`, and an enumerated one
/// with its first listed value. Both are about the same thing: a filler must
/// be accepted by the DESERIALIZER, or the call fails on the filler and says
/// nothing about the field under test. `""` is not a decimal id
/// (`SourceReference::fragment_id` rejects it) and `"1"` is not a variant of
/// `ColumnFilter::op` — with either mistake, the round-trip probe below would
/// report a type refusal caused by a sibling.
fn dummy_for(slot: &Value) -> Value {
    let Value::Object(map) = slot else {
        return Value::Null;
    };
    if let Some(listed) = first_enum_value(map) {
        return listed;
    }
    let primary = match map.get("type") {
        Some(Value::String(name)) => name.clone(),
        Some(Value::Array(names)) => names
            .iter()
            .filter_map(Value::as_str)
            .find(|name| *name != "null")
            .unwrap_or("null")
            .to_string(),
        _ => {
            // A union slot: take the first branch that names a type.
            for keyword in ["anyOf", "oneOf"] {
                if let Some(Value::Array(branches)) = map.get(keyword) {
                    if let Some(branch) = branches
                        .iter()
                        .find(|b| matches!(b, Value::Object(m) if m.get("type").is_some_and(|t| t != "null")))
                    {
                        return dummy_for(branch);
                    }
                }
            }
            return Value::Null;
        }
    };
    match primary.as_str() {
        "string" => Value::String("1".to_owned()),
        "number" | "integer" => json!(1),
        "boolean" => Value::Bool(true),
        "array" => json!([]),
        "object" => minimal_arguments(slot),
        _ => Value::Null,
    }
}

#[tokio::test]
async fn every_tool_accepts_a_call_carrying_exactly_its_required_fields() {
    let (_store, client) = connected().await;
    let tools = client.list_all_tools().await.expect("list tools");
    assert!(!tools.is_empty(), "the server advertises at least one tool");
    let mut liars: BTreeSet<String> = BTreeSet::new();
    for tool in &tools {
        let schema = Value::Object((*tool.input_schema).clone());
        let arguments = as_args(minimal_arguments(&schema));
        let outcome = client
            .call_tool(
                CallToolRequestParams::new(tool.name.clone()).with_arguments(arguments.clone()),
            )
            .await;
        let complaint = match &outcome {
            Err(error) => error.to_string(),
            Ok(result) => result
                .content
                .iter()
                .filter_map(|item| item.as_text().map(|text| text.text.clone()))
                .collect::<String>(),
        };
        if complaint.contains("missing field") {
            liars.insert(format!(
                "{}: sent {} -> {}",
                tool.name,
                Value::Object(arguments),
                complaint.trim()
            ));
        }
    }
    assert!(
        liars.is_empty(),
        "{} tool(s) reject a call carrying exactly the fields their schema declares required \
         — `required` understates the real contract: {liars:#?}",
        liars.len()
    );
    client.cancel().await.expect("close the MCP session");
}

// --- The advertised type must be what the server actually does -------------
//
// Until 2026-07-29 the tolerance of an id was declared in THREE places that
// could drift apart: the `deserialize_id` attribute on the field (the only
// one that is real), `context::wire::ID_KEYS`, and `schema::WIRE_ID_KEYS` —
// plus a per-tool list in each schema test. Three copies of one fact, and
// nothing compared them.
//
// The test below uses NO list. It reads the published schema, and for every
// slot it announces as a scalar it sends a real call twice — the value as a
// JSON integer, then as a decimal string — and requires the server to behave
// the way it just told the caller it would. `explain_compilation.fragment_id`
// is covered because it is published, not because it is named here.

/// One step of an argument path a caller can actually build.
#[derive(Clone)]
enum Step {
    Property(String),
    Items,
    /// A value of an open map (`additionalProperties`). The key is the
    /// caller's to choose, so the probe picks an arbitrary one.
    MapValue,
}

/// The key the probe invents for an open-map slot. Any key is admissible by
/// definition — `additionalProperties` is what the schema says about values
/// under names it does not enumerate.
const PROBE_MAP_KEY: &str = "probe";

/// rmcp's marker for an argument the server could not deserialize — the one
/// signal that separates "the wire form was refused" from "the business rule
/// said no". Everything else (an empty fact, a missing memory, an
/// unconfigured extractor) is a legitimate answer to a well-formed call.
const ARGUMENT_TYPE_REFUSAL: &str = "failed to deserialize parameters:";

/// Every slot the schema announces as a single scalar `integer` or `string`,
/// with the path needed to reach it.
///
/// `properties`, `items` and `additionalProperties` are followed: the three
/// keywords under which a caller can construct a value. The first version of
/// this walk followed only the first two and its comment claimed the third
/// could not hold a scalar slot — while `crate::schema` had just grown a
/// dedicated `additionalProperties` scalarization pass, and three tools
/// publish a map-valued policy through one
/// (`compile_context.policy.pricing.models`, whose values carry an `integer`
/// slot a caller fills in). A branch the walk skips is a branch the two
/// rules below exempt in silence — the opposite of what a list-free guard is
/// for. `probed_slots_reaches_a_scalar_under_additional_properties` pins it.
fn probed_slots(schema: &Value) -> Vec<(Vec<Step>, String)> {
    let mut found = Vec::new();
    walk_probes(schema, &mut Vec::new(), &mut found);
    found
}

fn walk_probes(node: &Value, path: &mut Vec<Step>, found: &mut Vec<(Vec<Step>, String)>) {
    let Value::Object(map) = node else {
        return;
    };
    if let Some(Value::Object(properties)) = map.get("properties") {
        for (name, slot) in properties {
            path.push(Step::Property(name.clone()));
            if let Some(kind) = probed_scalar(slot) {
                found.push((path.clone(), kind));
            }
            walk_probes(slot, path, found);
            path.pop();
        }
    }
    if let Some(items @ Value::Object(_)) = map.get("items") {
        path.push(Step::Items);
        walk_probes(items, path, found);
        path.pop();
    }
    if let Some(extra @ Value::Object(_)) = map.get("additionalProperties") {
        path.push(Step::MapValue);
        walk_probes(extra, path, found);
        path.pop();
    }
}

/// `"integer"` or `"string"`, and only those: the probe sends the two forms
/// an id can take on the wire, so it is meaningless on a boolean or an
/// object. An enumerated slot is skipped too — its value set is closed, and
/// neither `1` nor `"1"` belongs to it.
fn probed_scalar(slot: &Value) -> Option<String> {
    let Value::Object(map) = slot else {
        return None;
    };
    if map.contains_key("enum") || map.contains_key("const") {
        return None;
    }
    match map.get("type") {
        Some(Value::String(kind)) if kind == "integer" || kind == "string" => Some(kind.clone()),
        _ => None,
    }
}

/// The minimal call that carries `value` at `path`: every required sibling
/// along the way filled with the cheapest value its own schema admits, every
/// traversed array reduced to the single element that holds the path.
fn call_carrying(slot: &Value, path: &[Step], value: &Value) -> Value {
    match path.first() {
        None => value.clone(),
        Some(Step::Property(name)) => {
            let mut out = as_args(minimal_arguments(slot));
            let child = slot
                .get("properties")
                .and_then(|properties| properties.get(name))
                .cloned()
                .unwrap_or(Value::Null);
            out.insert(name.clone(), call_carrying(&child, &path[1..], value));
            Value::Object(out)
        }
        Some(Step::Items) => {
            let child = slot.get("items").cloned().unwrap_or(Value::Null);
            json!([call_carrying(&child, &path[1..], value)])
        }
        Some(Step::MapValue) => {
            let child = slot
                .get("additionalProperties")
                .cloned()
                .unwrap_or(Value::Null);
            json!({ PROBE_MAP_KEY: call_carrying(&child, &path[1..], value) })
        }
    }
}

fn render_path(path: &[Step]) -> String {
    let mut out = String::from("$");
    for step in path {
        match step {
            Step::Property(name) => {
                out.push('.');
                out.push_str(name);
            }
            Step::Items => out.push_str("[]"),
            Step::MapValue => out.push_str("{*}"),
        }
    }
    out
}

/// The server's complaint about a call, or an empty string when it had none.
async fn complaint_about(
    client: &RunningService<RoleClient, ()>,
    tool: &str,
    arguments: Value,
) -> String {
    let outcome = client
        .call_tool(CallToolRequestParams::new(tool.to_owned()).with_arguments(as_args(arguments)))
        .await;
    match &outcome {
        Err(error) => error.to_string(),
        Ok(result) => result
            .content
            .iter()
            .filter_map(|item| item.as_text().map(|text| text.text.clone()))
            .collect::<String>(),
    }
}

/// THE guard that replaces the three lists: what a slot ANNOUNCES and what
/// the server DOES are checked against each other, per slot, over the real
/// wire.
///
/// - a slot announced `string` must accept a decimal string. Announcing the
///   string form is how an id above 2^53 survives a float-lossy client; a
///   server that then refuses it publishes a contract it does not honour, and
///   the caller has no other form to fall back on.
/// - a slot announced `integer` must accept the integer form. It is the
///   trivial half, and it is what makes the first half non-vacuous: the rule
///   cannot be satisfied by announcing `string` everywhere.
#[tokio::test]
async fn every_scalar_input_slot_accepts_the_form_it_announces() {
    let (_store, client) = connected().await;
    let tools = client.list_all_tools().await.expect("list tools");
    assert!(!tools.is_empty(), "the server advertises at least one tool");

    let mut probed = 0usize;
    let mut broken: Vec<String> = Vec::new();
    for tool in &tools {
        let schema = Value::Object((*tool.input_schema).clone());
        for (path, kind) in probed_slots(&schema) {
            probed += 1;
            let announced = match kind.as_str() {
                "string" => json!("1"),
                _ => json!(1),
            };
            let arguments = call_carrying(&schema, &path, &announced);
            let complaint = complaint_about(&client, &tool.name, arguments.clone()).await;
            if complaint.contains(ARGUMENT_TYPE_REFUSAL) {
                broken.push(format!(
                    "  {} {} announces `{kind}` but refused {announced} — sent {arguments}\n    -> {}",
                    tool.name,
                    render_path(&path),
                    complaint.trim()
                ));
            }
        }
    }

    assert!(
        probed > 0,
        "the walk found no scalar slot at all — a green run would prove nothing"
    );
    assert!(
        broken.is_empty(),
        "{} of {probed} scalar slot(s) refuse the very form their schema announces:\n{}",
        broken.len(),
        broken.join("\n")
    );
    client.cancel().await.expect("close the MCP session");
}

/// A decimal string that only the ID CONTRACT accepts.
///
/// The complement below cannot simply be "a slot announced `integer` refuses
/// a string": measured on 2026-07-29, ten integer slots accept `"6"` —
/// `recall.limit`, `why.max_hops`, `compile_transcript.token_budget`… — and
/// that is not drift. It is `mcp::wire::lenient`, a deliberate, documented
/// server-side fallback for the harness that stringifies an argument BECAUSE
/// its view of the schema degraded. Removing it would break exactly the
/// clients it was written for, and the mission is to change what the client
/// READS, never what it may SEND.
///
/// So the probe must separate the two tolerances, and `"+1"` does it with no
/// list at all:
/// - the id contract (`model::deserialize_id`,
///   `context::wire::deserialize_optional_id`) parses with Rust's
///   `u64::from_str`, which accepts a leading `+` (pinned by
///   `model_tests::deserialize_id_plus_prefixed_string_parses`);
/// - `lenient` re-parses the string AS JSON, and JSON has no leading `+`, so
///   it refuses;
/// - a strict `u64` refuses too.
///
/// Accepting `"+1"` therefore means "this field carries the id contract",
/// which is per-field knowledge — precisely the kind that drifted across the
/// three lists.
const ID_CONTRACT_PROBE: &str = "+1";

/// The complement, and the reason the rule above cannot be satisfied by
/// announcing `string` everywhere: a slot that carries the id contract must
/// be ANNOUNCED as a string, never as an integer.
///
/// Together the two tests replace the three lists. One direction says an
/// announced `string` is honoured; this one says an id cannot hide behind an
/// `integer` announcement — which is how a caller ends up relaying an
/// `id_str` the schema told it not to send.
///
/// **What it does NOT catch, stated plainly.** The probe detects the id
/// DESERIALIZER, not the id-ness of the field: a slot that genuinely holds an
/// id but is a strict `u64` refuses `"+1"` like any counter, so it passes
/// here — and that is the worst of the two states, since the caller can then
/// neither read the id without rounding it nor send it back as a string.
/// `explain_compilation.fragment_id` was exactly that, and this test
/// certified it clean. The rule that catches the class is
/// `every_input_slot_named_for_an_id_announces_the_string_form`, which asks a
/// different question — does the surface itself call this name an id? — and
/// the two are complementary, not redundant.
#[tokio::test]
async fn no_integer_slot_hides_the_string_id_contract() {
    let (_store, client) = connected().await;
    let tools = client.list_all_tools().await.expect("list tools");
    let mut probed = 0usize;
    let mut hidden: Vec<String> = Vec::new();
    for tool in &tools {
        let schema = Value::Object((*tool.input_schema).clone());
        for (path, kind) in probed_slots(&schema) {
            if kind != "integer" {
                continue;
            }
            probed += 1;
            let arguments = call_carrying(&schema, &path, &json!(ID_CONTRACT_PROBE));
            let complaint = complaint_about(&client, &tool.name, arguments.clone()).await;
            if !complaint.contains(ARGUMENT_TYPE_REFUSAL) {
                hidden.push(format!(
                    "  {} {} — sent {arguments}",
                    tool.name,
                    render_path(&path)
                ));
            }
        }
    }
    assert!(
        probed > 0,
        "the walk found no integer slot at all — a green run would prove nothing"
    );
    assert!(
        hidden.is_empty(),
        "{} of {probed} slot(s) announced `integer` carry the string-or-number id contract — announce them \
         `string` (add the property name to the tool's `wire_safe_input_schema` keys), or drop \
         the contract. A caller that reads `integer` will relay a number, and an id above 2^53 \
         does not survive that on a float-lossy client:\n{}",
        hidden.len(),
        hidden.join("\n")
    );
    client.cancel().await.expect("close the MCP session");
}

// --- Les DEUX moities d'un aller-retour d'id -------------------------------
//
// Les deux regles ci-dessus ne regardent que l'ENTREE. Un aller-retour a deux
// jambes, et la revue adversariale du 2026-07-29 a montre que durcir une
// seule des deux fabrique une incoherence plutot que de la reparer : si
// `feedback.id` n'accepte plus qu'une chaine alors que `recall` ne rend qu'un
// nombre, le client n'a plus AUCUNE forme utilisable des deux cotes.
//
// Les deux tests qui suivent verrouillent la symetrie, et — comme les
// precedents — sans aucune liste ecrite a la main : l'ensemble des noms qui
// portent un id est LU sur la surface publiee. Le serveur est celui qui le
// declare, en typant un id `["integer", "string"]` en sortie
// (`widen_id_properties`) et en l'annoncant `string` en entree.

/// Un slot nomme : ou il est, ce qu'il annonce, et la fratrie ou il vit.
struct NamedSlot {
    path: String,
    name: String,
    slot: Value,
    siblings: BTreeSet<String>,
}

fn named_slots(schema: &Value) -> Vec<NamedSlot> {
    let mut found = Vec::new();
    walk_named_slots(schema, "$", &mut found);
    found
}

fn walk_named_slots(node: &Value, path: &str, found: &mut Vec<NamedSlot>) {
    let Value::Object(map) = node else {
        return;
    };
    if let Some(Value::Object(properties)) = map.get("properties") {
        let siblings: BTreeSet<String> = properties.keys().cloned().collect();
        for (name, slot) in properties {
            let child = format!("{path}.{name}");
            found.push(NamedSlot {
                path: child.clone(),
                name: name.clone(),
                slot: slot.clone(),
                siblings: siblings.clone(),
            });
            walk_named_slots(slot, &child, found);
        }
    }
    walk_named_children(map, path, found);
}

fn walk_named_children(map: &Map<String, Value>, path: &str, found: &mut Vec<NamedSlot>) {
    if let Some(items @ Value::Object(_)) = map.get("items") {
        walk_named_slots(items, &format!("{path}[]"), found);
    }
    if let Some(extra @ Value::Object(_)) = map.get("additionalProperties") {
        walk_named_slots(extra, &format!("{path}{{*}}"), found);
    }
    for keyword in ["anyOf", "oneOf"] {
        if let Some(Value::Array(branches)) = map.get(keyword) {
            for (index, branch) in branches.iter().enumerate() {
                walk_named_slots(branch, &format!("{path}|{keyword}[{index}]"), found);
            }
        }
    }
}

/// The shape `crate::schema::widen_id_properties` writes on the way OUT:
/// `["integer", "string"]`, directly or on the `items` of an id array
/// (`fragment_ids`). It is the server saying, in its own published bytes,
/// "a value under this name may legally be a decimal string".
fn carries_the_id_contract(slot: &Value) -> bool {
    if names_both_integer_and_string(slot) {
        return true;
    }
    matches!(slot.get("type"), Some(Value::String(kind)) if kind == "array")
        && slot.get("items").is_some_and(names_both_integer_and_string)
}

fn names_both_integer_and_string(slot: &Value) -> bool {
    let Some(Value::Array(forms)) = slot.get("type") else {
        return false;
    };
    let announced: BTreeSet<&str> = forms.iter().filter_map(Value::as_str).collect();
    announced.contains("integer") && announced.contains("string")
}

fn announces_exactly(slot: &Value, kind: &str) -> bool {
    matches!(slot.get("type"), Some(Value::String(name)) if name == kind)
}

/// Every property name any OUTPUT schema types with the id contract.
async fn id_contract_names(client: &RunningService<RoleClient, ()>) -> BTreeSet<String> {
    let tools = client.list_all_tools().await.expect("list tools");
    let mut names = BTreeSet::new();
    for tool in &tools {
        let Some(output) = tool.output_schema.as_ref() else {
            continue;
        };
        for slot in named_slots(&Value::Object((**output).clone())) {
            if carries_the_id_contract(&slot.slot) {
                names.insert(slot.name);
            }
        }
    }
    names
}

/// An id may only be ANNOUNCED one way on the input side, and that way is
/// the decimal string.
///
/// The rule that would have caught `explain_compilation.fragment_id`, which
/// three schema tests and two id-key lists all walked past: its own tool
/// emits `fragment_id` as a decimal string under
/// `CompilePolicy::ids_as_strings`, yet the parameter that selects a fragment
/// BY that id was a strict `u64` announced `integer`. The tool refused the
/// exact bytes it had just handed out.
///
/// No list: the id names are read back from the output schemas, where
/// `widen_id_properties` types them `["integer", "string"]`. A name the
/// server itself calls an id cannot be announced anything but `string` where
/// a caller has to supply it.
#[tokio::test]
async fn every_input_slot_named_for_an_id_announces_the_string_form() {
    let (_store, client) = connected().await;
    let id_names = id_contract_names(&client).await;
    assert!(
        !id_names.is_empty(),
        "no output schema types any field with the id contract — the rule below would be vacuous"
    );

    let tools = client.list_all_tools().await.expect("list tools");
    let mut probed = 0usize;
    let mut narrowed: Vec<String> = Vec::new();
    for tool in &tools {
        for slot in named_slots(&Value::Object((*tool.input_schema).clone())) {
            if !id_names.contains(&slot.name) {
                continue;
            }
            probed += 1;
            if !announces_exactly(&slot.slot, "string") {
                narrowed.push(format!(
                    "  {} {} announces {} — the tool's own results type `{}` as \
                     `[\"integer\", \"string\"]`",
                    tool.name,
                    slot.path,
                    slot.slot.get("type").unwrap_or(&Value::Null),
                    slot.name
                ));
            }
        }
    }

    assert!(
        probed > 0,
        "no input slot carries an id name at all — a green run would prove nothing"
    );
    assert!(
        narrowed.is_empty(),
        "{} of {probed} input slot(s) named for an id do not announce the decimal-string form. \
         Add the property name to that tool's `wire_safe_input_schema` keys AND give the field \
         the id contract (`model::deserialize_id` / `context::wire::deserialize_optional_id`), \
         or the caller cannot resubmit an id the server handed it as a string:\n{}",
        narrowed.len(),
        narrowed.join("\n")
    );
    client.cancel().await.expect("close the MCP session");
}

/// The OTHER leg: if a caller may only SEND an id as a string, it must be
/// able to READ one exactly.
///
/// `save_working_context` returned `{"id": <u64>}` and nothing else, while
/// `forget`/`feedback` announce `id` as a `string`: on a float-lossy client
/// the number is already rounded when it arrives, so there was no way to
/// build the string the input schema demands. Every other tool returning an
/// id already carried the `_str` twin — this rule is what makes that
/// convention enforceable instead of remembered.
///
/// Still no list: the id names come from the INPUT side (a name announced
/// `string` there is one a caller must relay), and the requirement lands on
/// the OUTPUT slots that answer under the same name as an `integer`.
#[tokio::test]
async fn every_lossy_id_in_a_result_offers_its_exact_string_twin() {
    let (_store, client) = connected().await;
    let tools = client.list_all_tools().await.expect("list tools");

    let mut relayed_as_strings: BTreeSet<String> = BTreeSet::new();
    for tool in &tools {
        for slot in named_slots(&Value::Object((*tool.input_schema).clone())) {
            if announces_exactly(&slot.slot, "string") && slot.slot.get("enum").is_none() {
                relayed_as_strings.insert(slot.name);
            }
        }
    }

    let mut probed = 0usize;
    let mut lossy: Vec<String> = Vec::new();
    for tool in &tools {
        let Some(output) = tool.output_schema.as_ref() else {
            continue;
        };
        for slot in named_slots(&Value::Object((**output).clone())) {
            if !relayed_as_strings.contains(&slot.name) || !announces_exactly(&slot.slot, "integer")
            {
                continue;
            }
            probed += 1;
            let twin = format!("{}_str", slot.name);
            if !slot.siblings.contains(&twin) {
                lossy.push(format!(
                    "  {} {} has no `{twin}` beside it",
                    tool.name, slot.path
                ));
            }
        }
    }

    assert!(
        probed > 0,
        "no result answers an integer under a name the input side wants as a string — \
         a green run would prove nothing"
    );
    assert!(
        lossy.is_empty(),
        "{} of {probed} result slot(s) hand back an id as a bare `integer` while the input side \
         accepts only the decimal string. A float-lossy client rounds the number on arrival and \
         can no longer build the string the schema asks for — add the `_str` twin, as \
         `RememberResult`, `ForgetResult` and `why`'s nodes already do:\n{}",
        lossy.len(),
        lossy.join("\n")
    );
    client.cancel().await.expect("close the MCP session");
}

/// `probed_slots` must reach a slot living under `additionalProperties`.
///
/// It did not, and its own doc-comment asserted the case could not exist —
/// while `crate::schema` had just grown a dedicated `additionalProperties`
/// scalarization pass, and three tools publish a map-valued policy through
/// one (`compile_context.policy.pricing.models`). A slot the walk never
/// visits is a slot the two rules above silently exempt.
#[test]
fn probed_slots_reaches_a_scalar_under_additional_properties() {
    let schema = json!({
        "type": "object",
        "properties": {
            "models": {
                "type": "object",
                "additionalProperties": {
                    "type": "object",
                    "properties": {
                        "input_micros_per_million_tokens": {"type": "integer"}
                    }
                }
            }
        }
    });
    let reached: Vec<String> = probed_slots(&schema)
        .into_iter()
        .map(|(path, _)| render_path(&path))
        .collect();
    assert!(
        reached
            .iter()
            .any(|path| path.ends_with("input_micros_per_million_tokens")),
        "the probe walk never reaches a scalar under `additionalProperties`: {reached:?}"
    );
}

/// The output rule, exercised where it actually bites: a legitimate `null`.
///
/// `load_working_context` answers `working: null` for a project+session that
/// was never saved — its documented, non-error miss. The official MCP SDKs
/// validate `structuredContent` against the advertised `outputSchema`, so the
/// day the input-side scalarization reaches an output schema, that answer
/// stops validating on the client and the server's own miss path breaks.
/// Nothing in this file tested it: `Surface::Output` accepts a slot that
/// merely carries SOME type, so a collapsed `anyOf: [WorkingContext, null]`
/// passed exactly like the union it replaced.
#[tokio::test]
async fn a_missing_working_context_answers_null_and_its_schema_still_admits_it() {
    let (_store, client) = connected().await;
    let tools = client.list_all_tools().await.expect("list tools");
    let tool = tools
        .iter()
        .find(|tool| tool.name == "load_working_context")
        .expect("load_working_context is advertised");
    let output = Value::Object(
        (**tool
            .output_schema
            .as_ref()
            .expect("load_working_context advertises an output schema"))
        .clone(),
    );

    let answered = client
        .call_tool(
            CallToolRequestParams::new("load_working_context").with_arguments(as_args(json!({
                "project": "velesdb",
                "session": "never-saved-anything-under-this-one",
            }))),
        )
        .await
        .expect("load_working_context call");
    let structured = answered
        .structured_content
        .expect("load_working_context returns structured content");
    assert_eq!(
        structured["working"],
        Value::Null,
        "a session that was never saved answers `working: null`: {structured}"
    );

    let slot = &output["properties"]["working"];
    assert!(
        admits_null(slot),
        "the server answers `working: null` but its advertised output schema no longer admits \
         null — an SDK that validates structuredContent rejects the server's own miss path: \
         {slot}"
    );
    client.cancel().await.expect("close the MCP session");
}

/// Whether a slot's advertised type accepts a JSON `null`: a direct
/// `"null"`, a list of forms containing it, or a union with a null branch.
fn admits_null(slot: &Value) -> bool {
    match slot.get("type") {
        Some(Value::String(kind)) => kind == "null",
        Some(Value::Array(names)) => names.iter().any(|name| name == "null"),
        _ => ["anyOf", "oneOf"].iter().any(|keyword| {
            matches!(slot.get(*keyword), Some(Value::Array(branches))
                if branches.iter().any(admits_null))
        }),
    }
}
