//! A `u64` event counter that concurrent writers do not contend on.
//!
//! A single `AtomicU64` incremented by every thread keeps its cache line
//! moving between cores, which made L1 hits of `LockFreeLruCache` scale
//! negatively with threads (measurements in #2861). This counter splits
//! the count over stripes, each alone on its cache line. A thread always
//! increments the same stripe, picked once per thread round-robin, and a
//! read sums the stripes, so the total stays exact.

use std::cell::Cell;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::OnceLock;

/// One stripe, padded to 128 bytes: the adjacent-line prefetch of x86 and
/// the cache lines of Apple silicon both span 128 bytes.
#[repr(align(128))]
struct Stripe(AtomicU64);

/// Upper bound on stripes per counter, so a many-core host does not give
/// every counter kilobytes of padding.
const MAX_STRIPES: usize = 64;

pub(super) struct StripedCounter {
    stripes: Box<[Stripe]>,
}

impl StripedCounter {
    pub(super) fn new() -> Self {
        Self {
            stripes: (0..stripe_count())
                .map(|_| Stripe(AtomicU64::new(0)))
                .collect(),
        }
    }

    pub(super) fn increment(&self) {
        // `stripe_count` is a power of two, so the mask is a modulo.
        let stripe = &self.stripes[thread_stripe_hint() & (self.stripes.len() - 1)];
        stripe.0.fetch_add(1, Ordering::Relaxed);
    }

    /// The total over all stripes. Exact once writers are quiescent; while
    /// they run, it is some value between the counts before and after.
    pub(super) fn get(&self) -> u64 {
        self.stripes
            .iter()
            .map(|stripe| stripe.0.load(Ordering::Relaxed))
            .sum()
    }

    #[cfg(test)]
    pub(super) fn stripe_size() -> usize {
        std::mem::size_of::<Stripe>()
    }
}

/// The host's parallelism rounded up to a power of two, capped at
/// `MAX_STRIPES`; 1 where the parallelism is unknown (e.g. wasm32).
fn stripe_count() -> usize {
    static COUNT: OnceLock<usize> = OnceLock::new();
    *COUNT.get_or_init(|| {
        std::thread::available_parallelism()
            .map_or(1, usize::from)
            .next_power_of_two()
            .min(MAX_STRIPES)
    })
}

/// A per-thread number handed out round-robin on first use, so the threads
/// of a process spread evenly over the stripes.
fn thread_stripe_hint() -> usize {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    thread_local! {
        static HINT: Cell<Option<usize>> = const { Cell::new(None) };
    }
    HINT.with(|hint| {
        hint.get().unwrap_or_else(|| {
            let assigned = NEXT.fetch_add(1, Ordering::Relaxed);
            hint.set(Some(assigned));
            assigned
        })
    })
}
