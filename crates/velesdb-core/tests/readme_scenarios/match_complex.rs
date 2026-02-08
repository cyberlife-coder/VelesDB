//! Complex MATCH scenario tests (BS2/BS3) — Phase 4 Plan 05.
//!
//! BS2: Fraud Detection — 2-hop MATCH with binding-aware WHERE on final node
//! BS3: Healthcare Diagnosis — 2-hop MATCH with IN on intermediate node + RETURN AVG
//! Variable-length path regression test

use std::collections::HashMap;

use velesdb_core::velesql::{
    CompareOp, Comparison, Condition, Direction, GraphPattern, InCondition, NodePattern,
    RelationshipPattern, Value,
};
use velesdb_core::DistanceMetric;

use super::helpers;

// ============================================================================
// BS2: Fraud Detection — Graph structure
// ============================================================================
//
// Transactions → Accounts → Linked Accounts (network)
//
// tx100 (Transaction, amount=5000) --FROM--> acc200 (Account, risk=low)
//     acc200 --LINKED_TO--> acc201 (Account, risk=high)     ← flagged
//     acc200 --LINKED_TO--> acc202 (Account, risk=medium)
//
// tx101 (Transaction, amount=15000) --FROM--> acc203 (Account, risk=medium)
//     acc203 --LINKED_TO--> acc204 (Account, risk=high)     ← flagged
//     acc203 --LINKED_TO--> acc205 (Account, risk=low)
//
// tx102 (Transaction, amount=200) --FROM--> acc206 (Account, risk=low)
//     acc206 --LINKED_TO--> acc207 (Account, risk=low)      ← not flagged
//
// For variable-length test, add deeper links:
//     acc201 --LINKED_TO--> acc208 (Account, risk=high)     ← depth 2
//     acc208 --LINKED_TO--> acc209 (Account, risk=medium)   ← depth 3

/// 3 Transactions + 10 Accounts for fraud detection network.
fn bs2_nodes() -> Vec<(u64, Vec<f32>, serde_json::Value)> {
    vec![
        // Transaction nodes
        (
            100,
            helpers::generate_embedding(100, 4),
            serde_json::json!({"_labels": ["Transaction"], "amount": 5000, "tx_type": "wire_transfer"}),
        ),
        (
            101,
            helpers::generate_embedding(101, 4),
            serde_json::json!({"_labels": ["Transaction"], "amount": 15000, "tx_type": "cash_withdrawal"}),
        ),
        (
            102,
            helpers::generate_embedding(102, 4),
            serde_json::json!({"_labels": ["Transaction"], "amount": 200, "tx_type": "purchase"}),
        ),
        // Account nodes
        (
            200,
            helpers::generate_embedding(200, 4),
            serde_json::json!({"_labels": ["Account"], "risk_level": "low", "account_id": "ACC-200"}),
        ),
        (
            201,
            helpers::generate_embedding(201, 4),
            serde_json::json!({"_labels": ["Account"], "risk_level": "high", "account_id": "ACC-201"}),
        ),
        (
            202,
            helpers::generate_embedding(202, 4),
            serde_json::json!({"_labels": ["Account"], "risk_level": "medium", "account_id": "ACC-202"}),
        ),
        (
            203,
            helpers::generate_embedding(203, 4),
            serde_json::json!({"_labels": ["Account"], "risk_level": "medium", "account_id": "ACC-203"}),
        ),
        (
            204,
            helpers::generate_embedding(204, 4),
            serde_json::json!({"_labels": ["Account"], "risk_level": "high", "account_id": "ACC-204"}),
        ),
        (
            205,
            helpers::generate_embedding(205, 4),
            serde_json::json!({"_labels": ["Account"], "risk_level": "low", "account_id": "ACC-205"}),
        ),
        (
            206,
            helpers::generate_embedding(206, 4),
            serde_json::json!({"_labels": ["Account"], "risk_level": "low", "account_id": "ACC-206"}),
        ),
        (
            207,
            helpers::generate_embedding(207, 4),
            serde_json::json!({"_labels": ["Account"], "risk_level": "low", "account_id": "ACC-207"}),
        ),
        // Deeper links for variable-length test
        (
            208,
            helpers::generate_embedding(208, 4),
            serde_json::json!({"_labels": ["Account"], "risk_level": "high", "account_id": "ACC-208"}),
        ),
        (
            209,
            helpers::generate_embedding(209, 4),
            serde_json::json!({"_labels": ["Account"], "risk_level": "medium", "account_id": "ACC-209"}),
        ),
    ]
}

