//! The collection's locks, and the unit-test check of the rule that keeps
//! them from deadlocking against a rayon pool.
//!
//! The rule (`docs/CONCURRENCY_MODEL.md`, "Collection locks and rayon"):
//!
//! 1. no job on a rayon pool takes a collection lock, and
//! 2. no thread holds a collection lock while it waits on the global pool.
//!
//! Either half alone closes the cycle of \#2343 one lock lower: a thread
//! holding `vector_storage`'s read guard waits for its global-pool jobs, a
//! writer queues behind that guard, and `parking_lot` then parks every new
//! reader, including the global workers whose jobs take the guard. A thread
//! that must hold a guard across parallel work submits that work to the
//! dedicated pool (`collection::guarded_pool`), whose jobs read through the
//! guards their caller holds and take none themselves.
//!
//! [`RwLock`] and [`Mutex`] wrap `parking_lot`'s for every lock a
//! `Collection` owns, so every accessor goes through [`RwLock::read`],
//! [`RwLock::write`] or [`Mutex::lock`]. Under `cfg(test)` each of them
//! panics when called on a rayon worker (half 1), and each guard counts
//! itself, in a thread-local, while it lives; [`assert_no_guard_held`],
//! called where a global-pool submission begins, panics when this thread
//! holds one (half 2). Outside `cfg(test)` the wrappers are the
//! `parking_lot` locks and guards with nothing added: the check costs
//! release builds nothing, and code that is not a unit test — an
//! application calling a collection from its own rayon jobs — is never
//! refused.

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
        #[cfg(test)]
        if let Some(worker) = rayon::current_thread_index().filter(|_| {
            // An application's own jobs may call into a collection.
            !crate::pools::on_application_thread()
        }) {
            panic!(
                "a collection lock was acquired on rayon worker {worker}: no job on a \
                 rayon pool may take a collection lock (docs/CONCURRENCY_MODEL.md, \
                 \"Collection locks and rayon\")"
            );
        }
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
/// Called where work is submitted to the global rayon pool: a thread that
/// waits on that pool must hold no collection lock. Compiles to nothing
/// outside unit tests.
#[inline]
#[track_caller]
pub(crate) fn assert_no_guard_held() {
    #[cfg(test)]
    HELD.with(|held| {
        assert!(
            held.get() == 0,
            "{} collection lock guard(s) held across a submission to the global rayon \
             pool: copy what the jobs need out first, or run them on \
             collection::guarded_pool (docs/CONCURRENCY_MODEL.md, \"Collection locks \
             and rayon\")",
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
