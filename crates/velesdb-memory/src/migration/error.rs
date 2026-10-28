//! The failures of the migration pipeline, as this crate's own type.
//!
//! They used to be a message wrapped in the storage engine's `Query` error, so
//! `MemoryError::category` billed every one of them as an internal fault and
//! every adapter told its client that the server had broken when an operator
//! had only run two steps out of order. The variants below are the causes the
//! adapters have to tell apart, named after who can fix them.
//!
//! The payload is prose on purpose, for the reason [`crate::MemoryError`]
//! gives: nothing outside this crate reads it programmatically, and these
//! messages are narratives for the person who has to act on them.

/// Why a migration step could not proceed.
///
/// `non_exhaustive`: a cause that deserves its own class is added without a
/// breaking release, and callers classify through
/// [`crate::MemoryError::category`] rather than by variant.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MigrationError {
    /// Something the caller named does not exist: a collection, a store
    /// directory, a migration journal.
    #[error("{0}")]
    NotFound(String),

    /// The step was asked for in a state or on an input it cannot accept: run
    /// out of order (validating before the rebuild, switching before the
    /// validation), pointed at a symlink or a special file, or aimed at a
    /// collection that is not an agent-memory one. Whoever called it fixes it.
    #[error("{0}")]
    Precondition(String),

    /// The machine failed the step: a filesystem call, no room left for the
    /// staging copy, a scratch directory that could not be allocated.
    #[error("{0}")]
    Environment(String),

    /// The online-migration observer could not durably classify a mutation, or
    /// refused one it cannot account for. The source write has not run when
    /// this error is returned.
    #[error("{0}")]
    Capture(String),

    /// The data contradicts itself: an edge whose id its triple does not
    /// derive, two indexes that disagree, a source that changed under a
    /// diagnosis, a fact that exists on one side only. Not the caller's fault
    /// and not something to retry blindly.
    #[error("integrity violation: {0}")]
    Integrity(String),
}

impl MigrationError {
    /// See [`Self::NotFound`].
    pub(crate) fn not_found(message: impl Into<String>) -> Self {
        Self::NotFound(message.into())
    }

    /// See [`Self::Precondition`].
    pub(crate) fn precondition(message: impl Into<String>) -> Self {
        Self::Precondition(message.into())
    }

    /// See [`Self::Environment`].
    pub(crate) fn environment(message: impl Into<String>) -> Self {
        Self::Environment(message.into())
    }

    /// See [`Self::Capture`].
    pub(crate) fn capture(message: impl Into<String>) -> Self {
        Self::Capture(message.into())
    }

    /// See [`Self::Integrity`].
    pub(crate) fn integrity(message: impl Into<String>) -> Self {
        Self::Integrity(message.into())
    }

    /// The class of this failure, as [`crate::MemoryError::category`] reports it.
    pub(crate) fn category(&self) -> crate::ErrorCategory {
        match self {
            Self::NotFound(_) => crate::ErrorCategory::NotFound,
            Self::Precondition(_) => crate::ErrorCategory::InvalidInput,
            Self::Environment(_) | Self::Capture(_) | Self::Integrity(_) => {
                crate::ErrorCategory::Internal
            }
        }
    }
}