/// Sets up the BS2 Fraud Detection graph.
///
/// Returns `(TempDir, Collection)` with the fraud detection network populated.
fn setup_bs2_scenario() -> (tempfile::TempDir, velesdb_core::Collection) {
    let (dir, db) = helpers::setup_test_db();
    let collection = helpers::setup_labeled_collection(&db, "fraud_net", 4, DistanceMetric::Cosine);

    helpers::insert_labeled_nodes(&collection, &bs2_nodes());

    // Edges: FROM (tx → account), LINKED_TO (account → account)
    helpers::add_edges(
        &collection,
        &[
            // FROM edges
            (1000, 100, 200, "FROM"),
            (1001, 101, 203, "FROM"),
            (1002, 102, 206, "FROM"),
            // LINKED_TO edges (depth 1)
            (2000, 200, 201, "LINKED_TO"),
            (2001, 200, 202, "LINKED_TO"),
            (2002, 203, 204, "LINKED_TO"),
            (2003, 203, 205, "LINKED_TO"),
            (2004, 206, 207, "LINKED_TO"),
            // Deeper links (depth 2, 3)
            (2005, 201, 208, "LINKED_TO"),
            (2006, 208, 209, "LINKED_TO"),
        ],
    );

    (dir, collection)
}

/// Builds the BS2 2-hop pattern:
/// `(tx:Transaction)-[:FROM]->(account:Account)-[:LINKED_TO]->(related:Account)`
fn build_bs2_pattern(linked_to_range: Option<(u32, u32)>) -> GraphPattern {
    GraphPattern {
        name: None,
        nodes: vec![
            NodePattern::new()
                .with_alias("tx")
                .with_label("Transaction"),
            NodePattern::new()
                .with_alias("account")
                .with_label("Account"),
            NodePattern::new()
                .with_alias("related")
                .with_label("Account"),
        ],
        relationships: vec![
            RelationshipPattern {
                alias: None,
                types: vec!["FROM".to_string()],
                direction: Direction::Outgoing,
                range: None,
                properties: HashMap::new(),
            },
            RelationshipPattern {
                alias: None,
                types: vec!["LINKED_TO".to_string()],
                direction: Direction::Outgoing,
                range: linked_to_range,
                properties: HashMap::new(),
            },
        ],
    }
}

// ============================================================================
// BS3: Healthcare Diagnosis — Graph structure
// ============================================================================
//
// patient300 --HAS_CONDITION--> cond400 (icd10=J18.9, pneumonia)
//     cond400 --TREATED_WITH--> treat500 (Amoxicillin, success_rate=0.85)
//     cond400 --TREATED_WITH--> treat501 (Azithromycin, success_rate=0.90)
//
// patient301 --HAS_CONDITION--> cond401 (icd10=J12.89, viral pneumonia)
//     cond401 --TREATED_WITH--> treat502 (Oseltamivir, success_rate=0.75)
//
// patient302 --HAS_CONDITION--> cond402 (icd10=K21.0, GERD — not in filter)
//     cond402 --TREATED_WITH--> treat503 (Omeprazole, success_rate=0.95)
//
// patient303 --HAS_CONDITION--> cond403 (icd10=J18.9, pneumonia — same code)
//     cond403 --TREATED_WITH--> treat500 (Amoxicillin again, shared treatment)

/// 4 Patients + 4 Conditions + 4 Treatments for healthcare scenario.
fn bs3_nodes() -> Vec<(u64, Vec<f32>, serde_json::Value)> {
    vec![
        // Patients
        (
            300,
            helpers::generate_embedding(300, 4),
            serde_json::json!({ "_labels": ["Patient"], "name": "Alice" }),
        ),
        (
            301,
            helpers::generate_embedding(301, 4),
            serde_json::json!({ "_labels": ["Patient"], "name": "Bob" }),
        ),
        (
            302,
            helpers::generate_embedding(302, 4),
            serde_json::json!({ "_labels": ["Patient"], "name": "Charlie" }),
        ),
        (
            303,
            helpers::generate_embedding(303, 4),
            serde_json::json!({ "_labels": ["Patient"], "name": "Diana" }),
        ),
        // Conditions
        (
            400,
            helpers::generate_embedding(400, 4),
            serde_json::json!({"_labels": ["Condition"], "icd10_code": "J18.9", "diagnosis": "Bacterial pneumonia"}),
        ),
        (
            401,
            helpers::generate_embedding(401, 4),
            serde_json::json!({"_labels": ["Condition"], "icd10_code": "J12.89", "diagnosis": "Viral pneumonia"}),
        ),
        (
            402,
            helpers::generate_embedding(402, 4),
            serde_json::json!({"_labels": ["Condition"], "icd10_code": "K21.0", "diagnosis": "GERD"}),
        ),
        (
            403,
            helpers::generate_embedding(403, 4),
            serde_json::json!({"_labels": ["Condition"], "icd10_code": "J18.9", "diagnosis": "Bacterial pneumonia"}),
        ),
        // Treatments
        (
            500,
            helpers::generate_embedding(500, 4),
            serde_json::json!({"_labels": ["Treatment"], "name": "Amoxicillin", "success_rate": 0.85}),
        ),
        (
            501,
            helpers::generate_embedding(501, 4),
            serde_json::json!({"_labels": ["Treatment"], "name": "Azithromycin", "success_rate": 0.90}),
        ),
        (
            502,
            helpers::generate_embedding(502, 4),
            serde_json::json!({"_labels": ["Treatment"], "name": "Oseltamivir", "success_rate": 0.75}),
        ),
        (
            503,
            helpers::generate_embedding(503, 4),
            serde_json::json!({"_labels": ["Treatment"], "name": "Omeprazole", "success_rate": 0.95}),
        ),
    ]
}

