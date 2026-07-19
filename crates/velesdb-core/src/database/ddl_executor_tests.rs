//! Tests for DDL and extended DML executor (Phase 5).

use super::*;
use crate::velesql::{
    AlterCollectionStatement, CompareOp, Comparison, Condition, CreateCollectionKind,
    CreateCollectionStatement, DdlStatement, DeleteEdgeStatement, DeleteStatement, DmlStatement,
    DropCollectionStatement, GraphCollectionParams, GraphSchemaMode, InCondition,
    InsertEdgeStatement, Query, SchemaDefinition, SelectEdgesStatement, Value,
    VectorCollectionParams,
};
use crate::wire::hash_edge_id;
use tempfile::tempdir;

// =========================================================================
// Helper: execute DDL via Database::execute_query
// =========================================================================

fn execute_ddl(db: &Database, ddl: DdlStatement) -> crate::Result<Vec<crate::SearchResult>> {
    let query = Query::new_ddl(ddl);
    db.execute_query(&query, &std::collections::HashMap::new())
}

fn execute_dml(db: &Database, dml: DmlStatement) -> crate::Result<Vec<crate::SearchResult>> {
    let query = Query::new_dml(dml);
    db.execute_query(&query, &std::collections::HashMap::new())
}

// =========================================================================
// CREATE VECTOR COLLECTION
// =========================================================================

#[test]
fn test_create_vector_collection_basic() {
    let dir = tempdir().expect("tempdir");
    let db = Database::open(dir.path()).expect("open");

    let ddl = DdlStatement::CreateCollection(CreateCollectionStatement {
        name: "docs".to_string(),
        kind: CreateCollectionKind::Vector(VectorCollectionParams {
            dimension: 128,
            metric: "cosine".to_string(),
            storage: None,
            m: None,
            ef_construction: None,
        }),
    });

    let result = execute_ddl(&db, ddl).expect("create");
    assert!(result.is_empty(), "DDL returns empty result set");
    assert!(db.list_collections().contains(&"docs".to_string()));

    let vc = db.get_vector_collection("docs").expect("get");
    assert_eq!(vc.dimension(), 128);
    assert_eq!(vc.metric(), crate::DistanceMetric::Cosine);
}

#[test]
fn test_create_vector_collection_with_hnsw_params() {
    let dir = tempdir().expect("tempdir");
    let db = Database::open(dir.path()).expect("open");

    let ddl = DdlStatement::CreateCollection(CreateCollectionStatement {
        name: "hnsw_coll".to_string(),
        kind: CreateCollectionKind::Vector(VectorCollectionParams {
            dimension: 64,
            metric: "euclidean".to_string(),
            storage: Some("sq8".to_string()),
            m: Some(32),
            ef_construction: Some(200),
        }),
    });

    let result = execute_ddl(&db, ddl).expect("create");
    assert!(result.is_empty());
    assert!(db.list_collections().contains(&"hnsw_coll".to_string()));

    let vc = db.get_vector_collection("hnsw_coll").expect("get");
    assert_eq!(vc.dimension(), 64);
    assert_eq!(vc.metric(), crate::DistanceMetric::Euclidean);
}

#[test]
fn test_create_vector_collection_dot_metric() {
    let dir = tempdir().expect("tempdir");
    let db = Database::open(dir.path()).expect("open");

    let ddl = DdlStatement::CreateCollection(CreateCollectionStatement {
        name: "dot_coll".to_string(),
        kind: CreateCollectionKind::Vector(VectorCollectionParams {
            dimension: 32,
            metric: "dotproduct".to_string(),
            storage: None,
            m: None,
            ef_construction: None,
        }),
    });

    execute_ddl(&db, ddl).expect("create");
    let vc = db.get_vector_collection("dot_coll").expect("get");
    assert_eq!(vc.metric(), crate::DistanceMetric::DotProduct);
}

// =========================================================================
// CREATE GRAPH COLLECTION
// =========================================================================

#[test]
fn test_create_graph_collection_schemaless() {
    let dir = tempdir().expect("tempdir");
    let db = Database::open(dir.path()).expect("open");

    let ddl = DdlStatement::CreateCollection(CreateCollectionStatement {
        name: "kg".to_string(),
        kind: CreateCollectionKind::Graph(GraphCollectionParams {
            dimension: None,
            metric: None,
            schema_mode: GraphSchemaMode::Schemaless,
        }),
    });

    let result = execute_ddl(&db, ddl).expect("create");
    assert!(result.is_empty());

    let gc = db.get_graph_collection("kg").expect("get");
    assert_eq!(gc.name(), "kg");
    assert!(gc.schema().is_schemaless());
}

