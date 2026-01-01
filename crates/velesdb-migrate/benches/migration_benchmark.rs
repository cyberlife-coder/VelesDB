//! Benchmarks for velesdb-migrate with real data.
//!
//! Run with: cargo bench -p velesdb-migrate
//!
//! For real data benchmarks, set environment variables:
//! - SUPABASE_URL, SUPABASE_SERVICE_KEY, SUPABASE_TABLE

#![allow(clippy::pedantic)]

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use std::env;

/// Check if real data benchmarks are enabled
fn real_data_enabled() -> bool {
    env::var("SUPABASE_URL").is_ok() && env::var("SUPABASE_SERVICE_KEY").is_ok()
}

/// Benchmark pgvector string parsing (used for Supabase vectors)
fn bench_parse_pgvector(c: &mut Criterion) {
    // Simulate a 1536D vector string (OpenAI embedding)
    let vector_str: String = {
        let values: Vec<String> = (0..1536)
            .map(|i| format!("{:.8}", (i as f32) * 0.001))
            .collect();
        format!("[{}]", values.join(","))
    };
    let _json_value = serde_json::json!(vector_str);

    c.bench_function("parse_pgvector_1536d", |b| {
        b.iter(|| {
            let trimmed = vector_str.trim_start_matches('[').trim_end_matches(']');
            let vec: Vec<f32> = trimmed
                .split(',')
                .filter_map(|x| x.trim().parse().ok())
                .collect();
            black_box(vec)
        })
    });

    // Benchmark different dimensions
    let mut group = c.benchmark_group("pgvector_parse_by_dimension");
    for dim in [384, 768, 1024, 1536, 3072] {
        let vector_str: String = {
            let values: Vec<String> = (0..dim)
                .map(|i| format!("{:.8}", (i as f32) * 0.001))
                .collect();
            format!("[{}]", values.join(","))
        };

        group.bench_with_input(BenchmarkId::new("dimension", dim), &vector_str, |b, s| {
            b.iter(|| {
                let trimmed = s.trim_start_matches('[').trim_end_matches(']');
                let vec: Vec<f32> = trimmed
                    .split(',')
                    .filter_map(|x| x.trim().parse().ok())
                    .collect();
                black_box(vec)
            })
        });
    }
    group.finish();
}

/// Benchmark vector normalization (often needed for cosine similarity)
fn bench_vector_operations(c: &mut Criterion) {
    let vector: Vec<f32> = (0..1536).map(|i| (i as f32) * 0.001).collect();

    c.bench_function("vector_normalize_1536d", |b| {
        b.iter(|| {
            let norm: f32 = vector.iter().map(|x| x * x).sum::<f32>().sqrt();
            let normalized: Vec<f32> = vector.iter().map(|x| x / norm).collect();
            black_box(normalized)
        })
    });

    c.bench_function("vector_dot_product_1536d", |b| {
        let other: Vec<f32> = (0..1536).map(|i| (i as f32) * 0.002).collect();
        b.iter(|| {
            let dot: f32 = vector.iter().zip(other.iter()).map(|(a, b)| a * b).sum();
            black_box(dot)
        })
    });
}

/// Benchmark batch processing simulation
fn bench_batch_processing(c: &mut Criterion) {
    // Simulate batch of 100 vectors with 1536 dimensions
    let batch: Vec<Vec<f32>> = (0..100)
        .map(|_| (0..1536).map(|i| (i as f32) * 0.001).collect())
        .collect();

    c.bench_function("process_batch_100x1536d", |b| {
        b.iter(|| {
            let processed: Vec<Vec<f32>> = batch
                .iter()
                .map(|vec| {
                    let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
                    vec.iter().map(|x| x / norm).collect()
                })
                .collect();
            black_box(processed)
        })
    });

    // Different batch sizes
    let mut group = c.benchmark_group("batch_size_impact");
    for batch_size in [10, 50, 100, 500, 1000] {
        let batch: Vec<Vec<f32>> = (0..batch_size)
            .map(|_| (0..1536).map(|i| (i as f32) * 0.001).collect())
            .collect();

        group.bench_with_input(
            BenchmarkId::new("vectors", batch_size),
            &batch,
            |b, batch| {
                b.iter(|| {
                    let processed: Vec<Vec<f32>> = batch
                        .iter()
                        .map(|vec| {
                            let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
                            vec.iter().map(|x| x / norm).collect()
                        })
                        .collect();
                    black_box(processed)
                })
            },
        );
    }
    group.finish();
}

