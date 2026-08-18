use super::*;

#[test]
fn test_parse_source_type_all_variants() {
    let cases = [
        ("supabase", SourceType::Supabase),
        ("qdrant", SourceType::Qdrant),
        ("pinecone", SourceType::Pinecone),
        ("weaviate", SourceType::Weaviate),
        ("milvus", SourceType::Milvus),
        ("chromadb", SourceType::ChromaDB),
        ("json_file", SourceType::JsonFile),
        ("json", SourceType::JsonFile),
        ("csv_file", SourceType::CsvFile),
        ("csv", SourceType::CsvFile),
        ("elasticsearch", SourceType::Elasticsearch),
        ("redis", SourceType::Redis),
    ];

    for (input, expected) in &cases {
        let result = parse_source_type(input);
        assert!(
            result.is_ok(),
            "parse_source_type({input:?}) should succeed"
        );
        assert_eq!(result.unwrap(), *expected);
    }
}

#[test]
fn test_parse_source_type_case_insensitive() {
    assert_eq!(parse_source_type("QDRANT").unwrap(), SourceType::Qdrant);
    assert_eq!(parse_source_type("Supabase").unwrap(), SourceType::Supabase);
}

#[test]
fn test_parse_source_type_unknown() {
    assert!(parse_source_type("unknown").is_err());
    assert!(parse_source_type("").is_err());
}

#[test]
fn test_build_source_config_qdrant() {
    let params = SourceParams {
        source_type: SourceType::Qdrant,
        url: "http://localhost:6333",
        api_key: Some("test-key"),
        collection: "vectors",
    };

    let result = build_source_config(&params);
    assert!(result.is_ok());
    let source = result.unwrap();
    match source {
        SourceConfig::Qdrant(cfg) => {
            assert_eq!(cfg.url, "http://localhost:6333");
            assert_eq!(cfg.collection, "vectors");
            assert_eq!(cfg.api_key, Some("test-key".to_string()));
        }
        _ => panic!("Expected Qdrant config"),
    }
}

#[test]
fn test_build_source_config_supabase() {
    let params = SourceParams {
        source_type: SourceType::Supabase,
        url: "https://xyz.supabase.co",
        api_key: Some("service-key"),
        collection: "documents",
    };

    let result = build_source_config(&params);
    assert!(result.is_ok());
    let source = result.unwrap();
    match source {
        SourceConfig::Supabase(cfg) => {
            assert_eq!(cfg.url, "https://xyz.supabase.co");
            assert_eq!(cfg.table, "documents");
            assert_eq!(cfg.api_key, "service-key");
        }
        _ => panic!("Expected Supabase config"),
    }
}

#[test]
fn test_build_source_config_pinecone() {
    let params = SourceParams {
        source_type: SourceType::Pinecone,
        url: "https://index.pinecone.io",
        api_key: Some("pinecone-key"),
        collection: "my-index",
    };

    let result = build_source_config(&params);
    assert!(result.is_ok());
    let source = result.unwrap();
    match source {
        SourceConfig::Pinecone(cfg) => {
            assert_eq!(cfg.api_key, "pinecone-key");
            assert_eq!(cfg.index, "my-index");
        }
        _ => panic!("Expected Pinecone config"),
    }
}

#[test]
fn test_build_source_config_weaviate() {
    let params = SourceParams {
        source_type: SourceType::Weaviate,
        url: "http://localhost:8080",
        api_key: None,
        collection: "Document",
    };

    let result = build_source_config(&params);
    assert!(result.is_ok());
    let source = result.unwrap();
    match source {
        SourceConfig::Weaviate(cfg) => {
            assert_eq!(cfg.url, "http://localhost:8080");
            assert_eq!(cfg.class_name, "Document");
            assert!(cfg.api_key.is_none());
        }
        _ => panic!("Expected Weaviate config"),
    }
}

#[test]
fn test_build_source_config_milvus() {
    let params = SourceParams {
        source_type: SourceType::Milvus,
        url: "http://localhost:19530",
        api_key: None,
        collection: "vectors",
    };

    let result = build_source_config(&params);
    assert!(result.is_ok());
    let source = result.unwrap();
    match source {
        SourceConfig::Milvus(cfg) => {
            assert_eq!(cfg.url, "http://localhost:19530");
            assert_eq!(cfg.collection, "vectors");
        }
        _ => panic!("Expected Milvus config"),
    }
}

