//! Autograph write cost as one entity hub accumulates facts (issue #1790).
//!
//! Each batch starts from an empty native store and writes `n` unique facts
//! that the deterministic extractor maps to the same entity. Setup and fact
//! construction stay outside the timed/allocation-counted section, so the
//! measurement covers `remember -> autograph_if -> wire_entities ->
//! wire_entity` without model latency or fixture construction.
//!
//! Run sequentially on an otherwise idle machine:
//!
//! ```sh
//! cargo bench -p velesdb-memory --bench autograph_write_benchmark
//! ```
//!
//! The `autograph_allocations` lines report allocation calls and bytes for
//! the same batch sizes Criterion reports. Compare revisions with the exact
//! same harness; the per-write values and their slope are the useful signals.
//! Native-store latency includes the durable write and can therefore be
//! dominated by WAL fsync; do not attribute sub-noise timing differences to
//! graph wiring when the allocation slope is the only discriminating signal.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput};
use tempfile::TempDir;
use velesdb_memory::{
    ExtractError, ExtractedFact, Extractor, HashEmbedder, MemoryService, DEFAULT_DIMENSION,
};

const ENTITY: &str = "benchmark-topic";
const HUB_CONTENT: &str = "Entity: benchmark-topic";
const BATCH_SIZES: [usize; 5] = [10, 50, 100, 250, 500];

struct CountingAllocator;

static ALLOCATION_CALLS: AtomicUsize = AtomicUsize::new(0);
static ALLOCATED_BYTES: AtomicUsize = AtomicUsize::new(0);

// SAFETY: every allocation operation is forwarded unchanged to `System`.
// The relaxed counter increments neither inspect nor modify allocated memory.
// - Condition 1: every layout and pointer reaches `System` unchanged.
// - Condition 2: the atomic counters cannot allocate or alias the pointer.
// Reason: the benchmark needs process-wide allocation counts.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATION_CALLS.fetch_add(1, Ordering::Relaxed);
        ALLOCATED_BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        // SAFETY: `layout` is forwarded unchanged from this method's caller.
        // - Condition 1: the caller satisfies `GlobalAlloc::alloc`'s layout
        //   contract, which is identical to `System::alloc`'s contract.
        // Reason: `System` remains the allocator; this wrapper only counts.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` and `layout` are forwarded to the allocator that
        // produced the pointer in `alloc` above.
        // - Condition 1: the runtime supplies the same pointer and layout
        //   pair that came from `System::alloc`.
        // Reason: `System` must release the allocation it produced.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

struct SharedEntityExtractor;

impl Extractor for SharedEntityExtractor {
    fn extract(&self, text: &str) -> Result<Vec<ExtractedFact>, ExtractError> {
        Ok(vec![ExtractedFact {
            text: text.to_owned(),
            entities: vec![ENTITY.to_owned()],
        }])
    }
}

type Fixture = (TempDir, MemoryService<HashEmbedder>, Vec<String>);

fn fixture(n: usize) -> Fixture {
    let dir = TempDir::new().expect("create benchmark store");
    let service = MemoryService::open(dir.path(), HashEmbedder::new(DEFAULT_DIMENSION))
        .expect("open benchmark store")
        .with_autograph(Arc::new(SharedEntityExtractor));
    let facts = (0..n)
        .map(|index| format!("benchmark fact {index} about the shared topic"))
        .collect();
    (dir, service, facts)
}

fn write_batch(service: &MemoryService<HashEmbedder>, facts: &[String]) {
    for fact in facts {
        service.remember(fact, &[], None).expect("remember fact");
    }
}

fn hub_reached_from(service: &MemoryService<HashEmbedder>, fact: &str) -> u64 {
    service
        .why(fact, 1, None)
        .expect("walk autograph edge")
        .nodes
        .into_iter()
        .find(|node| node.content == HUB_CONTENT)
        .expect("shared entity hub must be reachable")
        .id
}

fn assert_batch_wired(service: &MemoryService<HashEmbedder>, facts: &[String]) {
    let first = facts.first().expect("non-empty benchmark batch");
    let last = facts.last().expect("non-empty benchmark batch");
    assert_eq!(
        hub_reached_from(service, first),
        hub_reached_from(service, last),
        "the first and last facts must reach the same entity hub"
    );
}

fn reset_allocation_counters() {
    ALLOCATION_CALLS.store(0, Ordering::Relaxed);
    ALLOCATED_BYTES.store(0, Ordering::Relaxed);
}

fn report_allocations() {
    for n in BATCH_SIZES {
        let (_dir, service, facts) = fixture(n);
        reset_allocation_counters();
        write_batch(&service, &facts);
        let calls = ALLOCATION_CALLS.load(Ordering::Relaxed);
        let bytes = ALLOCATED_BYTES.load(Ordering::Relaxed);
        assert_batch_wired(&service, &facts);
        println!(
            "autograph_allocations n={n} calls={calls} calls_per_write={} bytes={bytes} bytes_per_write={}",
            calls / n,
            bytes / n
        );
    }
}

fn benchmark_latency(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("autograph_write_shared_entity");
    for n in BATCH_SIZES {
        let (_dir, service, facts) = fixture(n);
        write_batch(&service, &facts);
        assert_batch_wired(&service, &facts);
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |bencher, &size| {
            bencher.iter_batched(
                || fixture(size),
                |fixture| {
                    write_batch(&fixture.1, &fixture.2);
                    fixture
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

fn main() {
    report_allocations();
    let mut criterion = Criterion::default()
        .sample_size(10)
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(3))
        .configure_from_args();
    benchmark_latency(&mut criterion);
    criterion.final_summary();
}
