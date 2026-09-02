//! Behaviour: a by-label edge lookup allocates nothing for its key — MEASURED,
//! not asserted from the signature (#2089).
//!
//! Before the fix, `get_outgoing_by_label` / `get_incoming_by_label` built an
//! owned `(node_id, label.to_string())` key per call, so every `-[:TYPE]->` /
//! `<-[:TYPE]-` hop paid a malloc+memcpy+free. The label indices are now keyed
//! by interned `LabelId(u32)`, resolved from `&str` without allocating; this
//! file proves the per-hop allocation is gone with a counting global allocator
//! rather than trusting the code shape.
//!
//! The whole file is ONE test on purpose: the allocator meter is global to
//! the process, so a second test running on a sibling thread would bleed its
//! allocations into whichever measurement is in flight. Sequential sections
//! inside one `#[test]` are the only layout that keeps every delta clean.
//!
//! The positive control is what makes the measurement trustworthy: a HIT on
//! the same store must register at least the materialized result buffer. A
//! meter that failed to see that much would fail the control rather than
//! green-light the claim.

#![cfg(feature = "persistence")]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use velesdb_core::collection::graph::{ConcurrentEdgeStore, EdgeStore, GraphEdge, LabelTable};

/// [`System`] allocator with a monotonic count of bytes ever allocated.
/// Deallocations are deliberately not subtracted: the meter measures
/// allocation WORK (what #2089's per-hop cost is about), not peak footprint.
struct CountingAllocator;

static BYTES_ALLOCATED: AtomicUsize = AtomicUsize::new(0);

// SAFETY: `GlobalAlloc` demands that alloc/dealloc uphold the allocator
// contract (valid layouts honoured, no aliasing invented).
// - Condition 1: every call is forwarded verbatim to `System`, which upholds
//   the full contract by definition.
// - Condition 2: the only addition is a relaxed atomic increment, which
//   cannot allocate, panic, or touch the pointers involved.
// Reason: counting allocated bytes is the whole instrument of this test —
// there is no safe hook into the global allocator.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        BYTES_ALLOCATED.fetch_add(layout.size(), Ordering::Relaxed);
        // SAFETY: `System.alloc` requires the caller's layout obligations.
        // - Condition 1: `layout` is passed through unmodified from our own
        //   caller, who carries the same obligations.
        // Reason: delegation to the real allocator is the entire body.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `System.dealloc` requires `ptr` to come from `System.alloc`
        // with the same layout.
        // - Condition 1: `alloc` above only ever returns `System.alloc`
        //   pointers, and `ptr`/`layout` arrive unmodified from the runtime.
        // Reason: delegation to the real allocator is the entire body.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

/// Bytes allocated (anywhere in the process) while `f` runs.
fn bytes_allocated_during<T>(f: impl FnOnce() -> T) -> (T, usize) {
    let before = BYTES_ALLOCATED.load(Ordering::Relaxed);
    let value = f();
    let after = BYTES_ALLOCATED.load(Ordering::Relaxed);
    (value, after - before)
}

/// Nodes in the test graph — enough that the lookup loop below cannot be
/// satisfied from a single hot cache line by accident.
const NUM_NODES: u64 = 1_000;

/// Lookups per measured section; every one of them must allocate nothing.
const LOOKUPS: u64 = 10_000;

/// A store where every node has one outgoing `KNOWS` edge and one outgoing
/// `LIKES` edge (and therefore one incoming edge of each label too).
fn two_label_store() -> ConcurrentEdgeStore {
    let store = ConcurrentEdgeStore::new();
    for n in 0..NUM_NODES {
        let knows = GraphEdge::new(2 * n, n, (n + 1) % NUM_NODES, "KNOWS").expect("valid label");
        let likes =
            GraphEdge::new(2 * n + 1, n, (n + 2) % NUM_NODES, "LIKES").expect("valid label");
        store.add_edge(knows).expect("edge insert");
        store.add_edge(likes).expect("edge insert");
    }
    store
}

/// Positive control: the meter must SEE a hit's materialized result.
fn assert_meter_sees_hits(store: &ConcurrentEdgeStore) {
    let (hits, hit_bytes) = bytes_allocated_during(|| {
        let mut total = 0u64;
        for n in 0..LOOKUPS {
            total += store.get_outgoing_by_label(n % NUM_NODES, "KNOWS").len() as u64;
        }
        total
    });
    assert_eq!(hits, LOOKUPS, "every node has one KNOWS edge");
    let materialized_floor =
        usize::try_from(LOOKUPS).expect("fits usize") * std::mem::size_of::<GraphEdge>();
    assert!(
        hit_bytes >= materialized_floor,
        "control failed: {LOOKUPS} hits materialize at least their edge \
         buffers ({materialized_floor} bytes), yet the meter saw only \
         {hit_bytes} bytes — the measurement cannot be trusted"
    );
}

/// The claim on the concurrent hop path: misses — an uninterned label, and
/// an interned label on a node without it — allocate nothing.
fn assert_concurrent_misses_are_alloc_free(store: &ConcurrentEdgeStore) {
    let (found, miss_bytes) = bytes_allocated_during(|| {
        let mut total = 0usize;
        for n in 0..LOOKUPS {
            total += store
                .get_outgoing_by_label(n % NUM_NODES, "ABSENT_LABEL")
                .len();
            total += store
                .get_incoming_by_label(n % NUM_NODES, "ABSENT_LABEL")
                .len();
            // Interned label, but no node above NUM_NODES carries any edge.
            total += store.get_outgoing_by_label(NUM_NODES + n, "KNOWS").len();
        }
        total
    });
    assert_eq!(found, 0, "none of the probed (node, label) pairs exist");
    assert_eq!(
        miss_bytes, 0,
        "by-label misses allocated {miss_bytes} bytes across {LOOKUPS} \
         lookup rounds — the per-hop key allocation is back"
    );
}

/// Interning is idempotent without allocating: re-interning an existing
/// label returns the same id and touches no memory.
fn assert_reintern_is_alloc_free() {
    let mut table = LabelTable::new();
    let first = table.intern("KNOWS").expect("intern");
    let (second, reintern_bytes) =
        bytes_allocated_during(|| table.intern("KNOWS").expect("intern"));
    assert_eq!(first, second, "same label, same id");
    assert_eq!(
        reintern_bytes, 0,
        "re-interning an existing label allocated {reintern_bytes} bytes"
    );
}

/// Per-shard `EdgeStore` path (what the concurrent hops delegate to):
/// misses are allocation-free there too, `get_edges_by_label` included.
fn assert_plain_store_misses_are_alloc_free() {
    let mut plain = EdgeStore::new();
    plain
        .add_edge(GraphEdge::new(0, 1, 2, "KNOWS").expect("valid label"))
        .expect("edge insert");
    let (_, plain_miss_bytes) = bytes_allocated_during(|| {
        let mut total = 0usize;
        for _ in 0..LOOKUPS {
            total += plain.get_outgoing_by_label(1, "ABSENT_LABEL").len();
            total += plain.get_incoming_by_label(2, "ABSENT_LABEL").len();
            total += plain.get_edges_by_label("ABSENT_LABEL").len();
        }
        total
    });
    assert_eq!(
        plain_miss_bytes, 0,
        "EdgeStore by-label misses allocated {plain_miss_bytes} bytes"
    );
}

#[test]
fn by_label_lookups_do_not_allocate_per_hop() {
    let store = two_label_store();
    assert_meter_sees_hits(&store);
    assert_concurrent_misses_are_alloc_free(&store);
    assert_reintern_is_alloc_free();
    assert_plain_store_misses_are_alloc_free();
}
