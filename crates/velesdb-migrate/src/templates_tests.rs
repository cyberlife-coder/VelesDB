use super::*;
use velesdb_migrate::connectors::FieldInfo;

const ALL_SOURCES: &[&str] = &[
    "qdrant", "pinecone", "weaviate", "milvus", "chromadb", "supabase",
];

#[test]
fn test_get_template_known_sources() {
    for source in ALL_SOURCES {
        let tmpl = get_template(source);
        assert!(
            tmpl.is_some(),
            "get_template({source:?}) should return Some"
        );
        assert!(
            !tmpl.unwrap().is_empty(),
            "template for {source:?} should not be empty"
        );
    }
}

#[test]
fn test_get_template_unknown_source() {
    assert!(get_template("unknown").is_none());
    assert!(get_template("redis").is_none());
    assert!(get_template("").is_none());
}

#[test]
fn test_get_template_case_insensitive() {
    for source in ALL_SOURCES {
        let upper = source.to_uppercase();
        let title = {
            let mut chars = source.chars();
            match chars.next() {
                Some(c) => c.to_uppercase().to_string() + chars.as_str(),
                None => String::new(),
            }
        };

        assert!(
            get_template(&upper).is_some(),
            "get_template({upper:?}) should match case-insensitively"
        );
        assert!(
            get_template(&title).is_some(),
            "get_template({title:?}) should match case-insensitively"
        );
    }
}

#[test]
fn test_templates_contain_required_fields() {
    for source in ALL_SOURCES {
        let tmpl = get_template(source).unwrap();
        assert!(
            tmpl.contains("source:"),
            "template for {source:?} missing 'source:' key"
        );
        assert!(
            tmpl.contains("destination:"),
            "template for {source:?} missing 'destination:' key"
        );
        // Pinecone uses "index:" instead of "collection:", but all others
        // have either "collection:" or "table:" or "class_name:".
        let has_collection_like = tmpl.contains("collection:")
            || tmpl.contains("table:")
            || tmpl.contains("class_name:")
            || tmpl.contains("index:");
        assert!(
            has_collection_like,
            "template for {source:?} missing collection/table/index identifier"
        );
    }
}

fn make_schema(fields: Vec<FieldInfo>) -> SourceSchema {
    SourceSchema {
        source_type: "test".to_string(),
        collection: "my_collection".to_string(),
        dimension: 384,
        total_count: Some(5000),
        fields,
        vector_column: None,
        id_column: None,
        metric: None,
    }
}

fn make_params<'a>(source_type: &'a str, schema: &'a SourceSchema) -> AutoConfigParams<'a> {
    AutoConfigParams {
        source_type,
        url: "http://localhost:6333",
        collection: "my_collection",
        api_key: Some("secret-key"),
        dest_path: std::path::Path::new("./data"),
        schema,
    }
}

#[test]
fn test_generate_auto_config_qdrant() {
    let schema = make_schema(vec![]);
    let params = make_params("qdrant", &schema);
    let yaml = generate_auto_config(&params);

    assert!(yaml.contains("source:"), "missing source key");
    assert!(yaml.contains("type: qdrant"), "missing source type");
    assert!(yaml.contains("destination:"), "missing destination key");
    assert!(
        yaml.contains("collection: my_collection"),
        "missing collection"
    );
    assert!(yaml.contains("dimension: 384"), "missing dimension");
    assert!(yaml.contains("api_key: secret-key"), "missing api_key");
    assert!(yaml.contains("5000"), "missing total count");
}

#[test]
fn test_generate_auto_config_supabase() {
    let schema = SourceSchema {
        source_type: "supabase".to_string(),
        collection: "docs".to_string(),
        dimension: 768,
        total_count: Some(100),
        fields: vec![
            FieldInfo {
                name: "doc_id".to_string(),
                field_type: "integer".to_string(),
                indexed: true,
            },
            FieldInfo {
                name: "embedding".to_string(),
                field_type: "vector".to_string(),
                indexed: false,
            },
            FieldInfo {
                name: "title".to_string(),
                field_type: "string".to_string(),
                indexed: false,
            },
        ],
        vector_column: Some("embedding".to_string()),
        id_column: Some("doc_id".to_string()),
        metric: None,
    };
    let params = AutoConfigParams {
        source_type: "supabase",
        url: "https://proj.supabase.co",
        collection: "docs",
        api_key: None,
        dest_path: std::path::Path::new("./supabase_data"),
        schema: &schema,
    };
    let yaml = generate_auto_config(&params);

    assert!(yaml.contains("type: supabase"), "missing source type");
    assert!(
        yaml.contains("vector_column: embedding"),
        "missing vector column"
    );
    assert!(yaml.contains("id_column: doc_id"), "missing id column");
    assert!(yaml.contains("- title"), "missing payload field 'title'");
    assert!(
        !yaml.contains("- embedding"),
        "vector col should be excluded from fields"
    );
    assert!(
        !yaml.contains("- doc_id"),
        "id col should be excluded from fields"
    );
}