#[test]
fn test_create_graph_collection_with_embeddings() {
    let dir = tempdir().expect("tempdir");
    let db = Database::open(dir.path()).expect("open");

    let ddl = DdlStatement::CreateCollection(CreateCollectionStatement {
        name: "kg_embed".to_string(),
        kind: CreateCollectionKind::Graph(GraphCollectionParams {
            dimension: Some(256),
            metric: Some("cosine".to_string()),
            schema_mode: GraphSchemaMode::Schemaless,
        }),
    });

    execute_ddl(&db, ddl).expect("create");
    let gc = db.get_graph_collection("kg_embed").expect("get");
    assert!(gc.has_embeddings());
}

#[test]
fn test_create_graph_collection_typed_schema() {
    let dir = tempdir().expect("tempdir");
    let db = Database::open(dir.path()).expect("open");

    let ddl = DdlStatement::CreateCollection(CreateCollectionStatement {
        name: "social".to_string(),
        kind: CreateCollectionKind::Graph(GraphCollectionParams {
            dimension: None,
            metric: None,
            schema_mode: GraphSchemaMode::Typed(vec![
                SchemaDefinition::Node {
                    name: "Person".to_string(),
                    properties: vec![
                        ("name".to_string(), "STRING".to_string()),
                        ("age".to_string(), "INTEGER".to_string()),
                    ],
                },
                SchemaDefinition::Edge {
                    name: "KNOWS".to_string(),
                    from_type: "Person".to_string(),
                    to_type: "Person".to_string(),
                },
            ]),
        }),
    });

    execute_ddl(&db, ddl).expect("create");
    let gc = db.get_graph_collection("social").expect("get");
    assert!(!gc.schema().is_schemaless());
    assert!(gc.schema().has_node_type("Person"));
    assert!(gc.schema().has_edge_type("KNOWS"));
}

// =========================================================================
// CREATE METADATA COLLECTION
// =========================================================================

#[test]
fn test_create_metadata_collection() {
    let dir = tempdir().expect("tempdir");
    let db = Database::open(dir.path()).expect("open");

    let ddl = DdlStatement::CreateCollection(CreateCollectionStatement {
        name: "tags".to_string(),
        kind: CreateCollectionKind::Metadata,
    });

    let result = execute_ddl(&db, ddl).expect("create");
    assert!(result.is_empty());

    let mc = db.get_metadata_collection("tags").expect("get");
    assert_eq!(mc.name(), "tags");
}

// =========================================================================
// CREATE DUPLICATE — error
// =========================================================================

#[test]
fn test_create_duplicate_returns_error() {
    let dir = tempdir().expect("tempdir");
    let db = Database::open(dir.path()).expect("open");

    let ddl = DdlStatement::CreateCollection(CreateCollectionStatement {
        name: "dupe".to_string(),
        kind: CreateCollectionKind::Vector(VectorCollectionParams {
            dimension: 64,
            metric: "cosine".to_string(),
            storage: None,
            m: None,
            ef_construction: None,
        }),
    });

    execute_ddl(&db, ddl.clone()).expect("first create");
    let err = execute_ddl(&db, ddl).expect_err("duplicate");
    assert!(
        matches!(err, crate::Error::CollectionExists(_)),
        "Expected CollectionExists, got: {err:?}"
    );
}

// =========================================================================
// DROP COLLECTION
// =========================================================================

#[test]
fn test_drop_existing_collection() {
    let dir = tempdir().expect("tempdir");
    let db = Database::open(dir.path()).expect("open");

    // Create first.
    let create = DdlStatement::CreateCollection(CreateCollectionStatement {
        name: "to_drop".to_string(),
        kind: CreateCollectionKind::Metadata,
    });
    execute_ddl(&db, create).expect("create");
    assert!(db.list_collections().contains(&"to_drop".to_string()));

    // Drop.
    let drop = DdlStatement::DropCollection(DropCollectionStatement {
        name: "to_drop".to_string(),
        if_exists: false,
    });
    let result = execute_ddl(&db, drop).expect("drop");
    assert!(result.is_empty());
    assert!(!db.list_collections().contains(&"to_drop".to_string()));
}

#[test]
fn test_drop_nonexistent_returns_error() {
    let dir = tempdir().expect("tempdir");
    let db = Database::open(dir.path()).expect("open");

    let drop = DdlStatement::DropCollection(DropCollectionStatement {
        name: "ghost".to_string(),
        if_exists: false,
    });
    let err = execute_ddl(&db, drop).expect_err("should fail");
    assert!(
        matches!(err, crate::Error::CollectionNotFound(_)),
        "Expected CollectionNotFound, got: {err:?}"
    );
}