/// Sets up the BS3 Healthcare Diagnosis graph.
fn setup_bs3_scenario() -> (tempfile::TempDir, velesdb_core::Collection) {
    let (dir, db) = helpers::setup_test_db();
    let collection =
        helpers::setup_labeled_collection(&db, "healthcare", 4, DistanceMetric::Cosine);

    helpers::insert_labeled_nodes(&collection, &bs3_nodes());

    helpers::add_edges(
        &collection,
        &[
            // HAS_CONDITION
            (3000, 300, 400, "HAS_CONDITION"),
            (3001, 301, 401, "HAS_CONDITION"),
            (3002, 302, 402, "HAS_CONDITION"),
            (3003, 303, 403, "HAS_CONDITION"),
            // TREATED_WITH
            (4000, 400, 500, "TREATED_WITH"),
            (4001, 400, 501, "TREATED_WITH"),
            (4002, 401, 502, "TREATED_WITH"),
            (4003, 402, 503, "TREATED_WITH"),
            (4004, 403, 500, "TREATED_WITH"), // Diana→cond403→Amoxicillin (shared)
        ],
    );

    (dir, collection)
}

/// Builds the BS3 2-hop pattern:
/// `(patient:Patient)-[:HAS_CONDITION]->(condition:Condition)-[:TREATED_WITH]->(treatment:Treatment)`
fn build_bs3_pattern() -> GraphPattern {
    GraphPattern {
        name: None,
        nodes: vec![
            NodePattern::new()
                .with_alias("patient")
                .with_label("Patient"),
            NodePattern::new()
                .with_alias("condition")
                .with_label("Condition"),
            NodePattern::new()
                .with_alias("treatment")
                .with_label("Treatment"),
        ],
        relationships: vec![
            RelationshipPattern {
                alias: None,
                types: vec!["HAS_CONDITION".to_string()],
                direction: Direction::Outgoing,
                range: None,
                properties: HashMap::new(),
            },
            RelationshipPattern {
                alias: None,
                types: vec!["TREATED_WITH".to_string()],
                direction: Direction::Outgoing,
                range: None,
                properties: HashMap::new(),
            },
        ],
    }
}

// ============================================================================
// Tests
// ============================================================================

/// BS2 Fraud Detection: 2-hop MATCH with binding-aware WHERE on final node.
///
/// Pattern: `(tx:Transaction)-[:FROM]->(account:Account)-[:LINKED_TO]->(related:Account)`
/// WHERE: `related.risk_level = 'high'`
///
/// Expected results: 2 paths ending at high-risk accounts (acc201 via tx100, acc204 via tx101).
/// tx102 path excluded because acc207 has risk_level='low'.
#[test]
fn test_bs2_fraud_detection() {
    let (_dir, collection) = setup_bs2_scenario();

    // WHERE related.risk_level = 'high' (alias-qualified → binding-aware)
    let where_clause = Condition::Comparison(Comparison {
        column: "related.risk_level".to_string(),
        operator: CompareOp::Eq,
        value: Value::String("high".to_string()),
    });

    let match_clause = helpers::build_match_clause(
        vec![build_bs2_pattern(None)],
        Some(where_clause),
        vec![
            ("tx.amount", None),
            ("account.account_id", None),
            ("related.risk_level", None),
        ],
        None,
        Some(20),
    );

    let results = collection
        .execute_match(&match_clause, &HashMap::new())
        .expect("execute_match failed for BS2");

    // Expected: 2 fraud paths
    //   tx100 → acc200 → acc201 (high)
    //   tx101 → acc203 → acc204 (high)
    assert_eq!(
        results.len(),
        2,
        "Should find 2 fraud paths ending at high-risk accounts, got {}",
        results.len()
    );

    // All results should have the 3 aliases populated
    for result in &results {
        assert!(
            result.bindings.contains_key("tx"),
            "Result should have 'tx' binding"
        );
        assert!(
            result.bindings.contains_key("account"),
            "Result should have 'account' binding"
        );
        assert!(
            result.bindings.contains_key("related"),
            "Result should have 'related' binding"
        );
    }

    // All related accounts should have risk_level = 'high' in projection
    for result in &results {
        let risk = result
            .projected
            .get("related.risk_level")
            .and_then(|v| v.as_str());
        assert_eq!(
            risk,
            Some("high"),
            "Projected related.risk_level should be 'high', got {:?}",
            risk
        );
    }

    // Verify the specific fraud paths by checking tx node IDs
    let tx_ids: Vec<u64> = results
        .iter()
        .filter_map(|r| r.bindings.get("tx").copied())
        .collect();
    assert!(tx_ids.contains(&100), "tx100 should be in fraud results");
    assert!(tx_ids.contains(&101), "tx101 should be in fraud results");
    assert!(
        !tx_ids.contains(&102),
        "tx102 should NOT be in fraud results (no high-risk link)"
    );
}

