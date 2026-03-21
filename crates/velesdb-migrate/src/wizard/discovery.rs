//! Auto-discovery of collections and tables from source databases.

use crate::config::SourceConfig;
use crate::connectors::{create_connector, SourceSchema};
use crate::error::Result;

use super::SourceType;

/// Discovered collection information.
#[derive(Debug, Clone)]
pub struct DiscoveredCollection {
    /// Collection/table name.
    pub name: String,
    /// Vector count (if available).
    pub count: Option<u64>,
    /// Vector dimension (if available).
    pub dimension: usize,
}

/// Source discovery utilities.
pub struct SourceDiscovery;

impl SourceDiscovery {
    /// Lists available collections from a source.
    ///
    /// Note: Currently returns only the specified collection's schema.
    /// Future versions will support listing all collections.
    pub async fn list_collections(
        source_type: SourceType,
        url: &str,
        api_key: Option<&str>,
        collection: &str,
    ) -> Result<Vec<DiscoveredCollection>> {
        let source_config = Self::build_config(source_type, url, api_key, collection)?;
        let mut connector = create_connector(&source_config)?;

        connector.connect().await?;
        let schema = connector.get_schema().await?;
        connector.close().await?;

        Ok(vec![DiscoveredCollection {
            name: schema.collection.clone(),
            count: schema.total_count,
            dimension: schema.dimension,
        }])
    }

    /// Gets schema for a specific collection.
    pub async fn get_schema(
        source_type: SourceType,
        url: &str,
        api_key: Option<&str>,
        collection: &str,
    ) -> Result<SourceSchema> {
        let source_config = Self::build_config(source_type, url, api_key, collection)?;
        let mut connector = create_connector(&source_config)?;

        connector.connect().await?;
        let schema = connector.get_schema().await?;
        connector.close().await?;

        Ok(schema)
    }

    /// Builds source config for discovery.
    ///
    /// Delegates to [`crate::source_config_builder::build_source_config`]
    /// to avoid duplicating the per-source match block.
    fn build_config(
        source_type: SourceType,
        url: &str,
        api_key: Option<&str>,
        collection: &str,
    ) -> Result<SourceConfig> {
        let params = crate::source_config_builder::SourceParams {
            source_type,
            url,
            api_key,
            collection,
        };
        crate::source_config_builder::build_source_config(&params)
    }
}