#[test]
fn test_drop_if_exists_nonexistent_succeeds() {
    let dir = tempdir().expect("tempdir");
    let db = Database::open(dir.path()).expect("open");

    let drop = DdlStatement::DropCollection(DropCollectionStatement {
        name: "phantom".to_string(),
        if_exists: true,
    });
    let result = execute_ddl(&db, drop).expect("should succeed");
    assert!(result.is_empty());
}

// =========================================================================
// INSERT EDGE
// =========================================================================

#[test]
fn test_insert_edge_into_graph() {
    let dir = tempdir().expect("tempdir");
    let db = Database::open(dir.path()).expect("open");

    // Create graph collection first.
    let create = DdlStatement::CreateCollection(CreateCollectionStatement {
        name: "edges_g".to_string(),
        kind: CreateCollectionKind::Graph(GraphCollectionParams {
            dimension: None,
            metric: None,
            schema_mode: GraphSchemaMode::Schemaless,
        }),
    });
    execute_ddl(&db, create).expect("create graph");

    let gc = db.get_graph_collection("edges_g").expect("get graph");
    for id in [100, 200] {
        gc.upsert_node_payload(id, &serde_json::json!({}))
            .expect("store node");
    }

    let dml = DmlStatement::InsertEdge(InsertEdgeStatement {
        collection: "edges_g".to_string(),
        edge_id: Some(42),
        source: 100,
        target: 200,
        label: "KNOWS".to_string(),
        properties: Vec::new(),
    });

    let result = execute_dml(&db, dml).expect("insert edge");
    assert!(result.is_empty());

    let gc = db.get_graph_collection("edges_g").expect("get graph");
    assert_eq!(gc.edge_count(), 1);
    let edges = gc.get_edges(Some("KNOWS"));
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].id(), 42);
    assert_eq!(edges[0].source(), 100);
    assert_eq!(edges[0].target(), 200);
}

#[test]
fn test_insert_edge_with_properties() {
    let dir = tempdir().expect("tempdir");
    let db = Database::open(dir.path()).expect("open");

    let create = DdlStatement::CreateCollection(CreateCollectionStatement {
        name: "props_g".to_string(),
        kind: CreateCollectionKind::Graph(GraphCollectionParams {
            dimension: None,
            metric: None,
            schema_mode: GraphSchemaMode::Schemaless,
        }),
    });
    execute_ddl(&db, create).expect("create graph");

    let gc = db.get_graph_collection("props_g").expect("get graph");
    for id in [1, 2] {
        gc.upsert_node_payload(id, &serde_json::json!({}))
            .expect("store node");
    }

    let dml = DmlStatement::InsertEdge(InsertEdgeStatement {
        collection: "props_g".to_string(),
        edge_id: Some(7),
        source: 1,
        target: 2,
        label: "FRIEND".to_string(),
        properties: vec![
            ("since".to_string(), Value::Integer(2020)),
            ("weight".to_string(), Value::Float(0.9)),
        ],
    });

    execute_dml(&db, dml).expect("insert edge with props");

    let gc = db.get_graph_collection("props_g").expect("get graph");
    let edges = gc.get_edges(None);
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].property("since"), Some(&serde_json::json!(2020)));
    assert_eq!(edges[0].property("weight"), Some(&serde_json::json!(0.9)));
}

// =========================================================================
// DELETE (points by ID)
// =========================================================================

#[test]
fn test_delete_single_id() {
    let dir = tempdir().expect("tempdir");
    let db = Database::open(dir.path()).expect("open");

    db.create_vector_collection("del_coll", 4, crate::DistanceMetric::Cosine)
        .expect("create");
    let vc = db.get_vector_collection("del_coll").expect("get");
    vc.upsert(vec![
        crate::Point::new(1, vec![0.1, 0.2, 0.3, 0.4], None),
        crate::Point::new(2, vec![0.5, 0.6, 0.7, 0.8], None),
    ])
    .expect("upsert");

    let dml = DmlStatement::Delete(DeleteStatement {
        table: "del_coll".to_string(),
        where_clause: Condition::Comparison(Comparison {
            column: "id".to_string(),
            operator: CompareOp::Eq,
            value: Value::Integer(1),
        }),
    });

    let result = execute_dml(&db, dml).expect("delete");
    assert!(result.is_empty());

    let points = vc.get(&[1, 2]);
    assert!(points[0].is_none(), "point 1 should be deleted");
    assert!(points[1].is_some(), "point 2 should remain");
}

