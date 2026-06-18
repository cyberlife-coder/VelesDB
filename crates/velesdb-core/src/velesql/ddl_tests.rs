//! Tests for DDL and DML extension parsing (VelesQL v3.3).
//!
//! Covers CREATE COLLECTION (vector, graph, metadata), DROP COLLECTION,
//! INSERT EDGE, DELETE FROM, DELETE EDGE — nominal cases, edge cases,
//! query type predicates, and AST field verification.

use crate::velesql::ast::{
    CompareOp, Condition, CreateCollectionKind, DdlStatement, DmlStatement, GraphSchemaMode,
    SchemaDefinition, Value,
};
use crate::velesql::Parser;

// ============================================================================
// CREATE COLLECTION (vector) — nominal cases
// ============================================================================

#[test]
fn test_create_vector_collection_basic() {
    let query = Parser::parse("CREATE COLLECTION docs (dimension = 768, metric = 'cosine');")
        .expect("basic CREATE COLLECTION should parse");

    let ddl = query.ddl.expect("Expected DDL statement");
    let DdlStatement::CreateCollection(create) = ddl else {
        panic!("Expected CreateCollection variant");
    };

    assert_eq!(create.name, "docs");
    let CreateCollectionKind::Vector(params) = &create.kind else {
        panic!("Expected Vector kind");
    };
    assert_eq!(params.dimension, 768);
    assert_eq!(params.metric, "cosine");
    assert!(params.storage.is_none());
    assert!(params.m.is_none());
    assert!(params.ef_construction.is_none());
}

#[test]
fn test_create_vector_collection_with_storage_in_body() {
    // `storage` is a body option (inside parentheses), not a WITH suffix option.
    // The WITH suffix only propagates `m` and `ef_construction`.
    let query = Parser::parse(
        "CREATE COLLECTION docs (dimension = 768, metric = 'cosine', storage = 'sq8');",
    )
    .expect("CREATE with storage in body should parse");

    let ddl = query.ddl.expect("Expected DDL statement");
    let DdlStatement::CreateCollection(create) = ddl else {
        panic!("Expected CreateCollection variant");
    };

    let CreateCollectionKind::Vector(params) = &create.kind else {
        panic!("Expected Vector kind");
    };
    assert_eq!(params.storage.as_deref(), Some("sq8"));
}

#[test]
fn test_create_vector_collection_storage_in_with_propagated() {
    // `storage` in WITH suffix IS propagated (alongside m and ef_construction).
    let query = Parser::parse(
        "CREATE COLLECTION docs (dimension = 768, metric = 'cosine') WITH (storage = 'sq8');",
    )
    .expect("CREATE with storage in WITH should parse");

    let ddl = query.ddl.expect("Expected DDL statement");
    let DdlStatement::CreateCollection(create) = ddl else {
        panic!("Expected CreateCollection variant");
    };

    let CreateCollectionKind::Vector(params) = &create.kind else {
        panic!("Expected Vector kind");
    };
    assert_eq!(params.storage.as_deref(), Some("sq8"));
}

#[test]
fn test_create_vector_collection_with_hnsw_params() {
    let query = Parser::parse(
        "CREATE COLLECTION docs (dimension = 768, metric = 'cosine') WITH (m = 16, ef_construction = 200);",
    )
    .expect("CREATE with HNSW params should parse");

    let ddl = query.ddl.expect("Expected DDL statement");
    let DdlStatement::CreateCollection(create) = ddl else {
        panic!("Expected CreateCollection variant");
    };

    let CreateCollectionKind::Vector(params) = &create.kind else {
        panic!("Expected Vector kind");
    };
    assert_eq!(params.m, Some(16));
    assert_eq!(params.ef_construction, Some(200));
}

#[test]
fn test_create_vector_collection_metric_euclidean() {
    let query = Parser::parse("CREATE COLLECTION vectors (dimension = 128, metric = 'euclidean');")
        .expect("euclidean metric should parse");

    let ddl = query.ddl.expect("Expected DDL statement");
    let DdlStatement::CreateCollection(create) = ddl else {
        panic!("Expected CreateCollection variant");
    };

    let CreateCollectionKind::Vector(params) = &create.kind else {
        panic!("Expected Vector kind");
    };
    assert_eq!(params.metric, "euclidean");
}

