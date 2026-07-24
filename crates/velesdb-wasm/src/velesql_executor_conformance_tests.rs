//! Executor-level VelesQL conformance for the WASM executor.
//!
//! Companion to `velesdb-core/tests/velesql_executor_conformance.rs`. Loads the
//! SAME golden fixture (`conformance/velesql_executor_cases.json`), builds the
//! dataset **through the WASM executor** (`INSERT INTO ... VALUES`), runs each
//! case query through `velesql_exec::execute`, and asserts the result ids and
//! ordering match the golden `expect_ids` derived from `velesdb-core`.
//!
//! Native-target test (`#[test]`, not `wasm_bindgen_test`) that exercises
//! `DatabaseInner` through the executor dispatcher, mirroring
//! `velesql_exec_tests.rs`. The internal `DatabaseInner` / `execute` symbols
//! are `pub(crate)`, so this suite lives in-crate (like the other
//! `velesql_exec_*` test modules) rather than under `tests/`.
//!
//! # Case coverage
//!
//! All ten `cases` (X001–X010) are runnable by the WASM executor (scalar WHERE
//! filter, single- and multi-column ORDER BY, the ascending-id tie-break, and
//! bounded top-k) and are asserted against the golden ids. WASM runs its OWN
//! SELECT/ORDER BY pipeline (`velesql_select`/`velesql_orderby`), independent of
//! the core executor, so these goldens pin it against the `velesdb-core` result
//! set rather than assuming shared-executor equivalence.
//!
//! # known_bugs (B001)
//!
//! B001 is the core bug where `ORDER BY <column> + LIMIT` applies the LIMIT
//! before the sort on the bounded top-k path. The WASM SELECT pipeline
//! (`velesql_select::finalize_plain`) sorts the full row set *first* and only
//! then applies `skip(offset).take(limit)`, so it does NOT reproduce B001 — it
//! already returns the CORRECT golden result. The B001 case is therefore
//! asserted directly here as a non-bug for WASM (no `#[ignore]`).

use crate::database::DatabaseInner;
use crate::velesql_exec::execute;

use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;

#[derive(Deserialize)]
struct Fixture {
    dataset: Dataset,
    cases: Vec<Case>,
    #[serde(default)]
    known_bugs: Vec<Case>,
    #[serde(default)]
    extra_collections: Vec<Dataset>,
    #[serde(default)]
    join_cases: Vec<Case>,
    #[serde(default)]
    aggregate_cases: Vec<AggregateCase>,
    #[serde(default)]
    setops_cases: Vec<Case>,
    #[serde(default)]
    match_cases: Vec<MatchCase>,
}

#[derive(Deserialize)]
struct Dataset {
    collection: String,
    dimension: usize,
    metric: String,
    points: Vec<FixturePoint>,
}

#[derive(Deserialize)]
struct FixturePoint {
    id: u64,
    vector: Vec<f32>,
    payload: Value,
}

#[derive(Deserialize)]
struct Case {
    id: String,
    query: String,
    expect_ids: Vec<u64>,
}

/// A GROUP BY / HAVING golden case (issue #1544). Unlike core (which has a
/// separate `execute_aggregate` entry point), WASM's default SELECT dispatch
/// already runs the aggregation pipeline (`velesql_aggregate::apply`), so this
/// still goes through the ordinary `execute()` — only the assertion differs
/// (compares full JSON rows, not just ids, since a group has no natural point
/// id).
#[derive(Deserialize)]
struct AggregateCase {
    id: String,
    query: String,
    expect_groups: Vec<Value>,
}

/// A MATCH golden case (issue #1544). WASM addresses its graph store via
/// `@collection`-annotated standalone `MATCH ... RETURN` queries, which is
/// why this carries its own `wasm_query` instead of reusing `core_query` —
/// see `documented_divergences` D004 in the fixture.
#[derive(Deserialize)]
struct MatchCase {
    id: String,
    wasm_query: String,
    expect_ids: Vec<u64>,
}

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../conformance/velesql_executor_cases.json")
}

fn load_fixture() -> Fixture {
    let content = std::fs::read_to_string(fixture_path()).expect("test: read executor fixture");
    serde_json::from_str(&content).expect("test: parse executor fixture")
}

