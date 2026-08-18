//! Interactive migration wizard for zero-config migrations.
//!
//! The wizard guides users through the migration process step by step,
//! auto-detecting schema and configuration options.

mod discovery;
mod migration_builder;
mod prompts;
mod source_type;
mod ui;

pub use discovery::SourceDiscovery;
pub use prompts::WizardPrompts;
pub use source_type::SourceType;
pub use ui::WizardUI;

use crate::config::SourceConfig;
use crate::connectors::{create_connector, SourceSchema};
use crate::error::Result;
use crate::pipeline::Pipeline;
use crate::MigrationConfig;

/// Configuration collected during wizard interaction.
#[derive(Debug, Clone)]
pub struct WizardConfig {
    /// Selected source type.
    pub source_type: SourceType,
    /// Source URL or connection string.
    pub url: String,
    /// API key (if required by source).
    pub api_key: Option<String>,
    /// Collection/table/index name.
    pub collection: String,
    /// Destination path for VelesDB data.
    pub dest_path: String,
    /// Use SQ8 compression (4x smaller).
    pub use_sq8: bool,
}

/// Interactive migration wizard.
pub struct Wizard {
    ui: WizardUI,
    prompts: WizardPrompts,
}

impl Default for Wizard {
    fn default() -> Self {
        Self::new()
    }
}

impl Wizard {
    /// Creates a new wizard instance.
    pub fn new() -> Self {
        Self {
            ui: WizardUI::new(),
            prompts: WizardPrompts::new(),
        }
    }

    /// Runs the interactive wizard.
    pub async fn run(&self) -> Result<()> {
        self.ui.print_header();

        // Step 1: Select source type
        let source_type = self.prompts.select_source()?;

        // Step 2: Get connection details
        let config = self.prompts.get_connection_details(source_type)?;

        // Step 3: Connect and discover schema
        self.ui.print_connecting(&config.url);
        let source_config = self.build_source_config(&config)?;
        let mut connector = create_connector(&source_config)?;

        connector.connect().await?;
        let schema = connector.get_schema().await?;
        connector.close().await?;

        // Step 4: Show discovered schema
        self.ui.print_schema_discovered(&schema);

        // Step 5: Confirm migration
        if !self.prompts.confirm_migration(&schema, &config)? {
            self.ui.print_cancelled();
            return Ok(());
        }

        // Step 6: Run migration
        self.ui.print_starting_migration();

        let migration_config = self.build_migration_config(&config, &schema)?;
        let mut pipeline = Pipeline::new(migration_config)?;
        let stats = pipeline.run().await?;

        // Step 7: Show results
        self.ui.print_success(&stats, &config);

        Ok(())
    }

    /// Builds source config from wizard config.
    ///
    /// Delegates to [`crate::source_config_builder::build_source_config`]
    /// to avoid duplicating the per-source match block.
    fn build_source_config(&self, config: &WizardConfig) -> Result<SourceConfig> {
        let params = crate::source_config_builder::SourceParams {
            source_type: config.source_type,
            url: &config.url,
            api_key: config.api_key.as_deref(),
            collection: &config.collection,
        };
        crate::source_config_builder::build_source_config(&params)
    }

    /// Builds full migration config.
    fn build_migration_config(
        &self,
        config: &WizardConfig,
        schema: &SourceSchema,
    ) -> Result<MigrationConfig> {
        migration_builder::build_migration_config(config, schema)
    }
}

#[cfg(test)]
#[path = "wizard_tests.rs"]
mod tests;