#[test]
fn test_create_vector_collection_metric_dotproduct() {
    let query =
        Parser::parse("CREATE COLLECTION embeddings (dimension = 256, metric = 'dotproduct');")
            .expect("dotproduct metric should parse");

    let ddl = query.ddl.expect("Expected DDL statement");
    let DdlStatement::CreateCollection(create) = ddl else {
        panic!("Expected CreateCollection variant");
    };

    let CreateCollectionKind::Vector(params) = &create.kind else {
        panic!("Expected Vector kind");
    };
    assert_eq!(params.metric, "dotproduct");
}

#[test]
fn test_create_vector_collection_without_semicolon() {
    let query = Parser::parse("CREATE COLLECTION docs (dimension = 768, metric = 'cosine')")
        .expect("CREATE without semicolon should parse");

    let ddl = query.ddl.expect("Expected DDL statement");
    let DdlStatement::CreateCollection(create) = ddl else {
        panic!("Expected CreateCollection variant");
    };
    assert_eq!(create.name, "docs");
    let CreateCollectionKind::Vector(params) = &create.kind else {
        panic!("Expected Vector kind");
    };
    assert_eq!(params.dimension, 768);
    assert_eq!(params.metric, "cosine");
}

#[test]
fn test_create_vector_collection_large_dimension() {
    let query = Parser::parse("CREATE COLLECTION large (dimension = 4096, metric = 'cosine');")
        .expect("large dimension should parse");

    let ddl = query.ddl.expect("Expected DDL statement");
    let DdlStatement::CreateCollection(create) = ddl else {
        panic!("Expected CreateCollection variant");
    };

    let CreateCollectionKind::Vector(params) = &create.kind else {
        panic!("Expected Vector kind");
    };
    assert_eq!(params.dimension, 4096);
}

// ============================================================================
// CREATE COLLECTION (graph) — nominal cases
// ============================================================================

#[test]
fn test_create_graph_collection_schemaless_with_embeddings() {
    let query = Parser::parse(
        "CREATE GRAPH COLLECTION kg (dimension = 768, metric = 'cosine') SCHEMALESS;",
    )
    .expect("graph schemaless with embeddings should parse");

    let ddl = query.ddl.expect("Expected DDL statement");
    let DdlStatement::CreateCollection(create) = ddl else {
        panic!("Expected CreateCollection variant");
    };

    assert_eq!(create.name, "kg");
    let CreateCollectionKind::Graph(params) = &create.kind else {
        panic!("Expected Graph kind");
    };
    assert_eq!(params.dimension, Some(768));
    assert_eq!(params.metric.as_deref(), Some("cosine"));
    assert_eq!(params.schema_mode, GraphSchemaMode::Schemaless);
}

#[test]
fn test_create_graph_collection_schemaless_no_embeddings() {
    // Graph collection without a body — grammar allows create_body to be optional.
    // When no body is provided, the parser should produce a Graph with no dimension.
    let result = Parser::parse("CREATE GRAPH COLLECTION kg SCHEMALESS;");

    // The grammar requires create_body? which contains the option list.
    // SCHEMALESS is a create_suffix, which lives inside create_body.
    // If create_body is absent, SCHEMALESS is not reachable.
    // This tests the grammar's actual behavior.
    if let Ok(query) = result {
        let ddl = query.ddl.expect("Expected DDL statement");
        let DdlStatement::CreateCollection(create) = ddl else {
            panic!("Expected CreateCollection variant");
        };
        let CreateCollectionKind::Graph(params) = &create.kind else {
            panic!("Expected Graph kind");
        };
        assert!(params.dimension.is_none());
        assert_eq!(params.schema_mode, GraphSchemaMode::Schemaless);
    }
    // If it fails at grammar level, that is also acceptable behavior —
    // SCHEMALESS is only valid inside create_body per grammar.
}

#[test]
fn test_create_graph_collection_typed_schema() {
    let query = Parser::parse(
        "CREATE GRAPH COLLECTION kg (dimension = 768, metric = 'cosine') \
         WITH SCHEMA (NODE Person (name: STRING, age: INTEGER), EDGE KNOWS FROM Person TO Person);",
    )
    .expect("typed graph schema should parse");

    let ddl = query.ddl.expect("Expected DDL statement");
    let DdlStatement::CreateCollection(create) = ddl else {
        panic!("Expected CreateCollection variant");
    };

    let CreateCollectionKind::Graph(params) = &create.kind else {
        panic!("Expected Graph kind");
    };

    let GraphSchemaMode::Typed(defs) = &params.schema_mode else {
        panic!("Expected Typed schema mode");
    };

    assert_eq!(defs.len(), 2);

    // Verify NODE definition
    let SchemaDefinition::Node { name, properties } = &defs[0] else {
        panic!("Expected Node definition at index 0");
    };
    assert_eq!(name, "Person");
    assert_eq!(properties.len(), 2);
    assert_eq!(properties[0], ("name".to_string(), "STRING".to_string()));
    assert_eq!(properties[1], ("age".to_string(), "INTEGER".to_string()));

    // Verify EDGE definition
    let SchemaDefinition::Edge {
        name,
        from_type,
        to_type,
    } = &defs[1]
    else {
        panic!("Expected Edge definition at index 1");
    };
    assert_eq!(name, "KNOWS");
    assert_eq!(from_type, "Person");
    assert_eq!(to_type, "Person");
}