/// Benchmark JSON payload serialization (metadata handling)
fn bench_payload_serialization(c: &mut Criterion) {
    use std::collections::HashMap;

    let mut payload: HashMap<String, serde_json::Value> = HashMap::new();
    payload.insert(
        "title".to_string(),
        serde_json::json!("Example document title"),
    );
    payload.insert(
        "content".to_string(),
        serde_json::json!(
            "This is a longer content field with more text that simulates real metadata."
        ),
    );
    payload.insert("category".to_string(), serde_json::json!("technology"));
    payload.insert(
        "created_at".to_string(),
        serde_json::json!("2024-01-15T10:30:00Z"),
    );
    payload.insert(
        "tags".to_string(),
        serde_json::json!(["ai", "vectors", "database"]),
    );

    c.bench_function("serialize_payload", |b| {
        b.iter(|| {
            let json = serde_json::to_string(&payload).unwrap();
            black_box(json)
        })
    });

    c.bench_function("deserialize_payload", |b| {
        let json = serde_json::to_string(&payload).unwrap();
        b.iter(|| {
            let parsed: HashMap<String, serde_json::Value> = serde_json::from_str(&json).unwrap();
            black_box(parsed)
        })
    });
}

/// Async benchmarks for real Supabase connection (when enabled)
fn bench_real_supabase_connection(c: &mut Criterion) {
    if !real_data_enabled() {
        println!("⚠️  Skipping real data benchmarks (set SUPABASE_URL and SUPABASE_SERVICE_KEY)");
        return;
    }

    let rt = tokio::runtime::Runtime::new().unwrap();

    let url = env::var("SUPABASE_URL").unwrap();
    let key = env::var("SUPABASE_SERVICE_KEY").unwrap();
    let table = env::var("SUPABASE_TABLE").unwrap_or_else(|_| "embeddings".to_string());

    c.bench_function("supabase_schema_detection", |b| {
        b.to_async(&rt).iter(|| async {
            let config = velesdb_migrate::config::SupabaseConfig {
                url: url.clone(),
                api_key: key.clone(),
                table: table.clone(),
                vector_column: "embedding".to_string(),
                id_column: "id".to_string(),
                payload_columns: vec![],
            };

            let source_config = velesdb_migrate::config::SourceConfig::Supabase(config);
            let mut connector =
                velesdb_migrate::connectors::create_connector(&source_config).unwrap();

            connector.connect().await.unwrap();
            let schema = connector.get_schema().await.unwrap();
            connector.close().await.unwrap();

            black_box(schema)
        })
    });

    // Benchmark batch extraction with different sizes
    let mut group = c.benchmark_group("supabase_batch_extraction");
    group.sample_size(10); // Reduce samples for network calls

    for batch_size in [10, 50, 100] {
        group.bench_with_input(
            BenchmarkId::new("batch_size", batch_size),
            &batch_size,
            |b, &size| {
                b.to_async(&rt).iter(|| async {
                    let config = velesdb_migrate::config::SupabaseConfig {
                        url: url.clone(),
                        api_key: key.clone(),
                        table: table.clone(),
                        vector_column: "embedding".to_string(),
                        id_column: "id".to_string(),
                        payload_columns: vec![],
                    };

                    let source_config = velesdb_migrate::config::SourceConfig::Supabase(config);
                    let mut connector =
                        velesdb_migrate::connectors::create_connector(&source_config).unwrap();

                    connector.connect().await.unwrap();
                    let batch = connector.extract_batch(None, size).await.unwrap();
                    connector.close().await.unwrap();

                    black_box(batch)
                })
            },
        );
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_parse_pgvector,
    bench_vector_operations,
    bench_batch_processing,
    bench_payload_serialization,
    bench_real_supabase_connection,
);

criterion_main!(benches);
