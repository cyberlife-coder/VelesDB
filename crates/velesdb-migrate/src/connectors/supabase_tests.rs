use super::*;

#[test]
fn test_parse_pgvector_wire_format_from_string() {
    let val = serde_json::json!("[0.1,0.2,0.3]");
    let vec = parse_pgvector_wire_format(&val);
    assert_eq!(vec.len(), 3);
    assert!((vec[0] - 0.1).abs() < 0.001);
}

#[test]
fn test_parse_pgvector_wire_format_from_array() {
    let val = serde_json::json!([0.1, 0.2, 0.3]);
    let vec = parse_pgvector_wire_format(&val);
    assert_eq!(vec.len(), 3);
}

#[test]
fn test_supabase_connector_new() {
    let config = SupabaseConfig {
        url: "https://xxx.supabase.co".to_string(),
        api_key: "test-key".to_string(),
        table: "documents".to_string(),
        vector_column: "embedding".to_string(),
        id_column: "id".to_string(),
        payload_columns: vec![],
        metric: None,
    };

    let connector = SupabaseConnector::new(config);
    assert_eq!(connector.source_type(), "supabase");
}

#[test]
fn test_supabase_connect_rejects_file_url() {
    assert!(crate::connectors::common::validate_url("file:///etc/passwd").is_err());
}

#[test]
fn test_normalise_supabase_metric_maps_pgvector_operator_classes() {
    // pgvector operator class aliases are what operators actually
    // declare in their index DDL. The normaliser must accept them
    // verbatim so check_metric_fidelity compares apples to apples.
    assert_eq!(
        SupabaseConnector::normalise_supabase_metric("vector_cosine_ops"),
        "cosine"
    );
    assert_eq!(
        SupabaseConnector::normalise_supabase_metric("vector_l2_ops"),
        "euclidean"
    );
    assert_eq!(
        SupabaseConnector::normalise_supabase_metric("vector_ip_ops"),
        "dot"
    );
}

#[test]
fn test_normalise_supabase_metric_accepts_short_aliases() {
    // Operators might also write the short pgvector aliases.
    assert_eq!(
        SupabaseConnector::normalise_supabase_metric("l2"),
        "euclidean"
    );
    assert_eq!(SupabaseConnector::normalise_supabase_metric("ip"), "dot");
}

#[test]
fn test_normalise_supabase_metric_lowercases_known_values() {
    assert_eq!(
        SupabaseConnector::normalise_supabase_metric("Cosine"),
        "cosine"
    );
    assert_eq!(
        SupabaseConnector::normalise_supabase_metric("EUCLIDEAN"),
        "euclidean"
    );
}

#[test]
fn test_normalise_supabase_metric_preserves_unknown_values() {
    assert_eq!(
        SupabaseConnector::normalise_supabase_metric("manhattan"),
        "manhattan"
    );
}