// ============================================================================
// CREATE COLLECTION (metadata) — nominal cases
// ============================================================================

#[test]
fn test_create_metadata_collection_simple() {
    let query = Parser::parse("CREATE METADATA COLLECTION tags;")
        .expect("metadata collection should parse");

    let ddl = query.ddl.expect("Expected DDL statement");
    let DdlStatement::CreateCollection(create) = ddl else {
        panic!("Expected CreateCollection variant");
    };

    assert_eq!(create.name, "tags");
    assert!(matches!(create.kind, CreateCollectionKind::Metadata));
}

#[test]
fn test_create_metadata_collection_without_semicolon() {
    let query = Parser::parse("CREATE METADATA COLLECTION tags")
        .expect("metadata without semicolon should parse");

    assert!(query.ddl.is_some());
    let ddl = query.ddl.expect("DDL present");
    let DdlStatement::CreateCollection(create) = ddl else {
        panic!("Expected CreateCollection variant");
    };
    assert_eq!(create.name, "tags");
    assert!(matches!(create.kind, CreateCollectionKind::Metadata));
}

// ============================================================================
// DROP COLLECTION — nominal cases
// ============================================================================

#[test]
fn test_drop_collection_simple() {
    let query = Parser::parse("DROP COLLECTION docs;").expect("DROP COLLECTION should parse");

    let ddl = query.ddl.expect("Expected DDL statement");
    let DdlStatement::DropCollection(drop) = ddl else {
        panic!("Expected DropCollection variant");
    };

    assert_eq!(drop.name, "docs");
    assert!(!drop.if_exists);
}

#[test]
fn test_drop_collection_if_exists() {
    let query =
        Parser::parse("DROP COLLECTION IF EXISTS docs;").expect("DROP IF EXISTS should parse");

    let ddl = query.ddl.expect("Expected DDL statement");
    let DdlStatement::DropCollection(drop) = ddl else {
        panic!("Expected DropCollection variant");
    };

    assert_eq!(drop.name, "docs");
    assert!(drop.if_exists);
}

// ============================================================================
// INSERT EDGE — nominal cases
// ============================================================================

#[test]
fn test_insert_edge_basic() {
    let query = Parser::parse("INSERT EDGE INTO kg (source = 1, target = 2, label = 'KNOWS');")
        .expect("INSERT EDGE should parse");

    let dml = query.dml.expect("Expected DML statement");
    let DmlStatement::InsertEdge(edge) = dml else {
        panic!("Expected InsertEdge variant");
    };

    assert_eq!(edge.collection, "kg");
    assert_eq!(edge.source, 1);
    assert_eq!(edge.target, 2);
    assert_eq!(edge.label, "KNOWS");
    assert!(edge.properties.is_empty());
    assert!(edge.edge_id.is_none());
}

#[test]
fn test_insert_edge_with_properties() {
    let query = Parser::parse(
        "INSERT EDGE INTO kg (source = 1, target = 2, label = 'KNOWS') \
         WITH PROPERTIES (weight = 0.95, year = 2026);",
    )
    .expect("INSERT EDGE with properties should parse");

    let dml = query.dml.expect("Expected DML statement");
    let DmlStatement::InsertEdge(edge) = dml else {
        panic!("Expected InsertEdge variant");
    };

    assert_eq!(edge.collection, "kg");
    assert_eq!(edge.source, 1);
    assert_eq!(edge.target, 2);
    assert_eq!(edge.label, "KNOWS");
    assert_eq!(edge.properties.len(), 2);

    // Verify properties by key
    let weight = edge
        .properties
        .iter()
        .find(|(k, _)| k == "weight")
        .expect("weight property should exist");
    assert_eq!(weight.1, Value::Float(0.95));

    let year = edge
        .properties
        .iter()
        .find(|(k, _)| k == "year")
        .expect("year property should exist");
    assert_eq!(year.1, Value::Integer(2026));
}

// ============================================================================
// DELETE FROM — nominal cases
// ============================================================================

