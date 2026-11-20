//! A test's hooks into a ranked walk (`collection/search/ranked.rs`), each
//! run on the thread that set it, with no storage guard held:
//!
//! - [`on_guard_release`]: each time the walk releases the storage guards
//!   between two holds — where a write racing the walk lands;
//! - [`on_prefilter_built`]: once, when a walk has asked the secondary
//!   indexes for its prefilter bitmap and has not yet taken a storage guard
//!   — where a write the bitmap does not describe lands before the walk's
//!   first hold.

use std::cell::RefCell;

thread_local! {
    static ON_RELEASE: RefCell<Option<Box<dyn FnMut()>>> = const { RefCell::new(None) };
    static ON_PREFILTER: RefCell<Option<Box<dyn FnOnce()>>> = const { RefCell::new(None) };
}

/// Runs `hook` each time a ranked walk on this thread releases the storage
/// guards between two holds, until the thread ends.
pub(crate) fn on_guard_release(hook: impl FnMut() + 'static) {
    ON_RELEASE.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
}

/// The walk released the storage guards: the hook [`on_guard_release`] set
/// runs here.
pub(super) fn guards_released() {
    let hook = ON_RELEASE.with(|slot| slot.borrow_mut().take());
    if let Some(mut hook) = hook {
        hook();
        ON_RELEASE.with(|slot| *slot.borrow_mut() = Some(hook));
    }
}

/// Runs `hook` once, the next time a walk on this thread has asked the
/// secondary indexes for its prefilter bitmap.
pub(crate) fn on_prefilter_built(hook: impl FnOnce() + 'static) {
    ON_PREFILTER.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
}

/// The prefilter bitmap is built: the hook [`on_prefilter_built`] set runs
/// here.
pub(super) fn prefilter_built() {
    if let Some(hook) = ON_PREFILTER.with(|slot| slot.borrow_mut().take()) {
        hook();
    }
}
