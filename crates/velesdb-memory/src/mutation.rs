//! Pre-mutation classification for the native online-migration coordinator.

use std::sync::Arc;

use parking_lot::RwLock;

use crate::MemoryError;

pub(crate) mod catchup;
pub(crate) mod controller;
pub(crate) mod journal;

#[cfg(test)]
mod controller_tests;

/// Idempotent source state that a migration must re-read after a mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum DirtyKey {
    Fact(u64),
    OutgoingEdges(u64),
}

/// Sink invoked before a native source mutation is allowed to run.
pub(crate) trait MutationObserver: Send + Sync {
    fn before_mutation(&self, key: DirtyKey) -> Result<(), MemoryError>;
}

/// Replaceable observer slot. `None` is the steady-state no-capture path.
#[derive(Default)]
pub(crate) struct MutationCapture {
    observer: RwLock<Option<Arc<dyn MutationObserver>>>,
}

impl MutationCapture {
    pub(crate) fn observe(&self, key: DirtyKey) -> Result<(), MemoryError> {
        let observer = self.observer.read();
        match observer.as_ref() {
            Some(observer) => observer.before_mutation(key),
            None => Ok(()),
        }
    }

    pub(crate) fn replace(
        &self,
        observer: Option<Arc<dyn MutationObserver>>,
    ) -> Result<(), MemoryError> {
        let mut installed = self.observer.write();
        if installed.is_some() && observer.is_some() {
            return Err(MemoryError::MigrationCapture(
                "a mutation observer is already active".to_owned(),
            ));
        }
        *installed = observer;
        Ok(())
    }

    pub(crate) fn is_active(&self) -> bool {
        self.observer.read().is_some()
    }
}

#[cfg(test)]
#[path = "mutation_tests.rs"]
mod tests;

#[cfg(all(test, feature = "persistence"))]
mod catchup_tests;

#[cfg(test)]
mod journal_tests;
