//! The collection's locks, and the unit-test check of the rule that keeps
//! them from deadlocking against a rayon pool.
//!
//! The rule (`docs/CONCURRENCY_MODEL.md`, "Collection locks and rayon"):
//!
//! 1. no job on one of core's rayon pools takes a collection lock, and
//! 2. no thread holding a collection lock waits on a pool, except a thread
//!    that is no rayon worker waiting on one of core's dedicated pools, whose
//!    jobs take none.
//!
//! Breaking either closes the cycle of \#2343 one lock lower: a thread
//! holding `vector_storage`'s read guard waits for jobs that cannot run, a
//! writer queues behind that guard, and `parking_lot` then parks every new
//! reader, including whatever job the holder waits for. `crate::pools` is
//! the only place core submits work to a pool, and it holds rule 2 for every
//! submission: its parallel helpers assert this module's
//! [`assert_no_guard_held`], and its dedicated pools run their sequential
//! form on any caller that is a rayon worker.
//!
//! [`RwLock`] and [`Mutex`] wrap `parking_lot`'s for every lock a
//! `Collection` owns, so every accessor goes through [`RwLock::read`],
//! [`RwLock::write`] or [`Mutex::lock`]. Under `cfg(test)` each of them
//! panics when called on a rayon worker of any pool but a test's stand-in
//! for an application's (rule 1; an application's own jobs may call into a
//! collection), and each guard counts itself, in a thread-local, while it
//! lives, which is what [`assert_no_guard_held`] reads. Outside `cfg(test)`
//! the wrappers are the `parking_lot` locks and guards with nothing added:
//! the check costs release builds nothing, and an application calling a
//! collection from its own rayon jobs is served, not refused.
//!
//! Locks nested inside a field's value — `Bm25Index`'s, `PayloadMirror`'s,
//! `SecondaryIndex::BTree`'s, `QueryCache`'s, the edge store's shards — are
//! their types' own and outside this audit.

use std::ops::{Deref, DerefMut};

#[cfg(all(test, feature = "persistence"))]
#[path = "lock_audit_tests.rs"]
mod lock_audit_tests;

#[cfg(test)]
thread_local! {
    /// Collection lock guards this thread holds now.
    static HELD: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// One live guard on this thread, counted under `cfg(test)`.
struct Held(());

impl Held {
    #[track_caller]
    fn acquire() -> Self {
        // An application's own jobs may call into a collection.
        #[cfg(test)]
        assert!(
            !crate::pools::on_a_rayon_worker() || crate::pools::on_application_thread(),
            "a collection lock was acquired on a rayon worker: no job on a core pool may \
             take a collection lock (docs/CONCURRENCY_MODEL.md, \"Collection locks and \
             rayon\")"
        );
        #[cfg(test)]
        HELD.with(|held| held.set(held.get() + 1));
        Self(())
    }
}

#[cfg(test)]
impl Drop for Held {
    fn drop(&mut self) {
        HELD.with(|held| held.set(held.get() - 1));
    }
}

/// Panics, under `cfg(test)`, when this thread holds a collection lock guard.
///
/// Called by every parallel helper of `crate::pools`: a thread that waits on
/// its own pool must hold no collection lock. Compiles to nothing outside
/// unit tests.
#[inline]
#[track_caller]
pub(crate) fn assert_no_guard_held() {
    #[cfg(test)]
    HELD.with(|held| {
        assert!(
            held.get() == 0,
            "{} collection lock guard(s) held across a parallel submission: copy what \
             the jobs need out first, or run them through pools::on_guarded_pool \
             (docs/CONCURRENCY_MODEL.md, \"Collection locks and rayon\")",
            held.get()
        );
    });
}

/// The number of collection lock guards this thread holds now.
#[cfg(test)]
pub(crate) fn guards_held() -> usize {
    HELD.with(std::cell::Cell::get)
}

/// A collection's reader-writer lock: `parking_lot`'s, checked under
/// `cfg(test)` (see the module documentation).
#[derive(Default)]
pub(crate) struct RwLock<T: ?Sized>(parking_lot::RwLock<T>);

impl<T> RwLock<T> {
    /// A new lock holding `value`.
    pub(crate) const fn new(value: T) -> Self {
        Self(parking_lot::RwLock::new(value))
    }
}

impl<T: ?Sized> RwLock<T> {
    /// Takes the shared guard, as `parking_lot::RwLock::read` does.
    #[track_caller]
    pub(crate) fn read(&self) -> ReadGuard<'_, T> {
        let held = Held::acquire();
        ReadGuard(self.0.read(), held)
    }

    /// Takes the exclusive guard, as `parking_lot::RwLock::write` does.
    #[track_caller]
    pub(crate) fn write(&self) -> WriteGuard<'_, T> {
        let held = Held::acquire();
        WriteGuard(self.0.write(), held)
    }
}

impl<T: ?Sized + std::fmt::Debug> std::fmt::Debug for RwLock<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// A shared guard on a collection [`RwLock`].
#[must_use = "the lock is released when the guard is dropped"]
pub(crate) struct ReadGuard<'a, T: ?Sized>(parking_lot::RwLockReadGuard<'a, T>, Held);

impl<T: ?Sized> Deref for ReadGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

/// An exclusive guard on a collection [`RwLock`].
#[must_use = "the lock is released when the guard is dropped"]
pub(crate) struct WriteGuard<'a, T: ?Sized>(parking_lot::RwLockWriteGuard<'a, T>, Held);

impl<T: ?Sized> Deref for WriteGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T: ?Sized> DerefMut for WriteGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

/// A collection's mutex: `parking_lot`'s, checked under `cfg(test)` (see the
/// module documentation).
#[derive(Default)]
pub(crate) struct Mutex<T: ?Sized>(parking_lot::Mutex<T>);

impl<T> Mutex<T> {
    /// A new mutex holding `value`.
    pub(crate) const fn new(value: T) -> Self {
        Self(parking_lot::Mutex::new(value))
    }
}

impl<T: ?Sized> Mutex<T> {
    /// Takes the guard, as `parking_lot::Mutex::lock` does.
    #[track_caller]
    pub(crate) fn lock(&self) -> MutexGuard<'_, T> {
        let held = Held::acquire();
        MutexGuard(self.0.lock(), held)
    }
}

impl<T: ?Sized + std::fmt::Debug> std::fmt::Debug for Mutex<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// The guard of a collection [`Mutex`].
#[must_use = "the lock is released when the guard is dropped"]
pub(crate) struct MutexGuard<'a, T: ?Sized>(parking_lot::MutexGuard<'a, T>, Held);

impl<T: ?Sized> Deref for MutexGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T: ?Sized> DerefMut for MutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}