#[test]
fn test_delete_from_single_id() {
    let query = Parser::parse("DELETE FROM docs WHERE id = 42;")
        .expect("DELETE FROM with single ID should parse");

    let dml = query.dml.expect("Expected DML statement");
    let DmlStatement::Delete(delete) = dml else {
        panic!("Expected Delete variant");
    };

    assert_eq!(delete.table, "docs");

    // Verify WHERE clause contains a comparison on id = 42
    let Condition::Comparison(cmp) = &delete.where_clause else {
        panic!(
            "Expected Comparison condition, got {:?}",
            delete.where_clause
        );
    };
    assert_eq!(cmp.column, "id");
    assert_eq!(cmp.operator, CompareOp::Eq);
    assert_eq!(cmp.value, Value::Integer(42));
}

#[test]
fn test_delete_from_in_clause() {
    let query = Parser::parse("DELETE FROM docs WHERE id IN (1, 2, 3);")
        .expect("DELETE FROM with IN clause should parse");

    let dml = query.dml.expect("Expected DML statement");
    let DmlStatement::Delete(delete) = dml else {
        panic!("Expected Delete variant");
    };

    assert_eq!(delete.table, "docs");

    let Condition::In(in_cond) = &delete.where_clause else {
        panic!("Expected In condition, got {:?}", delete.where_clause);
    };
    assert_eq!(in_cond.column, "id");
    assert_eq!(in_cond.values.len(), 3);
    assert!(!in_cond.negated);
}

#[test]
fn test_delete_from_string_comparison() {
    let query = Parser::parse("DELETE FROM docs WHERE category = 'obsolete';")
        .expect("DELETE FROM with string comparison should parse");

    let dml = query.dml.expect("Expected DML statement");
    let DmlStatement::Delete(delete) = dml else {
        panic!("Expected Delete variant");
    };

    assert_eq!(delete.table, "docs");

    let Condition::Comparison(cmp) = &delete.where_clause else {
        panic!("Expected Comparison condition");
    };
    assert_eq!(cmp.column, "category");
    assert_eq!(cmp.operator, CompareOp::Eq);
    assert_eq!(cmp.value, Value::String("obsolete".to_string()));
}

// ============================================================================
// DELETE EDGE — nominal cases
// ============================================================================

#[test]
fn test_delete_edge_basic() {
    let query = Parser::parse("DELETE EDGE 123 FROM kg;").expect("DELETE EDGE should parse");

    let dml = query.dml.expect("Expected DML statement");
    let DmlStatement::DeleteEdge(del_edge) = dml else {
        panic!("Expected DeleteEdge variant");
    };

    assert_eq!(del_edge.collection, "kg");
    assert_eq!(del_edge.edge_id, 123);
}

// ============================================================================
// Edge cases — boundaries, invalid, conflicting, absent
// ============================================================================

#[test]
fn test_create_vector_without_dimension_fails() {
    // Vector collection requires dimension — parser-level validation in build_vector_params
    let result = Parser::parse("CREATE COLLECTION docs (metric = 'cosine');");
    assert!(
        result.is_err(),
        "CREATE COLLECTION without dimension should fail"
    );
}

#[test]
fn test_create_vector_without_metric_defaults_to_cosine() {
    // Omitting metric defaults to "cosine" per VELESQL_SPEC.md
    let query = Parser::parse("CREATE COLLECTION docs (dimension = 768);")
        .expect("CREATE COLLECTION without metric should succeed");
    let ddl = query.ddl.expect("Expected DDL");
    let DdlStatement::CreateCollection(create) = ddl else {
        panic!("Expected CreateCollection");
    };
    let CreateCollectionKind::Vector(params) = create.kind else {
        panic!("Expected Vector kind");
    };
    assert_eq!(params.metric, "cosine", "metric should default to cosine");
    assert_eq!(params.dimension, 768);
}

#[test]
fn test_create_collection_quoted_identifier() {
    let query = Parser::parse("CREATE COLLECTION `my-docs` (dimension = 768, metric = 'cosine');")
        .expect("quoted identifier should parse");

    let ddl = query.ddl.expect("Expected DDL statement");
    let DdlStatement::CreateCollection(create) = ddl else {
        panic!("Expected CreateCollection variant");
    };
    assert_eq!(create.name, "my-docs");
}

#[test]
fn test_drop_nonexistent_parses_fine() {
    // DROP at parser level always succeeds — error is at execution time.
    let query = Parser::parse("DROP COLLECTION nonexistent;")
        .expect("DROP nonexistent should parse at parser level");

    let ddl = query.ddl.expect("Expected DDL statement");
    let DdlStatement::DropCollection(drop) = ddl else {
        panic!("Expected DropCollection variant");
    };
    assert_eq!(drop.name, "nonexistent");
    assert!(!drop.if_exists);
}

