//! Test hook for the window `brute_force_search_rayon` scans in: after its
//! `inner` read guard — and so its vector snapshot — is released, before the
//! rayon scan resolves each slot to an id. A renumber running there
//! (`reorder_for_locality`, `vacuum`) is the race of #2607; this module is
//! the seam the test parks one in. Same shape as
//! `persistence::save_window`.

use std::sync::Mutex;

type Hook = Box<dyn Fn() + Send + Sync>;

static INSTALLED: Mutex<Option<Hook>> = Mutex::new(None);

/// Clears the installed hook when dropped, including on a panic.
pub(crate) struct Installed;

impl Drop for Installed {
    fn drop(&mut self) {
        *INSTALLED.lock().expect("the scan window is never poisoned") = None;
    }
}

/// Runs `hook` in the scan window of every `brute_force_search_rayon` call
/// until the returned guard is dropped.
pub(crate) fn install(hook: impl Fn() + Send + Sync + 'static) -> Installed {
    *INSTALLED.lock().expect("the scan window is never poisoned") = Some(Box::new(hook));
    Installed
}

/// Runs the installed hook, if any.
pub(crate) fn pause() {
    // The hook is called under the lock: it never installs another, and
    // holding it keeps `Installed::drop` from freeing it mid-call.
    let installed = INSTALLED.lock().expect("the scan window is never poisoned");
    if let Some(hook) = installed.as_ref() {
        hook();
    }
}