#[test]
fn test_generate_auto_config_generic_fallback() {
    let schema = make_schema(vec![]);
    let params = make_params("elasticsearch", &schema);
    let yaml = generate_auto_config(&params);

    assert!(yaml.contains("source:"), "missing source key");
    assert!(yaml.contains("type: elasticsearch"), "missing source type");
    assert!(yaml.contains("destination:"), "missing destination key");
    assert!(yaml.contains("dimension: 384"), "missing dimension");
}

#[test]
fn test_generate_auto_config_zero_dimension_defaults_to_768() {
    let schema = SourceSchema {
        source_type: "test".to_string(),
        collection: "coll".to_string(),
        dimension: 0,
        total_count: None,
        fields: vec![],
        vector_column: None,
        id_column: None,
        metric: None,
    };
    let params = make_params("milvus", &schema);
    let yaml = generate_auto_config(&params);

    assert!(
        yaml.contains("dimension: 768"),
        "zero dimension should default to 768"
    );
}

#[test]
fn test_generate_auto_config_no_api_key() {
    let schema = make_schema(vec![]);
    let params = AutoConfigParams {
        source_type: "qdrant",
        url: "http://localhost:6333",
        collection: "coll",
        api_key: None,
        dest_path: std::path::Path::new("./data"),
        schema: &schema,
    };
    let yaml = generate_auto_config(&params);

    assert!(
        yaml.contains("# api_key:"),
        "missing api_key should produce a commented-out line"
    );
}

#[test]
fn test_detect_vector_column_heuristic() {
    let schema = SourceSchema {
        source_type: "test".to_string(),
        collection: "coll".to_string(),
        dimension: 128,
        total_count: None,
        fields: vec![
            FieldInfo {
                name: "id".to_string(),
                field_type: "integer".to_string(),
                indexed: true,
            },
            FieldInfo {
                name: "content_embedding".to_string(),
                field_type: "vector".to_string(),
                indexed: false,
            },
        ],
        vector_column: None,
        id_column: None,
        metric: None,
    };
    let col = detect_vector_column(&schema);
    assert_eq!(col, "content_embedding");
}

#[test]
fn test_detect_id_column_heuristic() {
    let schema = SourceSchema {
        source_type: "test".to_string(),
        collection: "coll".to_string(),
        dimension: 128,
        total_count: None,
        fields: vec![
            FieldInfo {
                name: "doc_id".to_string(),
                field_type: "integer".to_string(),
                indexed: true,
            },
            FieldInfo {
                name: "embedding".to_string(),
                field_type: "vector".to_string(),
                indexed: false,
            },
        ],
        vector_column: None,
        id_column: None,
        metric: None,
    };
    let col = detect_id_column(&schema);
    assert_eq!(col, "doc_id");
}

#[test]
fn test_build_fields_list_excludes_id_and_vector() {
    let schema = SourceSchema {
        source_type: "test".to_string(),
        collection: "coll".to_string(),
        dimension: 128,
        total_count: None,
        fields: vec![
            FieldInfo {
                name: "id".to_string(),
                field_type: "integer".to_string(),
                indexed: true,
            },
            FieldInfo {
                name: "embedding".to_string(),
                field_type: "vector".to_string(),
                indexed: false,
            },
            FieldInfo {
                name: "title".to_string(),
                field_type: "string".to_string(),
                indexed: false,
            },
            FieldInfo {
                name: "category".to_string(),
                field_type: "string".to_string(),
                indexed: true,
            },
        ],
        vector_column: None,
        id_column: None,
        metric: None,
    };
    let list = build_fields_list(&schema, "id", "embedding");
    assert!(list.contains("- title"));
    assert!(list.contains("- category"));
    assert!(!list.contains("- id"));
    assert!(!list.contains("- embedding"));
}

#[test]
fn test_build_fields_list_empty_returns_comment() {
    let schema = make_schema(vec![]);
    let list = build_fields_list(&schema, "id", "embedding");
    assert!(
        list.contains("automatically"),
        "empty fields should produce a comment"
    );
}