#[test]
fn test_delete_multiple_ids_via_in() {
    let dir = tempdir().expect("tempdir");
    let db = Database::open(dir.path()).expect("open");

    db.create_vector_collection("del_multi", 4, crate::DistanceMetric::Cosine)
        .expect("create");
    let vc = db.get_vector_collection("del_multi").expect("get");
    vc.upsert(vec![
        crate::Point::new(10, vec![0.1, 0.2, 0.3, 0.4], None),
        crate::Point::new(20, vec![0.5, 0.6, 0.7, 0.8], None),
        crate::Point::new(30, vec![0.9, 1.0, 1.1, 1.2], None),
    ])
    .expect("upsert");

    let dml = DmlStatement::Delete(DeleteStatement {
        table: "del_multi".to_string(),
        where_clause: Condition::In(InCondition {
            column: "id".to_string(),
            values: vec![Value::Integer(10), Value::Integer(30)],
            negated: false,
        }),
    });

    execute_dml(&db, dml).expect("delete");

    let points = vc.get(&[10, 20, 30]);
    assert!(points[0].is_none(), "point 10 should be deleted");
    assert!(points[1].is_some(), "point 20 should remain");
    assert!(points[2].is_none(), "point 30 should be deleted");
}

// =========================================================================
// DELETE EDGE
// =========================================================================

#[test]
fn test_delete_edge() {
    let dir = tempdir().expect("tempdir");
    let db = Database::open(dir.path()).expect("open");

    let create = DdlStatement::CreateCollection(CreateCollectionStatement {
        name: "del_edge_g".to_string(),
        kind: CreateCollectionKind::Graph(GraphCollectionParams {
            dimension: None,
            metric: None,
            schema_mode: GraphSchemaMode::Schemaless,
        }),
    });
    execute_ddl(&db, create).expect("create graph");

    // Insert two edges.
    let gc = db.get_graph_collection("del_edge_g").expect("get");
    for id in [10, 20, 30] {
        gc.upsert_node_payload(id, &serde_json::json!({}))
            .expect("store node");
    }
    gc.add_edge(crate::GraphEdge::new(1, 10, 20, "A").expect("edge"))
        .expect("add 1");
    gc.add_edge(crate::GraphEdge::new(2, 20, 30, "B").expect("edge"))
        .expect("add 2");
    assert_eq!(gc.edge_count(), 2);

    // Delete edge 1 via DML — returns affected-rows feedback.
    let dml = DmlStatement::DeleteEdge(DeleteEdgeStatement {
        collection: "del_edge_g".to_string(),
        edge_id: 1,
    });
    let results = execute_dml(&db, dml).expect("delete edge");
    assert_eq!(results.len(), 1, "DELETE EDGE should return one result");
    let payload = results[0].point.payload.as_ref().expect("payload");
    assert_eq!(payload["deleted"], true);
    assert_eq!(payload["edge_id"], 1);

    assert_eq!(gc.edge_count(), 1);
    let remaining = gc.get_edges(None);
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id(), 2);
}

#[test]
fn test_delete_edge_nonexistent_reports_false() {
    let dir = tempdir().expect("tempdir");
    let db = Database::open(dir.path()).expect("open");

    let create = DdlStatement::CreateCollection(CreateCollectionStatement {
        name: "del_edge_noop".to_string(),
        kind: CreateCollectionKind::Graph(GraphCollectionParams {
            dimension: None,
            metric: None,
            schema_mode: GraphSchemaMode::Schemaless,
        }),
    });
    execute_ddl(&db, create).expect("create graph");

    // Delete nonexistent edge — should succeed but report deleted: false.
    let dml = DmlStatement::DeleteEdge(DeleteEdgeStatement {
        collection: "del_edge_noop".to_string(),
        edge_id: 999,
    });
    let results = execute_dml(&db, dml).expect("delete edge noop");
    assert_eq!(results.len(), 1);
    let payload = results[0].point.payload.as_ref().expect("payload");
    assert_eq!(payload["deleted"], false);
    assert_eq!(payload["edge_id"], 999);
}

// =========================================================================
// Error edge cases
// =========================================================================

#[test]
fn test_create_with_invalid_metric_returns_error() {
    let dir = tempdir().expect("tempdir");
    let db = Database::open(dir.path()).expect("open");

    let ddl = DdlStatement::CreateCollection(CreateCollectionStatement {
        name: "bad_metric".to_string(),
        kind: CreateCollectionKind::Vector(VectorCollectionParams {
            dimension: 64,
            metric: "manhattan".to_string(),
            storage: None,
            m: None,
            ef_construction: None,
        }),
    });

    let err = execute_ddl(&db, ddl).expect_err("invalid metric");
    assert!(
        matches!(err, crate::Error::Query(_)),
        "Expected Query error, got: {err:?}"
    );
}

