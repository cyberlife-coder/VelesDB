//! Steady-state cost of the no-capture service-generation guard.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use velesdb_memory::{FactStore, HashEmbedder, MemoryService, NativeStore};

const DIMENSION: usize = 4;

fn generation_gate(c: &mut Criterion) {
    let direct_dir = tempfile::tempdir().expect("direct tempdir");
    let direct = NativeStore::open(direct_dir.path(), DIMENSION).expect("direct store");
    let service_dir = tempfile::tempdir().expect("service tempdir");
    let service = MemoryService::open(service_dir.path(), HashEmbedder::new(DIMENSION))
        .expect("guarded service");
    let mut group = c.benchmark_group("generation_gate_no_capture");

    group.bench_function("native_count_direct", |b| {
        b.iter(|| black_box(direct.count()));
    });
    group.bench_function("service_count_guarded", |b| {
        b.iter(|| black_box(service.fact_count()));
    });
    group.finish();
}

criterion_group!(benches, generation_gate);
criterion_main!(benches);
