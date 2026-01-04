//! State management for the `VelesDB` Tauri plugin.
//!
//! Manages the database instance and provides thread-safe access
//! to collections across Tauri commands.

use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::RwLock;
use velesdb_core::Database;

use crate::error::{Error, Result};

/// Plugin state holding the database instance.
///
/// This struct is managed by Tauri and provides thread-safe access
/// to the `VelesDB` database from all commands.
pub struct VelesDbState {
    /// The database instance wrapped in Arc<RwLock> for thread-safe access.
    db: Arc<RwLock<Option<Database>>>,
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
            let db = Database::open(&self.path)?;
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
    pub fn get_db(&self) -> Result<Arc<RwLock<Option<Database>>>> {
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
        F: FnOnce(&Database) -> Result<T>,
    {
        self.open()?;
        let db_guard = self.db.read();
        let db = db_guard
            .as_ref()
            .ok_or_else(|| Error::InvalidConfig("Database not initialized".to_string()))?;
        f(db)
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
}
