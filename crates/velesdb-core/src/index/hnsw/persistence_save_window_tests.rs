use std::path::{Path, PathBuf};
use std::sync::Mutex;

type Hook = Box<dyn Fn() + Send + Sync>;

static INSTALLED: Mutex<Option<(PathBuf, Hook)>> = Mutex::new(None);

/// Clears the installed window when dropped, including on a panic.
pub(crate) struct Installed;

impl Drop for Installed {
    fn drop(&mut self) {
        *INSTALLED.lock().expect("the save window is never poisoned") = None;
    }
}

/// Runs `hook` in the window of every save into `path` until the returned
/// guard is dropped.
pub(crate) fn install(path: &Path, hook: impl Fn() + Send + Sync + 'static) -> Installed {
    *INSTALLED.lock().expect("the save window is never poisoned") =
        Some((path.to_path_buf(), Box::new(hook)));
    Installed
}

/// Runs the installed hook if it was installed for `path`.
pub(crate) fn pause(path: &Path) {
    // The hook is called under the lock: it never installs another, and
    // holding it keeps `Installed::drop` from freeing it mid-call.
    let installed = INSTALLED.lock().expect("the save window is never poisoned");
    if let Some((wanted, hook)) = installed.as_ref() {
        if wanted == path {
            hook();
        }
    }
}
