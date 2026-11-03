//! Interactive prompts for the migration wizard.

use console::Style;
use dialoguer::{theme::ColorfulTheme, Confirm, Input, Password, Select};

use super::{SourceType, WizardConfig};
use crate::connectors::SourceSchema;
use crate::error::{Error, Result};

fn input_err(e: impl std::fmt::Display) -> Error {
    Error::Config(format!("Input cancelled: {e}"))
}

fn selection_err(e: impl std::fmt::Display) -> Error {
    Error::Config(format!("Selection cancelled: {e}"))
}

/// Interactive prompts handler.
pub struct WizardPrompts {
    theme: ColorfulTheme,
}

impl Default for WizardPrompts {
    fn default() -> Self {
        Self::new()
    }
}

impl WizardPrompts {
    /// Creates a new prompts handler.
    pub fn new() -> Self {
        Self {
            theme: ColorfulTheme::default(),
        }
    }

    /// Prompts user to select a source type.
    pub fn select_source(&self) -> Result<SourceType> {
        let sources = SourceType::all();
        let items: Vec<&str> = sources.iter().map(|s| s.display_name()).collect();

        let selection = Select::with_theme(&self.theme)
            .with_prompt("Where are your vectors stored?")
            .items(&items)
            .default(0)
            .interact()
            .map_err(selection_err)?;

        Ok(sources[selection])
    }

    /// Gets connection details based on source type.
    ///
    /// The API key is written to `api_key` rather than into the returned
    /// [`WizardConfig`]: the config struct is rendered to the console, so it
    /// must never hold a credential.
    pub fn get_connection_details(
        &self,
        source_type: SourceType,
        api_key: &mut Option<String>,
    ) -> Result<WizardConfig> {
        let url = self.prompt_url(source_type)?;
        *api_key = self.prompt_api_key(source_type)?;
        let collection = self.prompt_collection(source_type)?;
        let dest_path = self.prompt_destination()?;
        let use_sq8 = self.prompt_compression()?;

        Ok(WizardConfig {
            source_type,
            url,
            collection,
            dest_path,
            use_sq8,
        })
    }

    /// Prompts for URL.
    fn prompt_url(&self, source_type: SourceType) -> Result<String> {
        let default = match source_type {
            SourceType::Supabase => "https://your-project.supabase.co",
            SourceType::Qdrant => "http://localhost:6333",
            SourceType::Pinecone => "https://your-index.svc.pinecone.io",
            SourceType::Weaviate => "http://localhost:8080",
            SourceType::Milvus => "http://localhost:19530",
            SourceType::ChromaDB => "http://localhost:8000",
            SourceType::JsonFile => "./vectors.json",
            SourceType::CsvFile => "./vectors.csv",
            SourceType::Elasticsearch => "http://localhost:9200",
            SourceType::Redis => "redis://localhost:6379",
        };

        let prompt = match source_type {
            SourceType::Supabase => "Supabase Project URL",
            SourceType::JsonFile => "JSON File Path",
            SourceType::CsvFile => "CSV File Path",
            SourceType::Elasticsearch => "Elasticsearch URL",
            SourceType::Redis => "Redis URL",
            _ => "Source URL",
        };

        Input::with_theme(&self.theme)
            .with_prompt(prompt)
            .default(default.to_string())
            .interact_text()
            .map_err(input_err)
    }

    /// Prompts for API key.
    fn prompt_api_key(&self, source_type: SourceType) -> Result<Option<String>> {
        if source_type.requires_api_key() {
            let key = Password::with_theme(&self.theme)
                .with_prompt("API Key")
                .interact()
                .map_err(input_err)?;

            if key.is_empty() {
                return Err(Error::Config("API key is required".to_string()));
            }

            Ok(Some(key))
        } else if source_type.optional_api_key() {
            let has_key = Confirm::with_theme(&self.theme)
                .with_prompt("Do you have an API key? (optional)")
                .default(false)
                .interact()
                .map_err(input_err)?;

            if has_key {
                let key = Password::with_theme(&self.theme)
                    .with_prompt("API Key")
                    .interact()
                    .map_err(input_err)?;

                Ok(Some(key))
            } else {
                Ok(None)
            }
        } else {
            Ok(None)
        }
    }

    /// Prompts for collection/table name.
    fn prompt_collection(&self, source_type: SourceType) -> Result<String> {
        let prompt = match source_type {
            SourceType::Supabase => "Table name",
            SourceType::Weaviate => "Class name",
            SourceType::Pinecone => "Index name",
            _ => "Collection name",
        };

        Input::with_theme(&self.theme)
            .with_prompt(prompt)
            .interact_text()
            .map_err(input_err)
    }

    /// Prompts for destination path.
    fn prompt_destination(&self) -> Result<String> {
        Input::with_theme(&self.theme)
            .with_prompt("VelesDB data path")
            .default("./velesdb_data".to_string())
            .interact_text()
            .map_err(input_err)
    }

    /// Prompts for compression option.
    fn prompt_compression(&self) -> Result<bool> {
        let items = vec![
            "Full precision (best quality)",
            "SQ8 compression (4x smaller, ~99% recall)",
        ];

        let selection = Select::with_theme(&self.theme)
            .with_prompt("Storage mode")
            .items(&items)
            .default(0)
            .interact()
            .map_err(selection_err)?;

        Ok(selection == 1)
    }

    /// Confirms migration before starting.
    pub fn confirm_migration(&self, schema: &SourceSchema, config: &WizardConfig) -> Result<bool> {
        println!("{}", format_preview(schema, config));

        Confirm::with_theme(&self.theme)
            .with_prompt("Start migration?")
            .default(true)
            .interact()
            .map_err(input_err)
    }
}

/// Builds the migration preview box. `WizardConfig` carries no credentials,
/// so the whole struct is safe to render here.
pub(crate) fn format_preview(schema: &SourceSchema, config: &WizardConfig) -> String {
    use std::fmt::Write as _;
    let bold = Style::new().bold();
    let mut out = format!(
        "\n{}\n┌─────────────────────────────────────────────────────────────┐\n│ Source:      {} @ {}\n│ Vectors:     {}\n│ Dimension:   {}\n",
        bold.apply_to("📊 Migration Preview:"),
        schema.collection,
        config.source_type.short_name(),
        schema
            .total_count
            .map_or("unknown".to_string(), |c| format!("{c}")),
        schema.dimension
    );

    if !schema.fields.is_empty() {
        let field_names: Vec<&str> = schema
            .fields
            .iter()
            .take(5)
            .map(|f| f.name.as_str())
            .collect();
        let fields_str = if schema.fields.len() > 5 {
            format!(
                "{}, ... (+{})",
                field_names.join(", "),
                schema.fields.len() - 5
            )
        } else {
            field_names.join(", ")
        };
        let _ = writeln!(out, "│ Fields:      {fields_str}");
    }

    let _ = write!(
        out,
        "│ Destination: {}/{}\n│ Compression: {}\n└─────────────────────────────────────────────────────────────┘\n",
        config.dest_path,
        config.collection,
        if config.use_sq8 { "SQ8 (4x)" } else { "Full" }
    );
    out
}
