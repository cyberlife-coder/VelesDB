use super::*;

#[test]
fn test_normalise_milvus_metric_maps_l2_to_euclidean() {
    // Milvus reports 'L2' for squared-L2 distance; VelesDB core
    // uses 'euclidean'. The mapping is what allows
    // check_metric_fidelity to honestly compare a Milvus source
    // against a core collection created with metric: "euclidean".
    assert_eq!(MilvusConnector::normalise_milvus_metric("L2"), "euclidean");
    assert_eq!(MilvusConnector::normalise_milvus_metric("l2"), "euclidean");
}

#[test]
fn test_normalise_milvus_metric_maps_ip_to_dot() {
    // Milvus 'IP' (inner product) maps to VelesDB's 'dot'.
    assert_eq!(MilvusConnector::normalise_milvus_metric("IP"), "dot");
    assert_eq!(MilvusConnector::normalise_milvus_metric("ip"), "dot");
}

#[test]
fn test_normalise_milvus_metric_lowercases_known_values() {
    assert_eq!(MilvusConnector::normalise_milvus_metric("COSINE"), "cosine");
    assert_eq!(
        MilvusConnector::normalise_milvus_metric("HAMMING"),
        "hamming"
    );
    assert_eq!(
        MilvusConnector::normalise_milvus_metric("JACCARD"),
        "jaccard"
    );
}

#[test]
fn test_normalise_milvus_metric_preserves_unknown_values() {
    // TANIMOTO is a legacy Milvus metric not supported by VelesDB
    // core — preserved verbatim so mismatch errors stay actionable.
    assert_eq!(
        MilvusConnector::normalise_milvus_metric("TANIMOTO"),
        "tanimoto"
    );
}

#[test]
fn test_milvus_connector_new() {
    let config = MilvusConfig {
        url: "http://localhost:19530".to_string(),
        collection: "test".to_string(),
        username: None,
        password: None,
    };

    let connector = MilvusConnector::new(config);
    assert_eq!(connector.source_type(), "milvus");
}

#[test]
fn test_query_request_serialization() {
    let req = QueryRequest {
        collection_name: "test".to_string(),
        filter: "".to_string(),
        limit: 100,
        offset: 0,
        output_fields: vec!["id".to_string(), "vector".to_string()],
    };

    let json = serde_json::to_string(&req).unwrap();
    assert!(json.contains("\"collectionName\":\"test\""));
    assert!(json.contains("\"limit\":100"));
}

#[test]
fn test_connect_rejects_file_url() {
    assert!(crate::connectors::common::validate_url("file:///etc/passwd").is_err());
}

fn dim_param(dim: usize) -> Vec<FieldParam> {
    vec![FieldParam {
        key: "dim".to_string(),
        value: dim.to_string(),
    }]
}

#[test]
fn test_find_vector_field_detects_float_vector() {
    let schema = CollectionSchema {
        fields: vec![
            FieldSchema {
                name: "id".to_string(),
                field_type: "Int64".to_string(),
                is_primary_key: Some(true),
                params: vec![],
            },
            FieldSchema {
                name: "embedding".to_string(),
                field_type: "FloatVector".to_string(),
                is_primary_key: None,
                params: dim_param(128),
            },
        ],
        indexes: vec![],
    };
    let (name, dim) =
        MilvusConnector::find_vector_field(&schema).expect("test: should detect FloatVector");
    assert_eq!(name, "embedding");
    assert_eq!(dim, 128);
}

#[test]
fn test_find_vector_field_detects_float_vector_uppercase() {
    let schema = CollectionSchema {
        fields: vec![FieldSchema {
            name: "vec".to_string(),
            field_type: "FLOAT_VECTOR".to_string(),
            is_primary_key: None,
            params: dim_param(768),
        }],
        indexes: vec![],
    };
    let (name, dim) =
        MilvusConnector::find_vector_field(&schema).expect("test: FLOAT_VECTOR uppercase");
    assert_eq!(name, "vec");
    assert_eq!(dim, 768);
}

