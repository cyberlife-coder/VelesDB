//! Metadata-only collection creation and retrieval operations.

use crate::collection::MetadataCollection;
use crate::{CollectionType, Result};

use super::Database;

#[allow(deprecated)]
impl Database {
    /// Creates a new metadata-only collection.
    ///
    /// # Errors
    ///
    /// Returns an error if a collection with the same name already exists.
    pub fn create_metadata_collection(&self, name: &str) -> Result<()> {
        self.ensure_collection_name_available(name)?;
        let path = self.data_dir.join(name);
        let coll = MetadataCollection::create(path, name)?;
        self.collections
            .write()
            .insert(name.to_string(), coll.inner.clone());
        self.metadata_colls.write().insert(name.to_string(), coll);

        if let Some(ref obs) = self.observer {
            obs.on_collection_created(name, &CollectionType::MetadataOnly);
        }

        self.schema_version
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        Ok(())
    }

    /// Returns a `MetadataCollection` by name.
    ///
    /// Checks the typed registry first.  Falls back to opening from disk for
    /// collections created before the typed API existed or after a restart.
    /// The instance is cached to avoid repeated disk reads.
    ///
    /// Returns `None` if the collection does not exist on disk.
    #[must_use]
    pub fn get_metadata_collection(&self, name: &str) -> Option<MetadataCollection> {
        if let Some(c) = self.metadata_colls.read().get(name).cloned() {
            return Some(c);
        }
        self.open_metadata_collection_from_disk(name)
    }

    /// Disk fallback for `get_metadata_collection`.
    fn open_metadata_collection_from_disk(&self, name: &str) -> Option<MetadataCollection> {
        let cfg = self.read_collection_config(name)?;
        if !cfg.metadata_only {
            return None;
        }
        let coll = MetadataCollection::open(self.data_dir.join(name)).ok()?;
        self.metadata_colls
            .write()
            .insert(name.to_string(), coll.clone());
        Some(coll)
    }
}
