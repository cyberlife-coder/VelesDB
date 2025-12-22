//! Benchmark suite for BM25 full-text search operations.
//!
//! Run with: `cargo bench --bench bm25_benchmark`
//!
//! Tests performance of:
//! - `Bm25Index` tokenization
//! - `Bm25Index::add_document`
//! - `Bm25Index::search` (BM25 scoring)
//! - `Collection::text_search`
//! - `Collection::hybrid_search` (RRF fusion)

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use serde_json::json;
use tempfile::tempdir;
use velesdb_core::distance::DistanceMetric;
use velesdb_core::index::Bm25Index;
use velesdb_core::{Collection, Point};

/// Sample documents for benchmarking
const SAMPLE_DOCS: &[&str] = &[
    "Rust is a systems programming language focused on safety and performance",
    "Python is great for machine learning and data science applications",
    "JavaScript powers the modern web with frameworks like React and Vue",
    "Go is designed for building scalable network services and cloud infrastructure",
    "TypeScript adds static typing to JavaScript for better tooling",
    "C++ remains the choice for high-performance game engines and systems",
    "Java enterprise applications run on billions of devices worldwide",
    "Swift enables iOS and macOS development with modern syntax",
    "Kotlin is the preferred language for Android development",
    "Ruby on Rails revolutionized web development with convention over configuration",
];

/// Generate more documents by combining sample docs
fn generate_documents(count: usize) -> Vec<String> {
    SAMPLE_DOCS
        .iter()
        .cycle()
        .take(count)
        .enumerate()
        .map(|(i, doc)| format!("Document {i} - {doc}"))
        .collect()
}

/// Benchmark `Bm25Index::add_document`
fn bench_bm25_add_document(c: &mut Criterion) {
    let mut group = c.benchmark_group("bm25_add_document");

    for doc_count in [100, 1000, 10000] {
        let docs = generate_documents(doc_count);

        group.throughput(Throughput::Elements(doc_count as u64));
        group.bench_function(BenchmarkId::new("add_docs", doc_count), |b| {
            b.iter(|| {
                let index = Bm25Index::new();
                for (id, doc) in docs.iter().enumerate() {
                    index.add_document(id as u64, doc);
                }
                black_box(index.len())
            });
        });
    }

    group.finish();
}

/// Benchmark `Bm25Index::search` with varying index sizes
fn bench_bm25_search(c: &mut Criterion) {
    let mut group = c.benchmark_group("bm25_search");

    for doc_count in [100, 1000, 10000] {
        // Pre-build index
        let index = Bm25Index::new();
        let docs = generate_documents(doc_count);
        for (id, doc) in docs.iter().enumerate() {
            index.add_document(id as u64, doc);
        }

        group.bench_function(BenchmarkId::new("search_single_term", doc_count), |b| {
            b.iter(|| black_box(index.search("rust", 10)));
        });

        group.bench_function(BenchmarkId::new("search_multi_term", doc_count), |b| {
            b.iter(|| black_box(index.search("rust programming language", 10)));
        });

        group.bench_function(BenchmarkId::new("search_no_match", doc_count), |b| {
            b.iter(|| black_box(index.search("xyznonexistent", 10)));
        });
    }

    group.finish();
}

/// Benchmark `Collection::text_search`
fn bench_collection_text_search(c: &mut Criterion) {
    let mut group = c.benchmark_group("collection_text_search");

    for doc_count in [100, 1000] {
        let dir = tempdir().unwrap();
        let path = dir.path().join("bench_collection");
        let collection = Collection::create(path, 128, DistanceMetric::Cosine).unwrap();

        // Insert points with text payloads
        let points: Vec<Point> = generate_documents(doc_count)
            .into_iter()
            .enumerate()
            .map(|(id, text)| {
                Point::new(
                    id as u64,
                    vec![0.1; 128], // Dummy vector
                    Some(json!({"content": text})),
                )
            })
            .collect();
        collection.upsert(points).unwrap();

        group.bench_function(BenchmarkId::new("text_search", doc_count), |b| {
            b.iter(|| black_box(collection.text_search("rust programming", 10)));
        });
    }

    group.finish();
}

/// Benchmark `Collection::hybrid_search` (vector + BM25 with RRF)
fn bench_collection_hybrid_search(c: &mut Criterion) {
    let mut group = c.benchmark_group("collection_hybrid_search");

    for doc_count in [100, 1000] {
        let dir = tempdir().unwrap();
        let path = dir.path().join("bench_collection");
        let collection = Collection::create(path, 128, DistanceMetric::Cosine).unwrap();

        // Insert points with text payloads and varied vectors
        let points: Vec<Point> = generate_documents(doc_count)
            .into_iter()
            .enumerate()
            .map(|(id, text)| {
                #[allow(clippy::cast_precision_loss)]
                let vector: Vec<f32> = (0..128).map(|i| ((id + i) as f32 * 0.01).sin()).collect();
                Point::new(id as u64, vector, Some(json!({"content": text})))
            })
            .collect();
        collection.upsert(points).unwrap();

        let query_vector: Vec<f32> = vec![0.1; 128];

        group.bench_function(BenchmarkId::new("hybrid_search", doc_count), |b| {
            b.iter(|| {
                black_box(
                    collection
                        .hybrid_search(&query_vector, "rust programming", 10, Some(0.5))
                        .unwrap(),
                )
            });
        });

        // Compare with pure vector search
        group.bench_function(BenchmarkId::new("vector_only", doc_count), |b| {
            b.iter(|| black_box(collection.search(&query_vector, 10).unwrap()));
        });

        // Compare with pure text search
        group.bench_function(BenchmarkId::new("text_only", doc_count), |b| {
            b.iter(|| black_box(collection.text_search("rust programming", 10)));
        });
    }

    group.finish();
}

/// Benchmark tokenization (internal, via `add_document` with empty index)
fn bench_tokenization(c: &mut Criterion) {
    let mut group = c.benchmark_group("bm25_tokenization");

    let short_text = "Rust programming";
    let medium_text =
        "Rust is a systems programming language focused on safety, speed, and concurrency";
    let long_text = "Rust is a multi-paradigm, general-purpose programming language that emphasizes performance, type safety, and concurrency. It enforces memory safety, meaning that all references point to valid memory, without a garbage collector. To simultaneously enforce memory safety and prevent data races, its type system distinguishes between mutable and shared references.";

    group.bench_function("short_text", |b| {
        b.iter(|| {
            let index = Bm25Index::new();
            index.add_document(1, black_box(short_text));
            black_box(index.len())
        });
    });

    group.bench_function("medium_text", |b| {
        b.iter(|| {
            let index = Bm25Index::new();
            index.add_document(1, black_box(medium_text));
            black_box(index.len())
        });
    });

    group.bench_function("long_text", |b| {
        b.iter(|| {
            let index = Bm25Index::new();
            index.add_document(1, black_box(long_text));
            black_box(index.len())
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_tokenization,
    bench_bm25_add_document,
    bench_bm25_search,
    bench_collection_text_search,
    bench_collection_hybrid_search,
);
criterion_main!(benches);
