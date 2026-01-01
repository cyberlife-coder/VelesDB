//! Integration tests for velesdb-migrate with real data sources.
//!
//! These tests require environment variables to be set:
//! - `SUPABASE_URL`: Your Supabase project URL
//! - `SUPABASE_SERVICE_KEY`: Your Supabase service role key
//! - `SUPABASE_TABLE`: The table name to test with
//!
//! Run with: `cargo test --test integration_test -- --ignored`

#![allow(clippy::pedantic)]

use std::env;
use tempfile::TempDir;

/// Helper to check if real data tests are enabled
fn real_data_enabled() -> bool {
    env::var("SUPABASE_URL").is_ok() && env::var("SUPABASE_SERVICE_KEY").is_ok()
}

/// Get Supabase config from environment
fn get_supabase_config() -> Option<(String, String, String)> {
    let url = env::var("SUPABASE_URL").ok()?;
    let key = env::var("SUPABASE_SERVICE_KEY").ok()?;
    let table = env::var("SUPABASE_TABLE").ok()?;
    Some((url, key, table))
}

#[tokio::test]
#[ignore] // Run with --ignored flag when env vars are set
async fn test_supabase_connection() {
    if !real_data_enabled() {
        eprintln!("Skipping: SUPABASE_URL and SUPABASE_SERVICE_KEY not set");
        return;
    }

    let (url, key, table) = get_supabase_config().unwrap();

    // Create connector config
    let config = velesdb_migrate::config::SupabaseConfig {
        url,
        api_key: key,
        table,
        vector_column: env::var("SUPABASE_VECTOR_COL").unwrap_or_else(|_| "embedding".to_string()),
        id_column: env::var("SUPABASE_ID_COL").unwrap_or_else(|_| "id".to_string()),
        payload_columns: vec![],
    };

    // Test connection
    let source_config = velesdb_migrate::config::SourceConfig::Supabase(config);
    let mut connector = velesdb_migrate::connectors::create_connector(&source_config)
        .expect("Failed to create connector");

    connector.connect().await.expect("Failed to connect");

    let schema = connector.get_schema().await.expect("Failed to get schema");

    println!("✅ Connected to Supabase!");
    println!("   Collection: {}", schema.collection);
    println!("   Dimension: {}", schema.dimension);
    println!("   Total count: {:?}", schema.total_count);
    println!("   Fields: {}", schema.fields.len());

    assert!(schema.dimension > 0, "Dimension should be detected");
    assert!(schema.total_count.unwrap_or(0) > 0, "Should have vectors");

    connector.close().await.expect("Failed to close");
}

#[tokio::test]
#[ignore]
async fn test_supabase_extract_batch() {
    if !real_data_enabled() {
        return;
    }

    let (url, key, table) = get_supabase_config().unwrap();

    let config = velesdb_migrate::config::SupabaseConfig {
        url,
        api_key: key,
        table,
        vector_column: env::var("SUPABASE_VECTOR_COL").unwrap_or_else(|_| "embedding".to_string()),
        id_column: env::var("SUPABASE_ID_COL").unwrap_or_else(|_| "id".to_string()),
        payload_columns: vec!["information_type".to_string(), "text_content".to_string()],
    };

    let source_config = velesdb_migrate::config::SourceConfig::Supabase(config);
    let mut connector = velesdb_migrate::connectors::create_connector(&source_config).unwrap();

    connector.connect().await.unwrap();

    // Extract first batch
    let batch = connector
        .extract_batch(None, 10)
        .await
        .expect("Failed to extract batch");

    println!("✅ Extracted {} points", batch.points.len());

    assert!(!batch.points.is_empty(), "Should extract some points");

    // Verify vector dimension
    if let Some(first) = batch.points.first() {
        println!("   First point ID: {}", first.id);
        println!("   Vector dimension: {}", first.vector.len());
        println!(
            "   Payload keys: {:?}",
            first.payload.keys().collect::<Vec<_>>()
        );

        assert!(
            first.vector.len() > 100,
            "Vector should have many dimensions"
        );
    }

    connector.close().await.unwrap();
}

#[tokio::test]
#[ignore]
async fn test_full_migration_to_velesdb() {
    if !real_data_enabled() {
        return;
    }

    let (url, key, table) = get_supabase_config().unwrap();

    // Create temp directory for VelesDB data
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let _dest_path = temp_dir.path().to_path_buf();

    let config = velesdb_migrate::config::SupabaseConfig {
        url,
        api_key: key,
        table: table.clone(),
        vector_column: env::var("SUPABASE_VECTOR_COL").unwrap_or_else(|_| "embedding".to_string()),
        id_column: env::var("SUPABASE_ID_COL").unwrap_or_else(|_| "id".to_string()),
        payload_columns: vec!["information_type".to_string(), "text_content".to_string()],
    };

    let source_config = velesdb_migrate::config::SourceConfig::Supabase(config);
    let mut connector = velesdb_migrate::connectors::create_connector(&source_config).unwrap();

    connector.connect().await.unwrap();
    let schema = connector.get_schema().await.unwrap();

    println!(
        "📊 Source schema: {}D, {:?} vectors",
        schema.dimension, schema.total_count
    );

    // Extract limited data for test (100 vectors)
    let batch = connector.extract_batch(None, 100).await.unwrap();
    connector.close().await.unwrap();

    println!("📥 Extracted {} vectors for test", batch.points.len());

    // Now test loading into VelesDB
    // This would require velesdb-core integration
    // For now, just verify the data is valid

    for point in &batch.points {
        assert!(!point.id.is_empty(), "Point should have ID");
        assert_eq!(
            point.vector.len(),
            schema.dimension,
            "Vector dimension should match"
        );
        assert!(
            point.vector.iter().all(|v| v.is_finite()),
            "Vector values should be finite"
        );
    }

    println!("✅ All {} vectors validated!", batch.points.len());
}

/// Test dimension detection accuracy
#[tokio::test]
#[ignore]
async fn test_dimension_detection_accuracy() {
    if !real_data_enabled() {
        return;
    }

    let (url, key, table) = get_supabase_config().unwrap();

    let config = velesdb_migrate::config::SupabaseConfig {
        url,
        api_key: key,
        table,
        vector_column: env::var("SUPABASE_VECTOR_COL").unwrap_or_else(|_| "embedding".to_string()),
        id_column: env::var("SUPABASE_ID_COL").unwrap_or_else(|_| "id".to_string()),
        payload_columns: vec![],
    };

    let source_config = velesdb_migrate::config::SourceConfig::Supabase(config);
    let mut connector = velesdb_migrate::connectors::create_connector(&source_config).unwrap();

    connector.connect().await.unwrap();
    let schema = connector.get_schema().await.unwrap();

    // Extract a batch to verify dimension
    let batch = connector.extract_batch(None, 5).await.unwrap();
    connector.close().await.unwrap();

    // Verify all vectors have the same dimension as detected
    for point in &batch.points {
        assert_eq!(
            point.vector.len(),
            schema.dimension,
            "Vector dimension mismatch: expected {}, got {}",
            schema.dimension,
            point.vector.len()
        );
    }

    // Common dimensions check
    let common_dimensions = [384, 768, 1024, 1536, 3072];
    assert!(
        common_dimensions.contains(&schema.dimension),
        "Dimension {} should be a common embedding size",
        schema.dimension
    );

    println!("✅ Dimension detection accurate: {}D", schema.dimension);
}
