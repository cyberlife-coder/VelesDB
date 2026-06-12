//! State management for the `VelesDB` Tauri plugin.
//!
//! Manages the database instance and provides thread-safe access
//! to collections across Tauri commands.

use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::RwLock;
use velesdb_core::agent::AgentMemory;
use velesdb_core::Database;

use crate::error::{Error, Result};

/// Sub-directory (under the database path) used to persist versioned memory snapshots.
const MEMORY_SNAPSHOT_DIR: &str = "_memory_snapshots";

/// Maximum number of versioned memory snapshots retained on disk.
const MEMORY_SNAPSHOT_MAX: usize = 16;

/// Plugin state holding the database instance.
///
/// This struct is managed by Tauri and provides thread-safe access
/// to the `VelesDB` database from all commands.
pub struct VelesDbState {
    /// The database instance wrapped in Arc<RwLock> for thread-safe access.
    db: Arc<RwLock<Option<Arc<Database>>>>,
    /// Persistent unified `AgentMemory` handle.
    ///
    /// Built lazily on first memory command and shared across all subsequent
    /// commands so the TTL registry, temporal index, eviction config, and
    /// snapshot manager survive between invocations. Re-opening a fresh memory
    /// per command (the previous behaviour) silently dropped the in-memory TTL
    /// registry, so TTL / auto-expire / snapshot versioning never worked.
    memory: Arc<RwLock<Option<Arc<AgentMemory>>>>,
    /// Path to the database directory.
    path: PathBuf,
}

impl VelesDbState {
    /// Creates a new plugin state with the specified database path.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the database directory
    ///
    /// # Returns
    ///
    /// A new `VelesDbState` instance (database not yet opened).
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self {
            db: Arc::new(RwLock::new(None)),
            memory: Arc::new(RwLock::new(None)),
            path,
        }
    }

    /// Opens the database, creating it if it doesn't exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the database cannot be opened.
    pub fn open(&self) -> Result<()> {
        let mut db_guard = self.db.write();
        if db_guard.is_none() {
            let db = Arc::new(Database::open(&self.path)?);
            *db_guard = Some(db);
            tracing::info!("VelesDB opened at {:?}", self.path);
        }
        Ok(())
    }

    /// Returns a reference to the database, opening it if necessary.
    ///
    /// # Errors
    ///
    /// Returns an error if the database cannot be accessed.
    pub fn get_db(&self) -> Result<Arc<RwLock<Option<Arc<Database>>>>> {
        // Ensure database is open
        {
            let db_guard = self.db.read();
            if db_guard.is_none() {
                drop(db_guard);
                self.open()?;
            }
        }
        Ok(Arc::clone(&self.db))
    }

    /// Executes a function with read access to the database.
    ///
    /// # Errors
    ///
    /// Returns an error if the database is not available.
    pub fn with_db<F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(Arc<Database>) -> Result<T>,
    {
        self.open()?;
        let db_guard = self.db.read();
        let db = db_guard
            .as_ref()
            .ok_or_else(|| Error::InvalidConfig("Database not initialized".to_string()))?;
        f(Arc::clone(db))
    }

    /// Executes a function with the persistent unified `AgentMemory` handle.
    ///
    /// The handle is built once (lazily) using the default embedding dimension
    /// and a snapshot directory under the database path, then reused for every
    /// later call. This keeps the TTL registry, temporal index, and snapshot
    /// manager alive across commands.
    ///
    /// # Errors
    ///
    /// Returns an error if the database or memory subsystems cannot be opened.
    pub fn with_memory<F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&AgentMemory) -> Result<T>,
    {
        let memory = self.memory_handle()?;
        f(&memory)
    }

    /// Returns the shared `AgentMemory`, building it on first use.
    ///
    /// Lock ordering: the `db` lock (via `with_db`) is fully released before the
    /// `memory` write lock is taken, so the two are never nested.
    fn memory_handle(&self) -> Result<Arc<AgentMemory>> {
        if let Some(existing) = self.memory.read().clone() {
            return Ok(existing);
        }
        let db = self.with_db(Ok)?;
        let snapshot_dir = self.path.join(MEMORY_SNAPSHOT_DIR);
        let mut guard = self.memory.write();
        if let Some(existing) = guard.clone() {
            return Ok(existing);
        }
        let snapshot_dir = snapshot_dir.to_string_lossy().into_owned();
        let memory =
            Arc::new(AgentMemory::new(db)?.with_snapshots(&snapshot_dir, MEMORY_SNAPSHOT_MAX));
        *guard = Some(Arc::clone(&memory));
        Ok(memory)
    }

    /// Returns the database path.
    #[must_use]
    pub fn path(&self) -> &PathBuf {
        &self.path
    }
}