#[test]
fn test_find_vector_field_returns_error_when_no_vector_field() {
    let schema = CollectionSchema {
        fields: vec![
            FieldSchema {
                name: "id".to_string(),
                field_type: "Int64".to_string(),
                is_primary_key: Some(true),
                params: vec![],
            },
            FieldSchema {
                name: "name".to_string(),
                field_type: "VarChar".to_string(),
                is_primary_key: None,
                params: vec![],
            },
        ],
        indexes: vec![],
    };
    assert!(MilvusConnector::find_vector_field(&schema).is_err());
}

#[test]
fn test_extract_index_metric_matches_by_field_name() {
    let schema = CollectionSchema {
        fields: vec![],
        indexes: vec![
            IndexInfo {
                field_name: "other".to_string(),
                metric_type: Some("L2".to_string()),
            },
            IndexInfo {
                field_name: "vector".to_string(),
                metric_type: Some("COSINE".to_string()),
            },
        ],
    };
    assert_eq!(
        MilvusConnector::extract_index_metric(&schema, "vector"),
        Some("cosine".to_string())
    );
}

#[test]
fn test_extract_index_metric_returns_none_when_field_absent() {
    let schema = CollectionSchema {
        fields: vec![],
        indexes: vec![IndexInfo {
            field_name: "other".to_string(),
            metric_type: Some("L2".to_string()),
        }],
    };
    assert_eq!(
        MilvusConnector::extract_index_metric(&schema, "vector"),
        None
    );
}

#[test]
fn test_extract_index_metric_returns_none_when_indexes_empty() {
    let schema = CollectionSchema {
        fields: vec![],
        indexes: vec![],
    };
    assert_eq!(
        MilvusConnector::extract_index_metric(&schema, "vector"),
        None
    );
}

#[test]
fn test_field_schema_dimension_parses_dim_param() {
    let field = FieldSchema {
        name: "v".to_string(),
        field_type: "FloatVector".to_string(),
        is_primary_key: None,
        params: dim_param(512),
    };
    assert_eq!(field.dimension(), 512);
}

#[test]
fn test_field_schema_dimension_returns_zero_when_dim_absent() {
    let field = FieldSchema {
        name: "v".to_string(),
        field_type: "FloatVector".to_string(),
        is_primary_key: None,
        params: vec![],
    };
    assert_eq!(field.dimension(), 0);
}

#[test]
fn test_field_schema_deserialises_primary_key_from_camelcase() {
    // Regression for the Milvus v2 REST primaryKey/isPrimaryKey
    // drift caught by Devin review on PR #583. The v2 REST
    // shape uses "primaryKey" as the JSON key, not
    // "isPrimaryKey" — the pre-Sprint-1.5 annotation silently
    // dropped the value to None for every field, so the
    // downstream FieldInfo.indexed flag was always false for
    // primary key fields.
    let json = r#"{
        "name": "id",
        "type": "Int64",
        "primaryKey": true
    }"#;
    let field: FieldSchema = serde_json::from_str(json).expect("parse primaryKey");
    assert_eq!(field.is_primary_key, Some(true));
}

#[test]
fn test_field_schema_deserialises_legacy_is_primary_key_alias() {
    // The serde alias should still accept the pre-Sprint-1.5
    // "isPrimaryKey" spelling in case an older Milvus version
    // emits it. Forward + backward compat in one field.
    let json = r#"{
        "name": "id",
        "type": "Int64",
        "isPrimaryKey": true
    }"#;
    let field: FieldSchema = serde_json::from_str(json).expect("parse isPrimaryKey");
    assert_eq!(field.is_primary_key, Some(true));
}

#[test]
fn test_field_schema_primary_key_none_when_key_absent() {
    let json = r#"{ "name": "x", "type": "Int64" }"#;
    let field: FieldSchema = serde_json::from_str(json).expect("parse without key");
    assert_eq!(field.is_primary_key, None);
}

#[test]
fn test_field_schema_dimension_handles_unparseable_dim() {
    let field = FieldSchema {
        name: "v".to_string(),
        field_type: "FloatVector".to_string(),
        is_primary_key: None,
        params: vec![FieldParam {
            key: "dim".to_string(),
            value: "not_a_number".to_string(),
        }],
    };
    assert_eq!(field.dimension(), 0);
}
