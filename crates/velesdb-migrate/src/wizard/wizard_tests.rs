use super::*;
use crate::config::{DistanceMetric, StorageMode};

// ==================== SourceType Tests ====================

#[test]
fn test_source_type_all_returns_all_variants() {
    let all = SourceType::all();
    assert_eq!(all.len(), 10);
    assert!(all.contains(&SourceType::Supabase));
    assert!(all.contains(&SourceType::Qdrant));
    assert!(all.contains(&SourceType::Pinecone));
    assert!(all.contains(&SourceType::Weaviate));
    assert!(all.contains(&SourceType::Milvus));
    assert!(all.contains(&SourceType::ChromaDB));
    assert!(all.contains(&SourceType::JsonFile));
    assert!(all.contains(&SourceType::CsvFile));
    assert!(all.contains(&SourceType::Elasticsearch));
    assert!(all.contains(&SourceType::Redis));
}

#[test]
fn test_source_type_display_names() {
    assert_eq!(
        SourceType::Supabase.display_name(),
        "Supabase (PostgreSQL + pgvector)"
    );
    assert_eq!(SourceType::Qdrant.display_name(), "Qdrant");
    assert_eq!(SourceType::Pinecone.display_name(), "Pinecone");
    assert_eq!(SourceType::Weaviate.display_name(), "Weaviate");
    assert_eq!(SourceType::Milvus.display_name(), "Milvus / Zilliz Cloud");
    assert_eq!(SourceType::ChromaDB.display_name(), "ChromaDB");
}

#[test]
fn test_source_type_short_names() {
    assert_eq!(SourceType::Supabase.short_name(), "supabase");
    assert_eq!(SourceType::Qdrant.short_name(), "qdrant");
    assert_eq!(SourceType::Pinecone.short_name(), "pinecone");
    assert_eq!(SourceType::Weaviate.short_name(), "weaviate");
    assert_eq!(SourceType::Milvus.short_name(), "milvus");
    assert_eq!(SourceType::ChromaDB.short_name(), "chromadb");
}

#[test]
fn test_source_type_requires_api_key() {
    // Sources that require an API key.
    assert!(SourceType::Supabase.requires_api_key());
    assert!(SourceType::Pinecone.requires_api_key());

    // Sources that do not require an API key.
    assert!(!SourceType::Qdrant.requires_api_key());
    assert!(!SourceType::Weaviate.requires_api_key());
    assert!(!SourceType::Milvus.requires_api_key());
    assert!(!SourceType::ChromaDB.requires_api_key());
}

#[test]
fn test_source_type_optional_api_key() {
    // Sources with an optional API key.
    assert!(SourceType::Qdrant.optional_api_key());
    assert!(SourceType::Weaviate.optional_api_key());
    assert!(SourceType::Milvus.optional_api_key());

    // Sources without an optional API key.
    assert!(!SourceType::Supabase.optional_api_key());
    assert!(!SourceType::Pinecone.optional_api_key());
    assert!(!SourceType::ChromaDB.optional_api_key());
}

// ==================== WizardConfig Tests ====================

#[test]
fn test_wizard_config_creation() {
    // Arrange & Act
    let config = WizardConfig {
        source_type: SourceType::Qdrant,
        url: "http://localhost:6333".to_string(),
        collection: "test_collection".to_string(),
        dest_path: "./velesdb_data".to_string(),
        use_sq8: false,
    };

    // Assert
    assert_eq!(config.source_type, SourceType::Qdrant);
    assert_eq!(config.url, "http://localhost:6333");
    assert_eq!(config.collection, "test_collection");
    assert_eq!(config.dest_path, "./velesdb_data");
    assert!(!config.use_sq8);
}

// ==================== Wizard Tests ====================

#[test]
fn test_wizard_new() {
    // Arrange & Act
    let wizard = Wizard::new();

    // Assert - wizard should be created without panic
    // We can't easily test internal state, but creation should work
    drop(wizard);
}

#[test]
fn test_wizard_default() {
    // Arrange & Act
    let wizard = Wizard::default();

    // Assert
    drop(wizard);
}

// ==================== Build Source Config Tests ====================