/// BS3 Healthcare Diagnosis: 2-hop MATCH with IN condition + RETURN AVG aggregation.
///
/// Pattern: `(patient)-[:HAS_CONDITION]->(condition)-[:TREATED_WITH]->(treatment)`
/// WHERE: `condition.icd10_code IN ('J18.9', 'J12.89')`
/// RETURN: `treatment.name, AVG(treatment.success_rate)`
///
/// Expected: Amoxicillin avg=0.85 (from cond400 + cond403), Azithromycin=0.90, Oseltamivir=0.75
/// Omeprazole excluded because K21.0 not in filter.
#[test]
fn test_bs3_healthcare() {
    let (_dir, collection) = setup_bs3_scenario();

    // WHERE icd10_code IN ('J18.9', 'J12.89') — unqualified, falls through to each bound node
    let where_clause = Condition::In(InCondition {
        column: "icd10_code".to_string(),
        values: vec![
            Value::String("J18.9".to_string()),
            Value::String("J12.89".to_string()),
        ],
    });

    let match_clause = helpers::build_match_clause(
        vec![build_bs3_pattern()],
        Some(where_clause),
        vec![
            ("treatment.name", None),
            ("AVG(treatment.success_rate)", None),
        ],
        None,
        Some(50),
    );

    let results = collection
        .execute_match(&match_clause, &HashMap::new())
        .expect("execute_match failed for BS3");

    // Aggregation groups by treatment.name
    // Expected groups:
    //   Amoxicillin: paths from cond400→treat500, cond403→treat500 → AVG(0.85, 0.85) = 0.85
    //   Azithromycin: path from cond400→treat501 → AVG(0.90) = 0.90
    //   Oseltamivir: path from cond401→treat502 → AVG(0.75) = 0.75
    assert_eq!(
        results.len(),
        3,
        "Should have 3 treatment groups (Amoxicillin, Azithromycin, Oseltamivir), got {}",
        results.len()
    );

    // Find each treatment group and verify AVG
    let amox = results
        .iter()
        .find(|r| r.projected.get("treatment.name").and_then(|v| v.as_str()) == Some("Amoxicillin"))
        .expect("Amoxicillin group should exist");
    let amox_avg = amox
        .projected
        .get("AVG(treatment.success_rate)")
        .and_then(serde_json::Value::as_f64)
        .expect("AVG should be a number");
    assert!(
        (amox_avg - 0.85).abs() < 0.01,
        "Amoxicillin AVG should be ~0.85, got {}",
        amox_avg
    );

    let azith = results
        .iter()
        .find(|r| {
            r.projected.get("treatment.name").and_then(|v| v.as_str()) == Some("Azithromycin")
        })
        .expect("Azithromycin group should exist");
    let azith_avg = azith
        .projected
        .get("AVG(treatment.success_rate)")
        .and_then(serde_json::Value::as_f64)
        .expect("AVG should be a number");
    assert!(
        (azith_avg - 0.90).abs() < 0.01,
        "Azithromycin AVG should be ~0.90, got {}",
        azith_avg
    );

    let osel = results
        .iter()
        .find(|r| r.projected.get("treatment.name").and_then(|v| v.as_str()) == Some("Oseltamivir"))
        .expect("Oseltamivir group should exist");
    let osel_avg = osel
        .projected
        .get("AVG(treatment.success_rate)")
        .and_then(serde_json::Value::as_f64)
        .expect("AVG should be a number");
    assert!(
        (osel_avg - 0.75).abs() < 0.01,
        "Oseltamivir AVG should be ~0.75, got {}",
        osel_avg
    );

    // Omeprazole should NOT appear (K21.0 not in filter)
    let omeprazole = results
        .iter()
        .find(|r| r.projected.get("treatment.name").and_then(|v| v.as_str()) == Some("Omeprazole"));
    assert!(
        omeprazole.is_none(),
        "Omeprazole should be excluded (K21.0 not in IN filter)"
    );
}