#[test]
fn test_insert_edge_into_nonexistent_collection() {
    let dir = tempdir().expect("tempdir");
    let db = Database::open(dir.path()).expect("open");

    let dml = DmlStatement::InsertEdge(InsertEdgeStatement {
        collection: "no_such_graph".to_string(),
        edge_id: Some(1),
        source: 1,
        target: 2,
        label: "REL".to_string(),
        properties: Vec::new(),
    });

    let err = execute_dml(&db, dml).expect_err("should fail");
    assert!(
        matches!(err, crate::Error::CollectionNotFound(_)),
        "Expected CollectionNotFound, got: {err:?}"
    );
}

#[test]
fn test_delete_with_unsupported_where_returns_error() {
    let dir = tempdir().expect("tempdir");
    let db = Database::open(dir.path()).expect("open");

    db.create_vector_collection("del_bad", 4, crate::DistanceMetric::Cosine)
        .expect("create");

    let dml = DmlStatement::Delete(DeleteStatement {
        table: "del_bad".to_string(),
        where_clause: Condition::Comparison(Comparison {
            column: "name".to_string(),
            operator: CompareOp::Eq,
            value: Value::String("test".to_string()),
        }),
    });

    let err = execute_dml(&db, dml).expect_err("should fail");
    assert!(
        matches!(err, crate::Error::Query(_)),
        "Expected Query error for unsupported WHERE, got: {err:?}"
    );
}

#[test]
fn test_insert_edge_auto_generates_id_when_none() {
    let dir = tempdir().expect("tempdir");
    let db = Database::open(dir.path()).expect("open");

    let create = DdlStatement::CreateCollection(CreateCollectionStatement {
        name: "autoid_g".to_string(),
        kind: CreateCollectionKind::Graph(GraphCollectionParams {
            dimension: None,
            metric: None,
            schema_mode: GraphSchemaMode::Schemaless,
        }),
    });
    execute_ddl(&db, create).expect("create graph");

    let gc = db.get_graph_collection("autoid_g").expect("get");
    for id in [1, 2] {
        gc.upsert_node_payload(id, &serde_json::json!({}))
            .expect("store node");
    }

    let dml = DmlStatement::InsertEdge(InsertEdgeStatement {
        collection: "autoid_g".to_string(),
        edge_id: None,
        source: 1,
        target: 2,
        label: "LINKED".to_string(),
        properties: Vec::new(),
    });

    execute_dml(&db, dml).expect("insert edge with auto ID");

    let gc = db.get_graph_collection("autoid_g").expect("get");
    assert_eq!(gc.edge_count(), 1);

    let edges = gc.get_edges(None);
    assert_eq!(edges.len(), 1);
    assert_eq!(
        edges[0].id(),
        hash_edge_id(1, 2, "LINKED"),
        "auto-generated ID must be the FNV-1a hash of (source, target, label)"
    );
    assert_eq!(edges[0].source(), 1);
    assert_eq!(edges[0].target(), 2);
}

// =========================================================================
// RBAC observer hooks
// =========================================================================

/// Observer that rejects all DDL and DML mutation requests.
struct RejectingObserver;

impl crate::observer::DatabaseObserver for RejectingObserver {
    fn on_ddl_request(&self, operation: &str, collection_name: &str) -> crate::Result<()> {
        Err(crate::Error::Query(format!(
            "RBAC: {operation} on '{collection_name}' denied"
        )))
    }

    fn on_dml_mutation_request(&self, operation: &str, collection_name: &str) -> crate::Result<()> {
        Err(crate::Error::Query(format!(
            "RBAC: {operation} on '{collection_name}' denied"
        )))
    }
}

/// Observer with default implementations — allows everything.
struct DefaultObserver;

impl crate::observer::DatabaseObserver for DefaultObserver {}

#[test]
fn test_default_observer_allows_ddl() {
    let dir = tempdir().expect("tempdir");
    let observer: std::sync::Arc<dyn crate::observer::DatabaseObserver> =
        std::sync::Arc::new(DefaultObserver);
    let db = Database::open_with_observer(dir.path(), observer).expect("open");

    let ddl = DdlStatement::CreateCollection(CreateCollectionStatement {
        name: "allowed".to_string(),
        kind: CreateCollectionKind::Metadata,
    });

    execute_ddl(&db, ddl).expect("default observer should allow DDL");
    assert!(db.list_collections().contains(&"allowed".to_string()));
}