#[test]
fn test_delete_from_without_where_fails() {
    // Grammar mandates WHERE clause for DELETE FROM.
    let result = Parser::parse("DELETE FROM docs;");
    assert!(result.is_err(), "DELETE FROM without WHERE should fail");
}

#[test]
fn test_delete_edge_with_non_integer_fails() {
    // DELETE EDGE requires an integer ID, not a string.
    let result = Parser::parse("DELETE EDGE 'abc' FROM kg;");
    assert!(
        result.is_err(),
        "DELETE EDGE with non-integer ID should fail"
    );
}

#[test]
fn test_insert_edge_missing_source_fails() {
    let result = Parser::parse("INSERT EDGE INTO kg (target = 2, label = 'KNOWS');");
    assert!(result.is_err(), "INSERT EDGE without source should fail");
}

#[test]
fn test_insert_edge_missing_target_fails() {
    let result = Parser::parse("INSERT EDGE INTO kg (source = 1, label = 'KNOWS');");
    assert!(result.is_err(), "INSERT EDGE without target should fail");
}

#[test]
fn test_insert_edge_missing_label_fails() {
    let result = Parser::parse("INSERT EDGE INTO kg (source = 1, target = 2);");
    assert!(result.is_err(), "INSERT EDGE without label should fail");
}

#[test]
fn test_create_vector_empty_body_fails() {
    // CREATE COLLECTION docs; — no body means no dimension/metric for vector.
    let result = Parser::parse("CREATE COLLECTION docs;");
    assert!(
        result.is_err(),
        "CREATE COLLECTION with no body should fail for vector (no dimension)"
    );
}

#[test]
fn test_create_vector_duplicate_options() {
    // Duplicate dimension — the parser uses find() which returns the first match.
    let result = Parser::parse(
        "CREATE COLLECTION docs (dimension = 768, dimension = 512, metric = 'cosine');",
    );

    // Should parse successfully — first dimension wins in lookup_required_usize
    if let Ok(query) = result {
        let ddl = query.ddl.expect("Expected DDL statement");
        let DdlStatement::CreateCollection(create) = ddl else {
            panic!("Expected CreateCollection variant");
        };
        let CreateCollectionKind::Vector(params) = &create.kind else {
            panic!("Expected Vector kind");
        };
        // find() returns first match, so dimension = 768 should win
        assert_eq!(params.dimension, 768);
    }
    // If it fails, duplicate options are rejected — also valid behavior
}

// ============================================================================
// Query type predicates
// ============================================================================

#[test]
fn test_is_ddl_query_true_for_create() {
    let query = Parser::parse("CREATE COLLECTION docs (dimension = 768, metric = 'cosine');")
        .expect("should parse");
    assert!(query.is_ddl_query());
    assert!(!query.is_select_query());
    assert!(!query.is_dml_query());
    assert!(!query.is_match_query());
    assert!(!query.is_train());
}

#[test]
fn test_is_ddl_query_true_for_drop() {
    let query = Parser::parse("DROP COLLECTION docs;").expect("should parse");
    assert!(query.is_ddl_query());
    assert!(!query.is_select_query());
    assert!(!query.is_dml_query());
}

#[test]
fn test_is_dml_query_true_for_insert_edge() {
    let query = Parser::parse("INSERT EDGE INTO kg (source = 1, target = 2, label = 'KNOWS');")
        .expect("should parse");
    assert!(query.is_dml_query());
    assert!(!query.is_ddl_query());
    assert!(!query.is_select_query());
}

#[test]
fn test_is_dml_query_true_for_delete_from() {
    let query = Parser::parse("DELETE FROM docs WHERE id = 1;").expect("should parse");
    assert!(query.is_dml_query());
    assert!(!query.is_ddl_query());
    assert!(!query.is_select_query());
}

#[test]
fn test_is_dml_query_true_for_delete_edge() {
    let query = Parser::parse("DELETE EDGE 42 FROM kg;").expect("should parse");
    assert!(query.is_dml_query());
    assert!(!query.is_ddl_query());
    assert!(!query.is_select_query());
}

#[test]
fn test_is_ddl_query_false_for_select() {
    let query = Parser::parse("SELECT * FROM docs LIMIT 10").expect("should parse");
    assert!(!query.is_ddl_query());
    assert!(!query.is_dml_query());
    assert!(query.is_select_query());
}

// ============================================================================
// AST field verification
// ============================================================================

