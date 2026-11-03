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
///
/// Deliberately carries no credential: this struct is echoed to the console
/// and its `Debug` impl is written to be log-safe, so the collected API key
/// travels separately as an `Option<String>` that no log path receives.
/// Adding a secret-typed field here is a cleartext-logging regression —
/// `test_wizard_config_debug_contains_no_api_key_field` guards it.
#[derive(Clone)]
pub struct WizardConfig {
    /// Selected source type.
    pub source_type: SourceType,
    /// Source URL or connection string.
    pub url: String,
    /// Collection/table/index name.
    pub collection: String,
    /// Destination path for VelesDB data.
    pub dest_path: String,
    /// Use SQ8 compression (4x smaller).
    pub use_sq8: bool,
}

impl std::fmt::Debug for WizardConfig {
    /// The `url` field may carry `user[:pass]@` userinfo needed to connect;
    /// `Debug` must be safe to log, so it renders the URL through
    /// [`crate::redact::redact_url`].
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WizardConfig")
            .field("source_type", &self.source_type)
            .field("url", &crate::redact::redact_url(&self.url))
            .field("collection", &self.collection)
            .field("dest_path", &self.dest_path)
            .field("use_sq8", &self.use_sq8)
            .finish()
    }
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

        // Step 2: Get connection details. The API key stays a bare variable:
        // it must never join `WizardConfig`, which is printed to the console.
        let mut api_key = None;
        let config = self
            .prompts
            .get_connection_details(source_type, &mut api_key)?;

        // Step 3: Connect and discover schema
        self.ui.print_connecting(&config.url);
        let source_config = self.build_source_config(&config, api_key.as_deref())?;
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

        let migration_config = self.build_migration_config(&config, &schema, api_key.as_deref())?;
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
    fn build_source_config(
        &self,
        config: &WizardConfig,
        api_key: Option<&str>,
    ) -> Result<SourceConfig> {
        let params = crate::source_config_builder::SourceParams {
            source_type: config.source_type,
            url: &config.url,
            api_key,
            collection: &config.collection,
        };
        crate::source_config_builder::build_source_config(&params)
    }

    /// Builds full migration config.
    fn build_migration_config(
        &self,
        config: &WizardConfig,
        schema: &SourceSchema,
        api_key: Option<&str>,
    ) -> Result<MigrationConfig> {
        migration_builder::build_migration_config(config, schema, api_key)
    }
}

#[cfg(test)]
#[path = "wizard_tests.rs"]
mod tests;