#[test]
fn test_rejecting_observer_blocks_create() {
    let dir = tempdir().expect("tempdir");
    let observer: std::sync::Arc<dyn crate::observer::DatabaseObserver> =
        std::sync::Arc::new(RejectingObserver);
    let db = Database::open_with_observer(dir.path(), observer).expect("open");

    let ddl = DdlStatement::CreateCollection(CreateCollectionStatement {
        name: "blocked".to_string(),
        kind: CreateCollectionKind::Metadata,
    });

    let err = execute_ddl(&db, ddl).expect_err("should be rejected");
    let msg = err.to_string();
    assert!(
        msg.contains("RBAC: CREATE on 'blocked' denied"),
        "Unexpected error: {msg}"
    );
    assert!(
        !db.list_collections().contains(&"blocked".to_string()),
        "Collection should not have been created"
    );
}

#[test]
fn test_rejecting_observer_blocks_drop() {
    let dir = tempdir().expect("tempdir");
    let db = Database::open(dir.path()).expect("open");

    // Create without observer so collection exists.
    let create = DdlStatement::CreateCollection(CreateCollectionStatement {
        name: "to_protect".to_string(),
        kind: CreateCollectionKind::Metadata,
    });
    execute_ddl(&db, create).expect("create");
    drop(db);

    // Re-open with rejecting observer.
    let observer: std::sync::Arc<dyn crate::observer::DatabaseObserver> =
        std::sync::Arc::new(RejectingObserver);
    let db = Database::open_with_observer(dir.path(), observer).expect("reopen");

    let drop_ddl = DdlStatement::DropCollection(DropCollectionStatement {
        name: "to_protect".to_string(),
        if_exists: false,
    });

    let err = execute_ddl(&db, drop_ddl).expect_err("should be rejected");
    let msg = err.to_string();
    assert!(
        msg.contains("RBAC: DROP on 'to_protect' denied"),
        "Unexpected error: {msg}"
    );
    assert!(
        db.list_collections().contains(&"to_protect".to_string()),
        "Collection should still exist"
    );
}

#[test]
fn test_rejecting_observer_blocks_insert_edge() {
    let dir = tempdir().expect("tempdir");
    let observer: std::sync::Arc<dyn crate::observer::DatabaseObserver> =
        std::sync::Arc::new(RejectingObserver);
    let db = Database::open_with_observer(dir.path(), observer).expect("open");

    let dml = DmlStatement::InsertEdge(InsertEdgeStatement {
        collection: "some_graph".to_string(),
        edge_id: Some(1),
        source: 10,
        target: 20,
        label: "REL".to_string(),
        properties: Vec::new(),
    });

    let err = execute_dml(&db, dml).expect_err("should be rejected");
    let msg = err.to_string();
    assert!(
        msg.contains("RBAC: INSERT_EDGE on 'some_graph' denied"),
        "Unexpected error: {msg}"
    );
}

#[test]
fn test_rejecting_observer_blocks_delete() {
    let dir = tempdir().expect("tempdir");
    let observer: std::sync::Arc<dyn crate::observer::DatabaseObserver> =
        std::sync::Arc::new(RejectingObserver);
    let db = Database::open_with_observer(dir.path(), observer).expect("open");

    let dml = DmlStatement::Delete(DeleteStatement {
        table: "some_coll".to_string(),
        where_clause: Condition::Comparison(Comparison {
            column: "id".to_string(),
            operator: CompareOp::Eq,
            value: Value::Integer(1),
        }),
    });

    let err = execute_dml(&db, dml).expect_err("should be rejected");
    let msg = err.to_string();
    assert!(
        msg.contains("RBAC: DELETE on 'some_coll' denied"),
        "Unexpected error: {msg}"
    );
}

#[test]
fn test_rejecting_observer_blocks_delete_edge() {
    let dir = tempdir().expect("tempdir");
    let observer: std::sync::Arc<dyn crate::observer::DatabaseObserver> =
        std::sync::Arc::new(RejectingObserver);
    let db = Database::open_with_observer(dir.path(), observer).expect("open");

    let dml = DmlStatement::DeleteEdge(DeleteEdgeStatement {
        collection: "some_graph".to_string(),
        edge_id: 42,
    });

    let err = execute_dml(&db, dml).expect_err("should be rejected");
    let msg = err.to_string();
    assert!(
        msg.contains("RBAC: DELETE_EDGE on 'some_graph' denied"),
        "Unexpected error: {msg}"
    );
}