/// Renders a payload scalar as a VelesQL literal (quoting strings).
fn payload_literal(v: &Value) -> String {
    match v {
        Value::String(s) => format!("'{}'", s.replace('\'', "''")),
        other => other.to_string(),
    }
}

/// Builds the dataset *through the WASM executor* by issuing one
/// `INSERT INTO <coll> (id, vector, <payload keys...>) VALUES (...)` per point,
/// with the vector bound to the `$v` parameter (the canonical WASM vector-
/// insert contract — inline vector literals are unsupported).
fn build_dataset(ds: &Dataset) -> DatabaseInner {
    let mut db = DatabaseInner::new();
    seed_dataset(&mut db, ds);
    db
}

/// Seeds one collection (`ds`) into an existing `DatabaseInner`, for cases
/// that span more than one collection (issue #1544 JOIN cases). Extracted
/// from `build_dataset` so both share the exact same `INSERT INTO` seeding
/// mechanism.
fn seed_dataset(db: &mut DatabaseInner, ds: &Dataset) {
    db.create_collection(&ds.collection, ds.dimension, &ds.metric)
        .unwrap_or_else(|e| panic!("test: create collection {}: {e}", ds.collection));

    for point in &ds.points {
        let Value::Object(obj) = &point.payload else {
            panic!("test: fixture payload must be a JSON object");
        };
        let mut columns = vec!["id".to_string(), "vector".to_string()];
        let mut values = vec![point.id.to_string(), "$v".to_string()];
        for (key, val) in obj {
            columns.push(key.clone());
            values.push(payload_literal(val));
        }
        let sql = format!(
            "INSERT INTO {} ({}) VALUES ({})",
            ds.collection,
            columns.join(", "),
            values.join(", ")
        );
        let vector_json =
            serde_json::to_string(&point.vector).expect("test: serialize fixture vector");
        let params = format!("{{\"v\": {vector_json}}}");
        execute(db, &sql, Some(&params))
            .unwrap_or_else(|e| panic!("test: seed point {} failed: {e}", point.id));
    }
}

/// Runs every case through the WASM executor and asserts ids + ordering.
fn assert_cases(db: &mut DatabaseInner, cases: &[Case]) {
    for case in cases {
        let result = execute(db, &case.query, None)
            .unwrap_or_else(|e| panic!("case {} failed to execute: {e}", case.id));
        let ids: Vec<u64> = result
            .rows_ref()
            .iter()
            .map(crate::velesql_result::QueryResultRow::id)
            .collect();
        assert_eq!(
            ids, case.expect_ids,
            "WASM executor conformance mismatch for case {}: query={:?}",
            case.id, case.query
        );
    }
}

/// All golden `cases` (X001–X010) must reproduce the core result set exactly.
#[test]
fn test_wasm_velesql_executor_conformance_fixture_cases() {
    let fixture = load_fixture();
    let mut db = build_dataset(&fixture.dataset);
    assert_cases(&mut db, &fixture.cases);
}

/// B001 is a core-only bug: the WASM SELECT pipeline sorts before applying
/// LIMIT, so it returns the CORRECT golden result. Asserting it here locks in
/// that the WASM executor is NOT affected by the core bounded-top-k bug.
#[test]
fn test_wasm_velesql_executor_known_bugs_are_correct() {
    let fixture = load_fixture();
    let mut db = build_dataset(&fixture.dataset);
    assert_cases(&mut db, &fixture.known_bugs);
}

/// UNION / INTERSECT / EXCEPT conformance against the WASM executor (issue #1544).
///
/// Compared as a **sorted set of ids**, not an ordered list: `velesql_setops`
/// concatenates branches left-to-right (preserving each branch's own ORDER BY)
/// and de-duplicates first-seen, which is a *different* — but equally valid —
/// sequence than core's score-sort. Only membership and count are part of the
/// golden contract (`documented_divergences` D003 in the fixture).
#[test]
fn test_wasm_velesql_executor_setops_cases() {
    let fixture = load_fixture();
    let mut db = build_dataset(&fixture.dataset);
    for case in &fixture.setops_cases {
        let result = execute(&mut db, &case.query, None)
            .unwrap_or_else(|e| panic!("setops case {} failed to execute: {e}", case.id));
        let mut ids: Vec<u64> = result
            .rows_ref()
            .iter()
            .map(crate::velesql_result::QueryResultRow::id)
            .collect();
        ids.sort_unstable();
        let mut expected = case.expect_ids.clone();
        expected.sort_unstable();
        assert_eq!(
            ids, expected,
            "WASM setops conformance mismatch (compared as a set) for case {}: query={:?}",
            case.id, case.query
        );
    }
}