#[test]
fn test_build_source_config_qdrant() {
    // Arrange
    let wizard = Wizard::new();
    let config = WizardConfig {
        source_type: SourceType::Qdrant,
        url: "http://localhost:6333".to_string(),
        collection: "vectors".to_string(),
        dest_path: "./data".to_string(),
        use_sq8: false,
    };

    // Act
    let result = wizard.build_source_config(&config, Some("test-key"));

    // Assert
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
    // Arrange
    let wizard = Wizard::new();
    let config = WizardConfig {
        source_type: SourceType::Supabase,
        url: "https://xyz.supabase.co".to_string(),
        collection: "documents".to_string(),
        dest_path: "./data".to_string(),
        use_sq8: false,
    };

    // Act
    let result = wizard.build_source_config(&config, Some("service-key"));

    // Assert
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
fn test_build_source_config_chromadb() {
    // Arrange
    let wizard = Wizard::new();
    let config = WizardConfig {
        source_type: SourceType::ChromaDB,
        url: "http://localhost:8000".to_string(),
        collection: "embeddings".to_string(),
        dest_path: "./data".to_string(),
        use_sq8: false,
    };

    // Act
    let result = wizard.build_source_config(&config, None);

    // Assert
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
fn test_build_source_config_pinecone() {
    // Arrange
    let wizard = Wizard::new();
    let config = WizardConfig {
        source_type: SourceType::Pinecone,
        url: "https://index.pinecone.io".to_string(),
        collection: "my-index".to_string(),
        dest_path: "./data".to_string(),
        use_sq8: false,
    };

    // Act
    let result = wizard.build_source_config(&config, Some("pinecone-key"));

    // Assert
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
    // Arrange
    let wizard = Wizard::new();
    let config = WizardConfig {
        source_type: SourceType::Weaviate,
        url: "http://localhost:8080".to_string(),
        collection: "Document".to_string(),
        dest_path: "./data".to_string(),
        use_sq8: false,
    };

    // Act
    let result = wizard.build_source_config(&config, None);

    // Assert
    assert!(result.is_ok());
    let source = result.unwrap();
    match source {
        SourceConfig::Weaviate(cfg) => {
            assert_eq!(cfg.url, "http://localhost:8080");
            assert_eq!(cfg.class_name, "Document");
        }
        _ => panic!("Expected Weaviate config"),
    }
}

#[test]
fn test_build_source_config_milvus() {
    // Arrange
    let wizard = Wizard::new();
    let config = WizardConfig {
        source_type: SourceType::Milvus,
        url: "http://localhost:19530".to_string(),
        collection: "vectors".to_string(),
        dest_path: "./data".to_string(),
        use_sq8: false,
    };

    // Act
    let result = wizard.build_source_config(&config, None);

    // Assert
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

// ==================== Build Migration Config Tests ====================

#[test]
fn test_build_migration_config_full_storage() {
    // Arrange
    let wizard = Wizard::new();
    let config = WizardConfig {
        source_type: SourceType::Qdrant,
        url: "http://localhost:6333".to_string(),
        collection: "test".to_string(),
        dest_path: "./velesdb_data".to_string(),
        use_sq8: false,
    };
    let schema = crate::connectors::SourceSchema {
        source_type: "qdrant".to_string(),
        collection: "test".to_string(),
        dimension: 768,
        total_count: Some(1000),
        fields: vec![],
        vector_column: None,
        id_column: None,
        metric: None,
    };

    // Act
    let result = wizard.build_migration_config(&config, &schema, None);

    // Assert
    assert!(result.is_ok());
    let migration = result.unwrap();
    assert_eq!(migration.destination.dimension, 768);
    assert_eq!(migration.destination.collection, "test");
    assert!(matches!(
        migration.destination.storage_mode,
        StorageMode::Full
    ));
    assert!(matches!(
        migration.destination.metric,
        DistanceMetric::Cosine
    ));
}

#[test]
fn test_build_migration_config_sq8_storage() {
    // Arrange
    let wizard = Wizard::new();
    let config = WizardConfig {
        source_type: SourceType::Qdrant,
        url: "http://localhost:6333".to_string(),
        collection: "test".to_string(),
        dest_path: "./velesdb_data".to_string(),
        use_sq8: true, // SQ8 enabled
    };
    let schema = crate::connectors::SourceSchema {
        source_type: "qdrant".to_string(),
        collection: "test".to_string(),
        dimension: 1536,
        total_count: Some(5000),
        fields: vec![],
        vector_column: None,
        id_column: None,
        metric: None,
    };

    // Act
    let result = wizard.build_migration_config(&config, &schema, None);

    // Assert
    assert!(result.is_ok());
    let migration = result.unwrap();
    assert_eq!(migration.destination.dimension, 1536);
    assert!(matches!(
        migration.destination.storage_mode,
        StorageMode::SQ8
    ));
}

#[test]
fn test_build_migration_config_options() {
    // Arrange
    let wizard = Wizard::new();
    let config = WizardConfig {
        source_type: SourceType::ChromaDB,
        url: "http://localhost:8000".to_string(),
        collection: "embeddings".to_string(),
        dest_path: "./data".to_string(),
        use_sq8: false,
    };
    let schema = crate::connectors::SourceSchema {
        source_type: "chromadb".to_string(),
        collection: "embeddings".to_string(),
        dimension: 384,
        total_count: None,
        fields: vec![],
        vector_column: None,
        id_column: None,
        metric: None,
    };

    // Act
    let result = wizard.build_migration_config(&config, &schema, None);

    // Assert
    assert!(result.is_ok());
    let migration = result.unwrap();
    assert_eq!(migration.options.batch_size, 1000);
    assert_eq!(migration.options.workers, 4);
    assert!(!migration.options.dry_run);
    assert!(migration.options.checkpoint_enabled);
}

// ==================== Secret Redaction Tests ====================
// The wizard collects an API key via `Password` input. Nothing the wizard
// writes to the console or tracing may contain that secret (issue #2601).

const CANARY_KEY: &str = "sk-canary-4f8e2d1c9b7a";

#[test]
fn test_redact_url_strips_userinfo_password() {
    let redacted = crate::redact::redact_url("redis://:s3cr3t-p4ss@cache.internal:6379/0");
    assert!(!redacted.contains("s3cr3t-p4ss"), "leaked: {redacted}");
    assert_eq!(redacted, "redis://***@cache.internal:6379/0");
}

#[test]
fn test_redact_url_strips_user_and_password() {
    let redacted = crate::redact::redact_url("postgres://admin:hunter2@db.local:5432/vecs");
    assert!(!redacted.contains("hunter2"), "leaked: {redacted}");
    assert!(!redacted.contains("admin"), "leaked: {redacted}");
    assert_eq!(redacted, "postgres://***@db.local:5432/vecs");
}

#[test]
fn test_redact_url_drops_query_and_fragment() {
    let redacted = crate::redact::redact_url("https://h.example.com/api?api_key=abc123#frag");
    assert!(!redacted.contains("abc123"), "leaked: {redacted}");
    assert_eq!(redacted, "https://h.example.com/api");
}

#[test]
fn test_redact_url_without_credentials_unchanged() {
    assert_eq!(
        crate::redact::redact_url("http://localhost:6333"),
        "http://localhost:6333"
    );
    assert_eq!(
        crate::redact::redact_url("./vectors.json"),
        "./vectors.json"
    );
}

#[test]
fn test_wizard_config_debug_contains_no_api_key_field() {
    // `WizardConfig` is echoed to the console; a struct that can hold the
    // collected key would leak it through its derived `Debug` too.
    let config = WizardConfig {
        source_type: SourceType::Qdrant,
        url: "http://localhost:6333".to_string(),
        collection: "vectors".to_string(),
        dest_path: "./data".to_string(),
        use_sq8: false,
    };
    let debug = format!("{config:?}");
    assert!(!debug.contains(CANARY_KEY), "leaked: {debug}");
    assert!(!debug.contains("api_key"), "leaked: {debug}");
}

#[test]
fn test_api_key_reaches_source_config_but_never_log_output() {
    // The key must still reach the connector config (its only consumer) while
    // every wizard sink renders output that cannot contain it.
    let config = WizardConfig {
        source_type: SourceType::Supabase,
        url: "https://xyz.supabase.co".to_string(),
        collection: "documents".to_string(),
        dest_path: "./data".to_string(),
        use_sq8: false,
    };
    let api_key = Some(CANARY_KEY.to_string());

    let wizard = Wizard::new();
    let source = wizard
        .build_source_config(&config, api_key.as_deref())
        .unwrap();
    match source {
        SourceConfig::Supabase(cfg) => assert_eq!(cfg.api_key, CANARY_KEY),
        _ => panic!("Expected Supabase config"),
    }

    let sinks = [
        super::ui::format_connecting(&config.url),
        format!("{config:?}"),
    ];
    for out in &sinks {
        assert!(!out.contains(CANARY_KEY), "leaked: {out}");
    }
}

#[test]
fn test_format_connecting_redacts_embedded_credentials() {
    let out = super::ui::format_connecting("redis://:hunter2@host:6379");
    assert!(!out.contains("hunter2"), "leaked: {out}");
    assert!(out.contains("host:6379"), "lost host: {out}");
}

#[test]
fn test_wizard_rendered_output_never_contains_secret() {
    // Render every wizard sink from a config plus a credential-bearing URL and
    // assert the secret cannot appear anywhere.
    let config = WizardConfig {
        source_type: SourceType::Supabase,
        url: "https://user:hunter2@proj.supabase.co".to_string(),
        collection: "documents".to_string(),
        dest_path: "./data".to_string(),
        use_sq8: false,
    };
    let schema = crate::connectors::SourceSchema {
        source_type: "supabase".to_string(),
        collection: "documents".to_string(),
        dimension: 768,
        total_count: Some(10),
        fields: vec![],
        vector_column: None,
        id_column: None,
        metric: None,
    };
    let stats = crate::pipeline::MigrationStats {
        extracted: 10,
        loaded: 10,
        failed: 0,
        batches: 1,
        duration_secs: 1.0,
        edges_created: 0,
        edges_failed: 0,
        relations_processed: 0,
    };

    let sinks = [
        super::ui::format_connecting(&config.url),
        super::prompts::format_preview(&schema, &config),
        super::ui::format_success(&stats, &config),
        format!("{config:?}"),
    ];
    for out in &sinks {
        assert!(!out.contains(CANARY_KEY), "leaked: {out}");
        assert!(!out.contains("hunter2"), "leaked: {out}");
    }
}
