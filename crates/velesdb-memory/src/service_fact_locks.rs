//! Per-fact write serialization — the service's answer to read-modify-write
//! races on one fact's payload. A `#[path]` child of `service`.
//!
//! `feedback` reads a fact's learned confidence and tallies, computes the
//! update, and writes it back; a re-store (`remember` of the same content)
//! reads the fact's reserved `_veles_*` state and carries it into the new
//! payload. Either one interleaved with another on the same fact loses the
//! first write. Holding the fact's lock across the read and the write makes
//! each of them atomic per fact.
//!
//! This is the complete defense, not a partial one: the store is opened by a
//! single process (`velesdb-core` holds an exclusive `flock` on
//! `velesdb.lock` for the `Database`'s lifetime — see
//! `tests/working_index_two_daemons_process.rs`), so every writer of a fact
//! runs inside this service.
//!
//! Locks are striped rather than one per id: a fixed table, so memory stays
//! constant however many facts exist, and two facts share a stripe only by
//! hash collision, which costs a wait and never correctness. Ids are
//! content hashes, so the low bits spread evenly.
//!
//! Lock order: the generation gate (shared) first, then one stripe; no code
//! path holds two stripes or takes the gate under a stripe.

/// Number of stripes. A divisor of 256, so the low byte of an id spreads
/// evenly over them.
const STRIPES: usize = 64;

/// The stripe table.
pub(super) struct FactWriteLocks([parking_lot::Mutex<()>; STRIPES]);

impl Default for FactWriteLocks {
    fn default() -> Self {
        Self(std::array::from_fn(|_| parking_lot::Mutex::new(())))
    }
}

impl FactWriteLocks {
    /// Hold `id`'s stripe until the guard drops.
    pub(super) fn for_fact(&self, id: u64) -> parking_lot::MutexGuard<'_, ()> {
        // The low byte of the id picks the stripe; STRIPES divides 256, so
        // every stripe is reached equally often.
        let stripe = usize::from(id.to_le_bytes()[0]) % STRIPES;
        self.0[stripe].lock()
    }
}