/// JOIN (ON and USING) conformance against the WASM executor (issue #1544).
///
/// Seeds `docs` plus every `extra_collections` entry into one `DatabaseInner`
/// via [`seed_dataset`], then runs each case exactly like a plain SELECT.
/// Unlike core (whose `SearchResult.point.id` directly carries the base row's
/// id), a WASM joined row's [`QueryResultRow::id`] is always the synthetic
/// placeholder `0` — the real id lives in the row's flattened JSON body under
/// the `"id"` key — so this asserts against `data_json["id"]`, not `.id()`.
#[test]
fn test_wasm_velesql_executor_join_cases() {
    let fixture = load_fixture();
    let mut db = build_dataset(&fixture.dataset);
    for extra in &fixture.extra_collections {
        seed_dataset(&mut db, extra);
    }
    for case in &fixture.join_cases {
        let result = execute(&mut db, &case.query, None)
            .unwrap_or_else(|e| panic!("join case {} failed to execute: {e}", case.id));
        let ids: Vec<u64> = result
            .rows_ref()
            .iter()
            .map(|row| {
                let parsed: Value =
                    serde_json::from_str(&row.data_json()).expect("test: parse row JSON");
                parsed
                    .get("id")
                    .and_then(Value::as_u64)
                    .unwrap_or_else(|| panic!("join case {}: row missing numeric id", case.id))
            })
            .collect();
        assert_eq!(
            ids, case.expect_ids,
            "WASM join conformance mismatch for case {}: query={:?}",
            case.id, case.query
        );
    }
}

/// GROUP BY / HAVING conformance against the WASM executor (issue #1544).
///
/// Compared as JSON values (a group has no natural point id), mirroring the
/// core aggregate test. Every case uses an explicit `AS` alias so the output
/// field names agree with core's `execute_aggregate` naming — see
/// `documented_divergences` D001 for the (deliberately unexercised) unaliased
/// naming gap.
#[test]
fn test_wasm_velesql_executor_aggregate_cases() {
    let fixture = load_fixture();
    let mut db = build_dataset(&fixture.dataset);
    for case in &fixture.aggregate_cases {
        let result = execute(&mut db, &case.query, None)
            .unwrap_or_else(|e| panic!("aggregate case {} failed to execute: {e}", case.id));
        let groups: Vec<Value> = result
            .rows_ref()
            .iter()
            .map(|row| serde_json::from_str(&row.data_json()).expect("test: parse row JSON"))
            .collect();
        assert_eq!(
            groups, case.expect_groups,
            "WASM aggregate conformance mismatch for case {}: query={:?}",
            case.id, case.query
        );
    }
}