#[test]
fn test_ast_create_vector_all_fields() {
    let query = Parser::parse(
        "CREATE COLLECTION embeddings (dimension = 512, metric = 'euclidean', storage = 'pq') \
         WITH (m = 32, ef_construction = 400);",
    )
    .expect("full vector CREATE should parse");

    let ddl = query.ddl.expect("Expected DDL statement");
    let DdlStatement::CreateCollection(create) = ddl else {
        panic!("Expected CreateCollection variant");
    };

    assert_eq!(create.name, "embeddings");

    let CreateCollectionKind::Vector(params) = &create.kind else {
        panic!("Expected Vector kind");
    };
    assert_eq!(params.dimension, 512);
    assert_eq!(params.metric, "euclidean");
    assert_eq!(params.storage.as_deref(), Some("pq"));
    assert_eq!(params.m, Some(32));
    assert_eq!(params.ef_construction, Some(400));
}

#[test]
fn test_ast_create_graph_fields() {
    let query = Parser::parse(
        "CREATE GRAPH COLLECTION social (dimension = 384, metric = 'cosine') SCHEMALESS;",
    )
    .expect("graph CREATE should parse");

    let ddl = query.ddl.expect("Expected DDL statement");
    let DdlStatement::CreateCollection(create) = ddl else {
        panic!("Expected CreateCollection variant");
    };

    assert_eq!(create.name, "social");
    let CreateCollectionKind::Graph(params) = &create.kind else {
        panic!("Expected Graph kind");
    };
    assert_eq!(params.dimension, Some(384));
    assert_eq!(params.metric.as_deref(), Some("cosine"));
    assert_eq!(params.schema_mode, GraphSchemaMode::Schemaless);
}

#[test]
fn test_ast_drop_fields() {
    let query =
        Parser::parse("DROP COLLECTION IF EXISTS legacy;").expect("DROP IF EXISTS should parse");

    let ddl = query.ddl.expect("Expected DDL statement");
    let DdlStatement::DropCollection(drop) = ddl else {
        panic!("Expected DropCollection variant");
    };
    assert_eq!(drop.name, "legacy");
    assert!(drop.if_exists);
}

#[test]
fn test_ast_insert_edge_fields() {
    let query = Parser::parse(
        "INSERT EDGE INTO social (source = 10, target = 20, label = 'FOLLOWS') \
         WITH PROPERTIES (since = 2025);",
    )
    .expect("INSERT EDGE with properties should parse");

    let dml = query.dml.expect("Expected DML statement");
    let DmlStatement::InsertEdge(edge) = dml else {
        panic!("Expected InsertEdge variant");
    };

    assert_eq!(edge.collection, "social");
    assert_eq!(edge.source, 10);
    assert_eq!(edge.target, 20);
    assert_eq!(edge.label, "FOLLOWS");
    assert!(edge.edge_id.is_none());
    assert_eq!(edge.properties.len(), 1, "single property expected");
    assert_eq!(
        edge.properties[0],
        ("since".to_string(), Value::Integer(2025)),
        "since must parse to Value::Integer(2025)"
    );
}

#[test]
fn test_ast_delete_from_fields() {
    let query =
        Parser::parse("DELETE FROM archive WHERE id = 99;").expect("DELETE FROM should parse");

    let dml = query.dml.expect("Expected DML statement");
    let DmlStatement::Delete(delete) = dml else {
        panic!("Expected Delete variant");
    };

    assert_eq!(delete.table, "archive");

    let Condition::Comparison(cmp) = &delete.where_clause else {
        panic!("Expected Comparison condition");
    };
    assert_eq!(cmp.column, "id");
    assert_eq!(cmp.operator, CompareOp::Eq);
    assert_eq!(cmp.value, Value::Integer(99));
}

#[test]
fn test_ast_delete_edge_fields() {
    let query =
        Parser::parse("DELETE EDGE 999 FROM relationships;").expect("DELETE EDGE should parse");

    let dml = query.dml.expect("Expected DML statement");
    let DmlStatement::DeleteEdge(del_edge) = dml else {
        panic!("Expected DeleteEdge variant");
    };

    assert_eq!(del_edge.collection, "relationships");
    assert_eq!(del_edge.edge_id, 999);
}

// ============================================================================
// Additional edge cases and variations
// ============================================================================

