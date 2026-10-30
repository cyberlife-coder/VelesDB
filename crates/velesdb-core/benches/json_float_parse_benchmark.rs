//! Cost of parsing a REST-sized vector body with `serde_json`, which
//! `velesdb-core` builds with the `float_roundtrip` feature.
//!
//! The feature is unified across a build, so the same parse runs in every
//! binary that links core. Run on a commit and on its parent to compare:
//! `cargo bench --package velesdb-core --bench json_float_parse_benchmark`.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use serde::Deserialize;

#[derive(Deserialize)]
struct SearchBody {
    vector: Vec<f32>,
    top_k: usize,
}

/// A 768-dimension query body as a client writes it: every `f32` printed in
/// full, from a fixed xorshift sequence so each run parses the same bytes.
fn query_body() -> String {
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    let vector: Vec<f32> = (0..768)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let mantissa = u32::try_from(state >> 40).expect("24 bits fit in u32");
            #[allow(clippy::cast_precision_loss)] // Reason: 24-bit value, exact in f32.
            let unit = mantissa as f32 / 16_777_216.0;
            unit - 0.5
        })
        .collect();
    serde_json::json!({ "vector": vector, "top_k": 10 }).to_string()
}

fn bench_parse(c: &mut Criterion) {
    let body = query_body();
    let mut group = c.benchmark_group("json_float_parse_768d");
    group.bench_function("typed_vec_f32", |b| {
        b.iter(|| {
            let parsed: SearchBody = serde_json::from_str(black_box(&body)).expect("valid body");
            black_box(parsed.vector.len() + parsed.top_k)
        });
    });
    group.bench_function("value", |b| {
        b.iter(|| {
            let parsed: serde_json::Value =
                serde_json::from_str(black_box(&body)).expect("valid body");
            black_box(parsed)
        });
    });
    group.finish();
}

criterion_group!(benches, bench_parse);
criterion_main!(benches);
