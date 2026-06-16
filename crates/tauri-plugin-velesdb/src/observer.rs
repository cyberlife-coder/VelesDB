//! Bridges core [`DatabaseObserver`] lifecycle hooks to Tauri events.
//!
//! Injecting a [`TauriObserver`] when the database is opened means collection
//! create/delete events reach the frontend no matter where they originate in
//! the core — direct commands, `VelesQL` DDL via the `query` command, or any
//! future entry path — instead of only the commands that emit manually.

use tauri::{AppHandle, Runtime};
use velesdb_core::collection::CollectionType;
use velesdb_core::DatabaseObserver;

use crate::events::{emit_collection_created, emit_collection_deleted};

/// A [`DatabaseObserver`] that forwards collection lifecycle hooks to the
/// plugin's Tauri event stream.
///
/// Generic over the Tauri [`Runtime`]; the generic is erased when the value is
/// stored as `Arc<dyn DatabaseObserver>` and handed to
/// [`Database::open_with_observer`](velesdb_core::Database::open_with_observer).
pub struct TauriObserver<R: Runtime> {
    app: AppHandle<R>,
}

impl<R: Runtime> TauriObserver<R> {
    /// Creates an observer that emits events through the given app handle.
    #[must_use]
    pub fn new(app: AppHandle<R>) -> Self {
        Self { app }
    }
}

impl<R: Runtime> DatabaseObserver for TauriObserver<R> {
    fn on_collection_created(&self, name: &str, _kind: &CollectionType) {
        emit_collection_created(&self.app, name);
    }

    fn on_collection_deleted(&self, name: &str) {
        emit_collection_deleted(&self.app, name);
    }
}