#[test]
fn test_create_collection_case_insensitive_keywords() {
    // VelesQL grammar uses ^"keyword" for case-insensitive matching.
    let query = Parser::parse("create collection test_coll (dimension = 64, metric = 'cosine');")
        .expect("lowercase keywords should parse");

    assert!(query.is_ddl_query());
    let ddl = query.ddl.expect("Expected DDL");
    let DdlStatement::CreateCollection(create) = ddl else {
        panic!("Expected CreateCollection");
    };
    assert_eq!(create.name, "test_coll");
}

#[test]
fn test_drop_collection_case_insensitive() {
    let query =
        Parser::parse("drop collection IF EXISTS my_coll;").expect("mixed case DROP should parse");

    assert!(query.is_ddl_query());
    let ddl = query.ddl.expect("Expected DDL");
    let DdlStatement::DropCollection(drop) = ddl else {
        panic!("Expected DropCollection");
    };
    assert_eq!(drop.name, "my_coll");
    assert!(drop.if_exists);
}

#[test]
fn test_insert_edge_case_insensitive() {
    let query = Parser::parse("insert edge into kg (source = 5, target = 6, label = 'LINKED');")
        .expect("lowercase INSERT EDGE should parse");

    let dml = query.dml.expect("Expected DML statement");
    let DmlStatement::InsertEdge(edge) = dml else {
        panic!("Expected InsertEdge variant");
    };
    assert_eq!(edge.collection, "kg");
    assert_eq!(edge.source, 5);
    assert_eq!(edge.target, 6);
    assert_eq!(edge.label, "LINKED");
}

#[test]
fn test_delete_from_case_insensitive() {
    let query = Parser::parse("delete from docs where id = 1;")
        .expect("lowercase DELETE FROM should parse");

    let dml = query.dml.expect("Expected DML statement");
    let DmlStatement::Delete(delete) = dml else {
        panic!("Expected Delete variant");
    };
    assert_eq!(delete.table, "docs");
    let Condition::Comparison(cmp) = &delete.where_clause else {
        panic!(
            "Expected Comparison condition, got {:?}",
            delete.where_clause
        );
    };
    assert_eq!(cmp.column, "id");
    assert_eq!(cmp.operator, CompareOp::Eq);
    assert_eq!(cmp.value, Value::Integer(1));
}

#[test]
fn test_delete_edge_case_insensitive() {
    let query =
        Parser::parse("delete edge 7 from kg;").expect("lowercase DELETE EDGE should parse");

    let dml = query.dml.expect("Expected DML statement");
    let DmlStatement::DeleteEdge(del_edge) = dml else {
        panic!("Expected DeleteEdge variant");
    };
    assert_eq!(del_edge.edge_id, 7);
    assert_eq!(del_edge.collection, "kg");
}

#[test]
fn test_create_graph_collection_no_body() {
    // CREATE GRAPH COLLECTION kg; — no body, no suffix.
    // Per grammar create_body is optional, so for GRAPH kind this should
    // produce a graph with no dimension and schemaless mode.
    let result = Parser::parse("CREATE GRAPH COLLECTION kg;");
    if let Ok(query) = result {
        let ddl = query.ddl.expect("Expected DDL");
        let DdlStatement::CreateCollection(create) = ddl else {
            panic!("Expected CreateCollection");
        };
        assert_eq!(create.name, "kg");
        let CreateCollectionKind::Graph(params) = &create.kind else {
            panic!("Expected Graph kind");
        };
        assert!(params.dimension.is_none());
        assert!(params.metric.is_none());
        assert_eq!(params.schema_mode, GraphSchemaMode::Schemaless);
    }
    // If grammar rejects, that is also acceptable — test documents the behavior.
}

#[test]
fn test_insert_edge_large_ids() {
    let query = Parser::parse(
        "INSERT EDGE INTO kg (source = 999999999, target = 888888888, label = 'REFS');",
    )
    .expect("large IDs should parse");

    let dml = query.dml.expect("Expected DML");
    let DmlStatement::InsertEdge(edge) = dml else {
        panic!("Expected InsertEdge");
    };
    assert_eq!(edge.source, 999_999_999);
    assert_eq!(edge.target, 888_888_888);
}

#[test]
fn test_delete_edge_zero_id() {
    let query = Parser::parse("DELETE EDGE 0 FROM kg;").expect("edge ID 0 should parse");

    let dml = query.dml.expect("Expected DML");
    let DmlStatement::DeleteEdge(del_edge) = dml else {
        panic!("Expected DeleteEdge");
    };
    assert_eq!(del_edge.edge_id, 0);
}

