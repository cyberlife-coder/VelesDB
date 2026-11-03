//! Migration config builder for the wizard.

use crate::config::{DestinationConfig, DistanceMetric, MigrationOptions, StorageMode};
use crate::connectors::SourceSchema;
use crate::error::Result;
use crate::MigrationConfig;
use std::path::PathBuf;

use super::WizardConfig;

/// Builds a full [`MigrationConfig`] from wizard-collected config and discovered schema.
///
/// `api_key` is kept out of [`WizardConfig`] on purpose (that struct is
/// rendered to the console); it is threaded through as a separate argument.
pub(crate) fn build_migration_config(
    config: &WizardConfig,
    schema: &SourceSchema,
    api_key: Option<&str>,
) -> Result<MigrationConfig> {
    let params = crate::source_config_builder::SourceParams {
        source_type: config.source_type,
        url: &config.url,
        api_key,
        collection: &config.collection,
    };
    let source = crate::source_config_builder::build_source_config(&params)?;

    let storage_mode = if config.use_sq8 {
        StorageMode::SQ8
    } else {
        StorageMode::Full
    };

    let destination = DestinationConfig {
        path: PathBuf::from(&config.dest_path),
        collection: config.collection.clone(),
        dimension: schema.dimension,
        metric: DistanceMetric::Cosine,
        storage_mode,
        graph_collection: None,
    };

    let options = MigrationOptions {
        batch_size: 1000,
        workers: 4,
        dry_run: false,
        continue_on_error: false,
        checkpoint_enabled: true,
        checkpoint_path: None,
        field_mappings: std::collections::HashMap::new(),
        allow_metric_mismatch: false,
    };

    Ok(MigrationConfig {
        source,
        destination,
        options,
        relations: vec![],
    })
}