#[test]
fn test_build_source_config_chromadb() {
    let params = SourceParams {
        source_type: SourceType::ChromaDB,
        url: "http://localhost:8000",
        api_key: None,
        collection: "embeddings",
    };

    let result = build_source_config(&params);
    assert!(result.is_ok());
    let source = result.unwrap();
    match source {
        SourceConfig::ChromaDB(cfg) => {
            assert_eq!(cfg.url, "http://localhost:8000");
            assert_eq!(cfg.collection, "embeddings");
        }
        _ => panic!("Expected ChromaDB config"),
    }
}

#[test]
fn test_build_source_config_json_file() {
    let params = SourceParams {
        source_type: SourceType::JsonFile,
        url: "./vectors.json",
        api_key: None,
        collection: "unused",
    };

    let result = build_source_config(&params);
    assert!(result.is_ok());
    match result.unwrap() {
        SourceConfig::JsonFile(cfg) => {
            assert_eq!(cfg.path, std::path::PathBuf::from("./vectors.json"));
            assert_eq!(cfg.id_field, "id");
            assert_eq!(cfg.vector_field, "vector");
        }
        _ => panic!("Expected JsonFile config"),
    }
}

#[test]
fn test_build_source_config_csv_file() {
    let params = SourceParams {
        source_type: SourceType::CsvFile,
        url: "./vectors.csv",
        api_key: None,
        collection: "unused",
    };

    let result = build_source_config(&params);
    assert!(result.is_ok());
    match result.unwrap() {
        SourceConfig::CsvFile(cfg) => {
            assert_eq!(cfg.path, std::path::PathBuf::from("./vectors.csv"));
            assert_eq!(cfg.delimiter, ',');
            assert!(cfg.has_header);
        }
        _ => panic!("Expected CsvFile config"),
    }
}

#[test]
fn test_build_source_config_elasticsearch() {
    let params = SourceParams {
        source_type: SourceType::Elasticsearch,
        url: "http://localhost:9200",
        api_key: Some("es-key"),
        collection: "vectors",
    };

    let result = build_source_config(&params);
    assert!(result.is_ok());
    match result.unwrap() {
        SourceConfig::Elasticsearch(cfg) => {
            assert_eq!(cfg.url, "http://localhost:9200");
            assert_eq!(cfg.index, "vectors");
            assert_eq!(cfg.api_key, Some("es-key".to_string()));
        }
        _ => panic!("Expected Elasticsearch config"),
    }
}

#[test]
fn test_build_source_config_redis() {
    let params = SourceParams {
        source_type: SourceType::Redis,
        url: "redis://localhost:6379",
        api_key: Some("redis-pass"),
        collection: "my-idx",
    };

    let result = build_source_config(&params);
    assert!(result.is_ok());
    match result.unwrap() {
        SourceConfig::Redis(cfg) => {
            assert_eq!(cfg.url, "redis://localhost:6379");
            assert_eq!(cfg.index, "my-idx");
            assert_eq!(cfg.password, Some("redis-pass".to_string()));
        }
        _ => panic!("Expected Redis config"),
    }
}

#[test]
fn test_build_source_config_missing_api_key_rejected() {
    // Supabase requires an API key — None must be rejected.
    let params = SourceParams {
        source_type: SourceType::Supabase,
        url: "https://xyz.supabase.co",
        api_key: None,
        collection: "docs",
    };
    assert!(build_source_config(&params).is_err());
}

#[test]
fn test_build_source_config_empty_api_key_rejected() {
    // An empty string is not a valid API key.
    let params = SourceParams {
        source_type: SourceType::Supabase,
        url: "https://xyz.supabase.co",
        api_key: Some(""),
        collection: "docs",
    };
    assert!(build_source_config(&params).is_err());
}

#[test]
fn test_pinecone_missing_api_key_rejected() {
    let params = SourceParams {
        source_type: SourceType::Pinecone,
        url: "https://index.pinecone.io",
        api_key: None,
        collection: "my-index",
    };
    assert!(build_source_config(&params).is_err());
}