/// MATCH (1- and 2-hop) conformance against the WASM executor (issue #1544).
///
/// WASM keeps a graph store separate from vector collections, populated via
/// `INSERT NODE`/`INSERT EDGE` and addressed by `@collection` annotations
/// (see `documented_divergences` D004 in the fixture). Builds the SAME
/// 4-node `KNOWS` chain topology as the core MATCH test
/// (`test_velesql_executor_conformance_match_cases`) and runs each case's
/// `wasm_query`.
#[test]
fn test_wasm_velesql_executor_match_cases() {
    let fixture = load_fixture();
    let mut db = DatabaseInner::new();
    let names = ["Ann", "Bo", "Cy", "Di"];
    for (i, name) in names.iter().enumerate() {
        let id = i + 1;
        let sql = format!(
            "INSERT NODE INTO people (id = {id}, payload = '{{\"name\": \"{name}\", \"labels\": [\"Person\"]}}')"
        );
        execute(&mut db, &sql, None).unwrap_or_else(|e| panic!("test: insert node {id}: {e}"));
    }
    for (src, dst) in [(1, 2), (2, 3), (3, 4)] {
        let sql =
            format!("INSERT EDGE INTO people (source = {src}, target = {dst}, label = 'KNOWS')");
        execute(&mut db, &sql, None)
            .unwrap_or_else(|e| panic!("test: insert edge {src}->{dst}: {e}"));
    }

    for case in &fixture.match_cases {
        let result = execute(&mut db, &case.wasm_query, None)
            .unwrap_or_else(|e| panic!("match case {} failed to execute: {e}", case.id));
        let ids: Vec<u64> = result
            .rows_ref()
            .iter()
            .map(|row| {
                let parsed: Value =
                    serde_json::from_str(&row.data_json()).expect("test: parse row JSON");
                parsed
                    .get("a")
                    .and_then(|a| a.get("id"))
                    .and_then(Value::as_u64)
                    .unwrap_or_else(|| panic!("match case {}: row missing a.id", case.id))
            })
            .collect();
        assert_eq!(
            ids, case.expect_ids,
            "WASM match conformance mismatch for case {}: wasm_query={:?}",
            case.id, case.wasm_query
        );
    }
}

/// Documents a real, currently-existing WASM JOIN gap
/// (`documented_divergences` D002 in the fixture): `ON <base>.<col> =
/// <joined>.<col>` matches correctly, but the reversed condition order
/// (`ON <joined>.<col> = <base>.<col>`) fails to match ANY row and every
/// joined column comes back NULL, because `equality_keys`/`key_of`
/// (`velesql_join.rs`) does not normalize which side names the base vs.
/// joined table the way core's `normalize_join_condition` does.
///
/// This is a **regression-lock for the current (buggy) behaviour**, not a
/// desired outcome: fixing the underlying normalization is out of scope for
/// issue #1544 (coverage of existing behaviour, not new WASM support). If
/// this test starts failing because someone *fixed* the normalization, that
/// is good news — update this test and `documented_divergences` D002 in
/// `conformance/velesql_executor_cases.json` to match.
#[test]
fn test_wasm_join_condition_side_order_gap_d002() {
    let mut db = DatabaseInner::new();
    db.create_collection("customers", 2, "cosine")
        .expect("test: create customers");
    db.create_collection("orders", 2, "cosine")
        .expect("test: create orders");
    execute(
        &mut db,
        "INSERT INTO customers (id, vector, name) VALUES (10, $v, 'Alice')",
        Some("{\"v\": [1.0, 0.0]}"),
    )
    .expect("test: seed customer");
    execute(
        &mut db,
        "INSERT INTO orders (id, vector, customer_id) VALUES (1, $v, 10)",
        Some("{\"v\": [1.0, 0.0]}"),
    )
    .expect("test: seed order");

    // Working order: base-table-first. Matches.
    let working = execute(
        &mut db,
        "SELECT * FROM orders LEFT JOIN customers ON orders.customer_id = customers.id",
        None,
    )
    .expect("test: working-order join");
    let working_row: Value =
        serde_json::from_str(&working.rows_ref()[0].data_json()).expect("test: parse row");
    assert_eq!(
        working_row.get("name").and_then(Value::as_str),
        Some("Alice"),
        "base-table-first ON order should match and enrich the row with the customer name"
    );

    // Reversed order: joined-table-first. Currently fails to match (D002).
    let reversed = execute(
        &mut db,
        "SELECT * FROM orders LEFT JOIN customers ON customers.id = orders.customer_id",
        None,
    )
    .expect("test: reversed-order join");
    let reversed_row: Value =
        serde_json::from_str(&reversed.rows_ref()[0].data_json()).expect("test: parse row");
    assert!(
        reversed_row.get("name").is_none() || reversed_row.get("name") == Some(&Value::Null),
        "documented WASM gap D002: joined-table-first ON order currently does NOT match \
         (customer name should be absent/null); if this assertion now fails, the underlying \
         bug was fixed — update documented_divergences D002 accordingly"
    );
}
