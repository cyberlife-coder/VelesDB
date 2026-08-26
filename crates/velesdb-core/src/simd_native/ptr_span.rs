//! Remaining-length checks for the pointer-walked SIMD loops.
//!
//! The kernels in this module tree walk a pair of `*const f32` cursors towards
//! an `end_ptr` one past the last element, consuming a lane's worth per
//! iteration. The guard they used to spell that with was:
//!
//! ```text
//! while a_ptr.add(16) <= end_ptr { … }
//! ```
//!
//! which is undefined behaviour on its final evaluation, and the fact that the
//! result is only compared and never dereferenced does not save it.
//! `pointer::add` requires the *computed* pointer to stay inside the same
//! allocated object (one past the end included). On the last check — the one
//! that ends the loop — fewer than 16 elements remain, so `a_ptr.add(16)`
//! lands beyond one-past-the-end and the call is UB before the comparison
//! happens. Miri reports it as an out-of-bounds pointer computation.
//!
//! In practice every compiler in use today emits the obvious address
//! comparison, which is why this never produced a wrong answer. That is not a
//! guarantee: LLVM is entitled to assume the pointer is in bounds, and an
//! `inbounds` GEP feeding a comparison is exactly the shape a future
//! optimisation is allowed to fold. The bug is that the code asks a question
//! it has no right to ask, not that anyone has yet answered it wrongly.
//!
//! [`has_at_least`] asks the same question about the *distance* between the
//! two cursors instead, on integers, where the arithmetic is defined for every
//! input. The index-form guards elsewhere in the tree (`i + 16 <= len`) are
//! the same idea; this is that idea for the sites that carry pointers rather
//! than indices, so neither has to be rewritten into the other.

/// Whether at least `count` values of `T` remain between `cur` and `end`.
///
/// `cur` and `end` must delimit a range of `T` — `end` one past its last
/// element — and both must derive from the same allocation. That is the same
/// precondition the callers already rely on; nothing here reads either
/// pointer, so violating it yields a wrong answer rather than UB.
///
/// The subtraction saturates rather than wrapping so that a cursor past `end`,
/// which the callers' loop structure makes unreachable, reports "nothing
/// remains" and stops the loop instead of computing an enormous span and
/// running off the buffer. Failing closed is the only safe direction here.
#[inline]
pub(crate) fn has_at_least<T>(cur: *const T, end: *const T, count: usize) -> bool {
    end.addr().saturating_sub(cur.addr()) >= count * core::mem::size_of::<T>()
}