#[test]
fn test_default_observer_allows_dml_mutations() {
    let dir = tempdir().expect("tempdir");
    let observer: std::sync::Arc<dyn crate::observer::DatabaseObserver> =
        std::sync::Arc::new(DefaultObserver);
    let db = Database::open_with_observer(dir.path(), observer).expect("open");

    // Create a graph collection for the INSERT EDGE test.
    let create = DdlStatement::CreateCollection(CreateCollectionStatement {
        name: "dml_graph".to_string(),
        kind: CreateCollectionKind::Graph(GraphCollectionParams {
            dimension: None,
            metric: None,
            schema_mode: GraphSchemaMode::Schemaless,
        }),
    });
    execute_ddl(&db, create).expect("create graph");

    let gc = db.get_graph_collection("dml_graph").expect("get");
    for id in [10, 20] {
        gc.upsert_node_payload(id, &serde_json::json!({}))
            .expect("store node");
    }

    // INSERT EDGE should succeed.
    let dml = DmlStatement::InsertEdge(InsertEdgeStatement {
        collection: "dml_graph".to_string(),
        edge_id: Some(1),
        source: 10,
        target: 20,
        label: "KNOWS".to_string(),
        properties: Vec::new(),
    });
    execute_dml(&db, dml).expect("default observer should allow INSERT EDGE");

    let gc = db.get_graph_collection("dml_graph").expect("get");
    assert_eq!(gc.edge_count(), 1);
}

// =========================================================================
// hash_edge_id — collision resistance
// =========================================================================

#[test]
fn test_hash_edge_id_collision_resistance() {
    // Same source/target, different labels should produce different IDs.
    let id1 = hash_edge_id(1, 2, "KNOWS");
    let id2 = hash_edge_id(1, 2, "LIKES");
    assert_ne!(id1, id2);

    // Same label, different source/target should produce different IDs.
    let id3 = hash_edge_id(1, 2, "KNOWS");
    let id4 = hash_edge_id(2, 1, "KNOWS");
    assert_ne!(id3, id4);

    // Deterministic: same inputs always give same output.
    let id5 = hash_edge_id(1, 2, "KNOWS");
    assert_eq!(id1, id5);
}

// =========================================================================
// ALTER COLLECTION — auto_reindex apply + persist
// =========================================================================

/// Builds an `ALTER COLLECTION <name> SET (auto_reindex = <enabled>)` statement.
fn alter_auto_reindex(name: &str, enabled: bool) -> DdlStatement {
    DdlStatement::AlterCollection(AlterCollectionStatement {
        collection: name.to_string(),
        options: vec![("auto_reindex".to_string(), enabled.to_string())],
    })
}

/// Creates a 32-dim vector collection for ALTER tests.
fn create_alter_collection(db: &Database, name: &str) {
    let create = DdlStatement::CreateCollection(CreateCollectionStatement {
        name: name.to_string(),
        kind: CreateCollectionKind::Vector(VectorCollectionParams {
            dimension: 32,
            metric: "cosine".to_string(),
            storage: None,
            m: None,
            ef_construction: None,
        }),
    });
    execute_ddl(db, create).expect("create");
}

#[test]
fn test_alter_collection_set_auto_reindex_true_attaches_manager() {
    let dir = tempdir().expect("tempdir");
    let db = Database::open(dir.path()).expect("open");
    create_alter_collection(&db, "alter_test");

    let results =
        execute_ddl(&db, alter_auto_reindex("alter_test", true)).expect("ALTER true must succeed");
    assert!(results.is_empty(), "ALTER returns no rows");

    let vc = db.get_vector_collection("alter_test").expect("get");
    let manager = vc
        .auto_reindex_manager()
        .expect("auto_reindex manager must be attached after ALTER");
    assert!(manager.is_enabled(), "auto_reindex must be enabled");
    assert!(
        vc.config().auto_reindex_config.is_some_and(|c| c.enabled),
        "persisted config reflects the enabled policy"
    );
}

#[test]
fn test_alter_collection_set_auto_reindex_false_disables() {
    let dir = tempdir().expect("tempdir");
    let db = Database::open(dir.path()).expect("open");
    create_alter_collection(&db, "alter_disable");

    execute_ddl(&db, alter_auto_reindex("alter_disable", true)).expect("enable");
    execute_ddl(&db, alter_auto_reindex("alter_disable", false)).expect("disable");

    let vc = db.get_vector_collection("alter_disable").expect("get");
    let manager = vc
        .auto_reindex_manager()
        .expect("manager stays attached (disabled config) for a symmetric round-trip");
    assert!(!manager.is_enabled(), "auto_reindex must be disabled");
    assert!(
        vc.config().auto_reindex_config.is_some_and(|c| !c.enabled),
        "persisted config reflects the disabled policy"
    );
}