impl Default for VelesDbState {
    fn default() -> Self {
        Self::new(PathBuf::from("./velesdb_data"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_state_new() {
        // Arrange
        let path = PathBuf::from("/tmp/test_db");

        // Act
        let state = VelesDbState::new(path.clone());

        // Assert
        assert_eq!(state.path(), &path);
    }

    #[test]
    fn test_state_default() {
        // Act
        let state = VelesDbState::default();

        // Assert
        assert_eq!(state.path(), &PathBuf::from("./velesdb_data"));
    }

    #[test]
    fn test_state_open_and_access() {
        // Arrange
        let dir = tempdir().expect("Failed to create temp dir");
        let state = VelesDbState::new(dir.path().to_path_buf());

        // Act
        let result = state.open();

        // Assert
        assert!(result.is_ok());
    }

    #[test]
    fn test_state_with_db() {
        // Arrange
        let dir = tempdir().expect("Failed to create temp dir");
        let state = VelesDbState::new(dir.path().to_path_buf());

        // Act
        let result = state.with_db(|db| {
            // Just verify we can access the database
            let collections = db.list_collections();
            Ok(collections.len())
        });
        // Note: db is Arc<Database> — list_collections() is reachable via Deref

        // Assert
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 0); // No collections initially
    }

    #[test]
    fn test_state_multiple_opens_idempotent() {
        // Arrange
        let dir = tempdir().expect("Failed to create temp dir");
        let state = VelesDbState::new(dir.path().to_path_buf());

        // Act - open multiple times
        let result1 = state.open();
        let result2 = state.open();
        let result3 = state.open();

        // Assert - all should succeed
        assert!(result1.is_ok());
        assert!(result2.is_ok());
        assert!(result3.is_ok());
    }

    /// The persistent handle must be one shared `AgentMemory`, not a fresh
    /// instance per call. Different instances would each own a separate TTL
    /// registry, which is exactly the bug this fix targets.
    #[test]
    fn test_memory_handle_is_shared_across_calls() {
        // Arrange
        let dir = tempdir().expect("Failed to create temp dir");
        let state = VelesDbState::new(dir.path().to_path_buf());

        // Act
        let first = state.memory_handle().expect("first handle");
        let second = state.memory_handle().expect("second handle");

        // Assert - same allocation, so the TTL registry is shared.
        assert!(Arc::ptr_eq(&first, &second));
    }

    /// A TTL set during one command must persist into a later command. With the
    /// previous per-command `new_from_db` memory, the TTL registry was dropped
    /// between calls and `auto_expire` would expire nothing.
    #[test]
    fn test_ttl_persists_across_with_memory_calls() {
        // Arrange
        let dir = tempdir().expect("Failed to create temp dir");
        let state = VelesDbState::new(dir.path().to_path_buf());
        let dim = velesdb_core::agent::DEFAULT_DIMENSION;
        let embedding = vec![0.1_f32; dim];

        // Act - command #1: store a fact with a 1-second TTL.
        state
            .with_memory(|mem| {
                mem.semantic()
                    .store_with_ttl(42, "ephemeral", &embedding, 1)?;
                Ok(())
            })
            .expect("store_with_ttl");

        // Wait past the TTL boundary (whole-second granularity).
        std::thread::sleep(std::time::Duration::from_millis(1_100));

        // Command #2: a separate call expires it via the shared registry.
        let result = state
            .with_memory(|mem| Ok(mem.auto_expire()?))
            .expect("auto_expire");

        // Assert - the entry tracked in call #1 was expired in call #2.
        assert_eq!(result.semantic_expired, 1);

        // And it is actually gone from the collection.
        let hits = state
            .with_memory(|mem| Ok(mem.semantic().query(&embedding, 10)?))
            .expect("query");
        assert!(hits.iter().all(|(id, _, _)| *id != 42));
    }

    /// A versioned snapshot taken in one call must be loadable in a later call,
    /// proving the snapshot manager is held in the persistent handle.
    #[test]
    fn test_snapshot_versioning_persists_across_calls() {
        // Arrange
        let dir = tempdir().expect("Failed to create temp dir");
        let state = VelesDbState::new(dir.path().to_path_buf());
        let dim = velesdb_core::agent::DEFAULT_DIMENSION;
        let embedding = vec![0.2_f32; dim];

        // Act - store, then snapshot in one call.
        let version = state
            .with_memory(|mem| {
                mem.semantic().store(7, "durable", &embedding)?;
                Ok(mem.snapshot()?)
            })
            .expect("snapshot");

        // A later call lists and reloads that version.
        let versions = state
            .with_memory(|mem| Ok(mem.list_snapshot_versions()?))
            .expect("list versions");
        assert!(versions.contains(&version));

        state
            .with_memory(|mem| {
                mem.load_snapshot_version(version)?;
                Ok(())
            })
            .expect("load version");

        // Assert - the stored fact survives the snapshot round-trip.
        let hits = state
            .with_memory(|mem| Ok(mem.semantic().query(&embedding, 10)?))
            .expect("query");
        assert!(hits.iter().any(|(id, _, _)| *id == 7));
    }
}
