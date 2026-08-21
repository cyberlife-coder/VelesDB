//! Behaviour: reading a bounded prefix of a super-node's edges allocates
//! O(cap), not O(degree) — MEASURED, not asserted from the signature (#1820).
//!
//! Issue #1743 bounded the RESPONSE of a graph walk, but the expansion cost
//! stayed O(degree): `get_outgoing` materializes a node's whole edge list
//! before any caller-side `.take(cap)` can apply, so one expansion of a
//! million-edge hub still cost a million clones transiently. The bounded
//! accessors push the cap into the shard's index scan itself; this file
//! proves the difference with a counting global allocator rather than
//! trusting the code shape.
//!
//! The whole file is ONE test on purpose: the allocator meter is global to
//! the process, so a second test running on a sibling thread would bleed its
//! allocations into whichever measurement is in flight. Sequential sections
//! inside one `#[test]` are the only layout that keeps every delta clean.
//!
//! The positive control is what makes the measurement trustworthy: the
//! UNBOUNDED read of the same hub must register at least the materialized
//! edge buffer (`degree × size_of::<GraphEdge>()`). A meter that failed to
//! see that much would fail the control rather than green-light the claim.

#![cfg(feature = "persistence")]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use velesdb_core::collection::graph::{ConcurrentEdgeStore, GraphEdge};

/// [`System`] allocator with a monotonic count of bytes ever allocated.
/// Deallocations are deliberately not subtracted: the meter measures
/// allocation WORK (what #1820's residual is about), not peak footprint.
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

/// The hub's degree. Large enough that O(degree) and O(cap) differ by
/// orders of magnitude, small enough to build in well under a second.
const HUB_DEGREE: usize = 100_000;

/// The bounded read's cap — the same order as the caps the memory layer
/// applies (`MAX_WHY_NODE_DEGREE`, 64).
const CAP: usize = 64;

/// The hub node every edge leaves from.
const HUB: u64 = 1;

/// A store holding one hub with [`HUB_DEGREE`] outgoing edges, each pointing
/// at a distinct target (which therefore carries one incoming edge from the
/// hub — the incoming side is measured on the hub's own incoming edges,
/// added separately below).
fn dense_hub_store() -> ConcurrentEdgeStore {
    let store = ConcurrentEdgeStore::new();
    for i in 0..HUB_DEGREE {
        let id = u64::try_from(i).expect("hub degree fits u64");
        let edge = GraphEdge::new(id, HUB, 1_000_000 + id, "mentions").expect("valid label");
        store.add_edge(edge).expect("edge insert");
    }
    (0..HUB_DEGREE).for_each(|i| {
        let id = u64::try_from(HUB_DEGREE + i).expect("hub degree fits u64");
        let edge = GraphEdge::new(id, 2_000_000 + id, HUB, "mentions").expect("valid label");
        store.add_edge(edge).expect("edge insert");
    });
    store
}

/// How much smaller than the unbounded read's allocation the bounded read
/// must measure. 100k-edge full reads allocate megabytes; 64-edge bounded
/// reads allocate kilobytes — a 50× margin leaves room for allocator noise
/// while still failing on any O(degree) regression.
const REQUIRED_FACTOR: usize = 50;

#[test]
fn a_bounded_read_of_a_dense_hub_allocates_o_cap_not_o_degree() {
    let store = dense_hub_store();

    // --- Positive control: the meter must SEE the O(degree) materialization.
    let (full, full_bytes) = bytes_allocated_during(|| store.get_outgoing(HUB));
    assert_eq!(full.len(), HUB_DEGREE, "the hub holds its whole degree");
    let materialized_floor = HUB_DEGREE * std::mem::size_of::<GraphEdge>();
    assert!(
        full_bytes >= materialized_floor,
        "control failed: the unbounded read materializes at least its edge \
         buffer ({materialized_floor} bytes), yet the meter saw only \
         {full_bytes} — the measurement cannot be trusted"
    );
    drop(full);

    // --- The claim, outgoing: O(cap) allocation, exact degree reported.
    let ((bounded, total), bounded_bytes) =
        bytes_allocated_during(|| store.get_outgoing_bounded(HUB, CAP));
    assert_eq!(bounded.len(), CAP, "cap honoured");
    assert_eq!(total, HUB_DEGREE, "total degree reported exactly");
    assert!(
        bounded_bytes * REQUIRED_FACTOR < full_bytes,
        "bounded read allocated {bounded_bytes} bytes — not even {REQUIRED_FACTOR}x \
         under the unbounded read's {full_bytes}: the cap is not reaching the scan"
    );

    // --- The claim, incoming: same contract on the mirror accessor.
    let (incoming_full, incoming_full_bytes) = bytes_allocated_during(|| store.get_incoming(HUB));
    assert_eq!(incoming_full.len(), HUB_DEGREE);
    drop(incoming_full);
    let ((bounded_in, total_in), bounded_in_bytes) =
        bytes_allocated_during(|| store.get_incoming_bounded(HUB, CAP));
    assert_eq!(bounded_in.len(), CAP, "incoming cap honoured");
    assert_eq!(
        total_in, HUB_DEGREE,
        "incoming total degree reported exactly"
    );
    assert!(
        bounded_in_bytes * REQUIRED_FACTOR < incoming_full_bytes,
        "bounded incoming read allocated {bounded_in_bytes} bytes against the \
         unbounded read's {incoming_full_bytes}: the cap is not reaching the scan"
    );

    // --- Honesty of the signal: under-cap nodes are NOT reported truncated.
    let first_source = 2_000_000 + u64::try_from(HUB_DEGREE).expect("fits u64");
    let (small, small_total) = store.get_outgoing_bounded(first_source, CAP);
    assert_eq!(small.len(), 1, "a leaf has exactly its one edge");
    assert_eq!(small_total, 1, "a leaf's degree is under the cap");

    // --- The bounded prefix is a prefix of the unbounded read, not a resample.
    let full_again = store.get_outgoing(HUB);
    let prefix: Vec<u64> = full_again.iter().take(CAP).map(GraphEdge::id).collect();
    let bounded_ids: Vec<u64> = bounded.iter().map(GraphEdge::id).collect();
    assert_eq!(
        bounded_ids, prefix,
        "bounded returns the same leading edges"
    );
}