#[test]
fn test_alter_collection_rejects_unknown_option() {
    let dir = tempdir().expect("tempdir");
    let db = Database::open(dir.path()).expect("open");
    create_alter_collection(&db, "alter_unknown");

    // Unknown options return the specific "Unsupported ALTER option"
    // diagnostic; nothing is applied or persisted.
    let alter = DdlStatement::AlterCollection(AlterCollectionStatement {
        collection: "alter_unknown".to_string(),
        options: vec![("nonexistent_option".to_string(), "value".to_string())],
    });
    let err = execute_ddl(&db, alter).expect_err("unknown option must error");
    let err_msg = err.to_string();
    assert!(
        err_msg.contains("Unsupported ALTER option"),
        "must reject unknown option specifically: {err_msg}"
    );
    assert!(
        err_msg.contains("nonexistent_option"),
        "must echo the unknown option name: {err_msg}"
    );
}

#[test]
fn test_alter_collection_rejects_non_bool_value() {
    let dir = tempdir().expect("tempdir");
    let db = Database::open(dir.path()).expect("open");
    create_alter_collection(&db, "alter_bad_value");

    // Malformed values return the type-specific diagnostic; nothing is applied.
    let alter = DdlStatement::AlterCollection(AlterCollectionStatement {
        collection: "alter_bad_value".to_string(),
        options: vec![("auto_reindex".to_string(), "not_a_bool".to_string())],
    });
    let err = execute_ddl(&db, alter).expect_err("bad value must error");
    assert!(
        err.to_string()
            .contains("auto_reindex must be 'true' or 'false'"),
        "must reject invalid bool value specifically: {err}"
    );
}

#[test]
fn test_alter_collection_multi_option_is_atomic_on_error() {
    let dir = tempdir().expect("tempdir");
    let db = Database::open(dir.path()).expect("open");
    create_alter_collection(&db, "alter_atomic");

    // A valid option followed by an invalid one must leave the collection
    // untouched: every option is validated before any is applied.
    let alter = DdlStatement::AlterCollection(AlterCollectionStatement {
        collection: "alter_atomic".to_string(),
        options: vec![
            ("auto_reindex".to_string(), "true".to_string()),
            ("nonexistent_option".to_string(), "x".to_string()),
        ],
    });
    let err = execute_ddl(&db, alter).expect_err("multi-option with a bad key must error");
    assert!(
        err.to_string().contains("Unsupported ALTER option"),
        "got: {err}"
    );

    let vc = db.get_vector_collection("alter_atomic").expect("get");
    assert!(
        vc.auto_reindex_manager().is_none(),
        "the valid auto_reindex=true must NOT be applied when a later option fails"
    );
    assert!(
        vc.config().auto_reindex_config.is_none(),
        "no policy must be persisted after the failed ALTER"
    );
}

// =========================================================================
// SELECT EDGES AND — condition ordering optimization (Finding 3)
// =========================================================================

#[test]
fn test_select_edges_and_swaps_for_selectivity() {
    let dir = tempdir().expect("tempdir");
    let db = Database::open(dir.path()).expect("open");

    let create = DdlStatement::CreateCollection(CreateCollectionStatement {
        name: "sel_and_g".to_string(),
        kind: CreateCollectionKind::Graph(GraphCollectionParams {
            dimension: None,
            metric: None,
            schema_mode: GraphSchemaMode::Schemaless,
        }),
    });
    execute_ddl(&db, create).expect("create graph");

    let gc = db.get_graph_collection("sel_and_g").expect("get");
    for id in [10, 20, 30, 40, 50] {
        gc.upsert_node_payload(id, &serde_json::json!({}))
            .expect("store node");
    }
    gc.add_edge(crate::GraphEdge::new(1, 10, 20, "KNOWS").expect("edge"))
        .expect("add 1");
    gc.add_edge(crate::GraphEdge::new(2, 10, 30, "LIKES").expect("edge"))
        .expect("add 2");
    gc.add_edge(crate::GraphEdge::new(3, 40, 50, "KNOWS").expect("edge"))
        .expect("add 3");

    // Query: label = 'KNOWS' AND source = 10  (label on left, source on right)
    // The optimizer should swap so source drives the lookup.
    let select = DmlStatement::SelectEdges(SelectEdgesStatement {
        collection: "sel_and_g".to_string(),
        where_clause: Some(Condition::And(
            Box::new(Condition::Comparison(Comparison {
                column: "label".to_string(),
                operator: CompareOp::Eq,
                value: Value::String("KNOWS".to_string()),
            })),
            Box::new(Condition::Comparison(Comparison {
                column: "source".to_string(),
                operator: CompareOp::Eq,
                value: Value::Integer(10),
            })),
        )),
        limit: None,
    });
    let results = execute_dml(&db, select).expect("select edges");

    // Only edge 1 matches (source=10 AND label=KNOWS); edge 2 is LIKES, edge 3 is source=40.
    assert_eq!(results.len(), 1);
    let payload = results[0].point.payload.as_ref().expect("payload");
    assert_eq!(payload["edge_id"], 1);
}