/// BS3 Healthcare: verify raw results (without aggregation) to confirm IN filter works.
///
/// Uses the same graph but returns non-aggregated properties to verify the
/// IN condition filters intermediate nodes correctly.
#[test]
fn test_bs3_in_filter_raw_results() {
    let (_dir, collection) = setup_bs3_scenario();

    let where_clause = Condition::In(InCondition {
        column: "icd10_code".to_string(),
        values: vec![
            Value::String("J18.9".to_string()),
            Value::String("J12.89".to_string()),
        ],
    });

    // No aggregation — just project all three aliases
    let match_clause = helpers::build_match_clause(
        vec![build_bs3_pattern()],
        Some(where_clause),
        vec![
            ("patient.name", None),
            ("condition.icd10_code", None),
            ("treatment.name", None),
            ("treatment.success_rate", None),
        ],
        None,
        Some(50),
    );

    let results = collection
        .execute_match(&match_clause, &HashMap::new())
        .expect("execute_match failed");

    // Expected paths (IN filter passes for J18.9 and J12.89):
    //   Alice → cond400(J18.9) → Amoxicillin
    //   Alice → cond400(J18.9) → Azithromycin
    //   Bob → cond401(J12.89) → Oseltamivir
    //   Diana → cond403(J18.9) → Amoxicillin
    // Charlie excluded: cond402(K21.0) not in filter
    assert_eq!(
        results.len(),
        4,
        "Should return 4 treatment paths (excluding K21.0), got {}",
        results.len()
    );

    // All icd10 codes in results should be in the filter set
    for result in &results {
        let code = result
            .projected
            .get("condition.icd10_code")
            .and_then(|v| v.as_str())
            .expect("icd10_code should be projected");
        assert!(
            code == "J18.9" || code == "J12.89",
            "Only J18.9 or J12.89 should pass IN filter, got '{}'",
            code
        );
    }
}

/// BS2 Variable-length path: `LINKED_TO*1..3` produces results at multiple depths.
///
/// With single-hop (`LINKED_TO`), tx100 reaches acc201 and acc202.
/// With `*1..3`, tx100 also reaches acc208 (depth 2) and acc209 (depth 3).
#[test]
fn test_variable_length_fraud() {
    let (_dir, collection) = setup_bs2_scenario();

    // Single-hop baseline (for comparison)
    let single_hop_clause = helpers::build_match_clause(
        vec![build_bs2_pattern(None)],
        None, // No WHERE — count all paths
        vec![("related.account_id", None)],
        None,
        Some(100),
    );
    let single_hop_results = collection
        .execute_match(&single_hop_clause, &HashMap::new())
        .expect("single-hop execute_match failed");

    // Variable-length: LINKED_TO*1..3
    let var_length_clause = helpers::build_match_clause(
        vec![build_bs2_pattern(Some((1, 3)))],
        None,
        vec![("related.account_id", None)],
        None,
        Some(100),
    );
    let var_length_results = collection
        .execute_match(&var_length_clause, &HashMap::new())
        .expect("variable-length execute_match failed");

    // Variable-length should produce MORE results than single-hop
    assert!(
        var_length_results.len() > single_hop_results.len(),
        "Variable-length (*1..3) should produce more results ({}) than single-hop ({})",
        var_length_results.len(),
        single_hop_results.len()
    );

    // Single-hop: 5 paths (tx100→{201,202}, tx101→{204,205}, tx102→{207})
    assert_eq!(
        single_hop_results.len(),
        5,
        "Single-hop should find 5 paths, got {}",
        single_hop_results.len()
    );

    // Variable-length should include deeper accounts (acc208, acc209)
    let var_related_ids: Vec<u64> = var_length_results
        .iter()
        .filter_map(|r| r.bindings.get("related").copied())
        .collect();
    assert!(
        var_related_ids.contains(&208),
        "Variable-length should reach acc208 at depth 2"
    );
    assert!(
        var_related_ids.contains(&209),
        "Variable-length should reach acc209 at depth 3"
    );
}