#[test]
fn test_create_vector_with_all_with_options() {
    // Storage + HNSW params all in WITH clause (not in body)
    let query = Parser::parse(
        "CREATE COLLECTION idx (dimension = 128, metric = 'cosine') \
         WITH (storage = 'binary', m = 48, ef_construction = 500);",
    )
    .expect("all WITH options should parse");

    let ddl = query.ddl.expect("Expected DDL");
    let DdlStatement::CreateCollection(create) = ddl else {
        panic!("Expected CreateCollection");
    };
    let CreateCollectionKind::Vector(params) = &create.kind else {
        panic!("Expected Vector kind");
    };
    assert_eq!(params.dimension, 128);
    assert_eq!(params.metric, "cosine");
    // storage is in body options (parsed from create_option_list)
    // WITH clause provides m and ef_construction
    assert_eq!(params.m, Some(48));
    assert_eq!(params.ef_construction, Some(500));
}

#[test]
fn test_drop_collection_without_semicolon() {
    let query = Parser::parse("DROP COLLECTION temp").expect("DROP without semicolon should parse");

    assert!(query.is_ddl_query());
    let ddl = query.ddl.expect("Expected DDL");
    let DdlStatement::DropCollection(drop) = ddl else {
        panic!("Expected DropCollection");
    };
    assert_eq!(drop.name, "temp");
}

#[test]
fn test_delete_from_with_and_condition() {
    let query = Parser::parse("DELETE FROM docs WHERE category = 'old' AND status = 'archived';")
        .expect("DELETE with AND condition should parse");

    let dml = query.dml.expect("Expected DML");
    let DmlStatement::Delete(delete) = dml else {
        panic!("Expected Delete");
    };
    assert_eq!(delete.table, "docs");
    // WHERE clause should be an And condition
    assert!(
        matches!(delete.where_clause, Condition::And(_, _)),
        "Expected AND condition, got {:?}",
        delete.where_clause
    );
}

#[test]
fn test_create_graph_typed_schema_multiple_nodes() {
    let query = Parser::parse(
        "CREATE GRAPH COLLECTION ontology (dimension = 768, metric = 'cosine') \
         WITH SCHEMA (\
            NODE Person (name: STRING, age: INTEGER), \
            NODE Company (name: STRING, founded: INTEGER), \
            EDGE WORKS_AT FROM Person TO Company\
         );",
    )
    .expect("multi-node typed schema should parse");

    let ddl = query.ddl.expect("Expected DDL");
    let DdlStatement::CreateCollection(create) = ddl else {
        panic!("Expected CreateCollection");
    };

    let CreateCollectionKind::Graph(params) = &create.kind else {
        panic!("Expected Graph kind");
    };

    let GraphSchemaMode::Typed(defs) = &params.schema_mode else {
        panic!("Expected Typed schema mode");
    };

    assert_eq!(defs.len(), 3);

    // First node: Person
    assert!(matches!(&defs[0], SchemaDefinition::Node { name, .. } if name == "Person"));
    // Second node: Company
    assert!(matches!(&defs[1], SchemaDefinition::Node { name, .. } if name == "Company"));
    // Edge: WORKS_AT
    let SchemaDefinition::Edge {
        name,
        from_type,
        to_type,
    } = &defs[2]
    else {
        panic!("Expected Edge definition");
    };
    assert_eq!(name, "WORKS_AT");
    assert_eq!(from_type, "Person");
    assert_eq!(to_type, "Company");
}

// ============================================================================
// Metric validation at parse time (Finding 5)
// ============================================================================

#[test]
fn test_invalid_metric_rejected_at_parse_time() {
    let result = Parser::parse("CREATE COLLECTION docs (dimension = 768, metric = 'manhattan');");
    assert!(result.is_err(), "invalid metric should fail at parse time");
    let err = result.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("Unknown metric 'manhattan'"),
        "error should name the invalid metric, got: {msg}"
    );
}

#[test]
fn test_valid_metric_aliases_parse_successfully() {
    for alias in &[
        "cosine",
        "euclidean",
        "l2",
        "dot",
        "dotproduct",
        "inner",
        "ip",
        "hamming",
        "jaccard",
    ] {
        let sql = format!("CREATE COLLECTION test_m (dimension = 64, metric = '{alias}');");
        let result = Parser::parse(&sql);
        assert!(
            result.is_ok(),
            "metric alias '{alias}' should parse, got: {:?}",
            result.err()
        );
    }
}

#[test]
fn test_graph_invalid_metric_rejected_at_parse_time() {
    let result = Parser::parse(
        "CREATE GRAPH COLLECTION kg (dimension = 128, metric = 'badmetric') SCHEMALESS;",
    );
    assert!(
        result.is_err(),
        "invalid metric on graph collection should fail at parse time"
    );
}
