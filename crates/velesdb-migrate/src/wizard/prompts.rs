//! Interactive prompts for the migration wizard.

use console::Style;
use dialoguer::{theme::ColorfulTheme, Confirm, Input, Password, Select};

use super::{SourceType, WizardConfig};
use crate::connectors::SourceSchema;
use crate::error::{Error, Result};

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
            .map_err(|e| Error::Config(format!("Selection cancelled: {e}")))?;

        Ok(sources[selection])
    }

    /// Gets connection details based on source type.
    pub fn get_connection_details(&self, source_type: SourceType) -> Result<WizardConfig> {
        let url = self.prompt_url(source_type)?;
        let api_key = self.prompt_api_key(source_type)?;
        let collection = self.prompt_collection(source_type)?;
        let dest_path = self.prompt_destination()?;
        let use_sq8 = self.prompt_compression()?;

        Ok(WizardConfig {
            source_type,
            url,
            api_key,
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
            SourceType::PgVector => "postgres://user:pass@localhost:5432/db",
            SourceType::JsonFile => "./vectors.json",
            SourceType::CsvFile => "./vectors.csv",
            SourceType::MongoDB => "https://data.mongodb-api.com/app/<app-id>/endpoint/data/v1",
            SourceType::Elasticsearch => "http://localhost:9200",
            SourceType::Redis => "redis://localhost:6379",
        };

        let prompt = match source_type {
            SourceType::Supabase => "Supabase Project URL",
            SourceType::PgVector => "PostgreSQL Connection String",
            SourceType::JsonFile => "JSON File Path",
            SourceType::CsvFile => "CSV File Path",
            SourceType::MongoDB => "MongoDB Data API URL",
            SourceType::Elasticsearch => "Elasticsearch URL",
            SourceType::Redis => "Redis URL",
            _ => "Source URL",
        };

        Input::with_theme(&self.theme)
            .with_prompt(prompt)
            .default(default.to_string())
            .interact_text()
            .map_err(|e| Error::Config(format!("Input cancelled: {e}")))
    }

    /// Prompts for API key.
    fn prompt_api_key(&self, source_type: SourceType) -> Result<Option<String>> {
        if source_type.requires_api_key() {
            let key = Password::with_theme(&self.theme)
                .with_prompt("API Key")
                .interact()
                .map_err(|e| Error::Config(format!("Input cancelled: {e}")))?;

            if key.is_empty() {
                return Err(Error::Config("API key is required".to_string()));
            }

            Ok(Some(key))
        } else if source_type.optional_api_key() {
            let has_key = Confirm::with_theme(&self.theme)
                .with_prompt("Do you have an API key? (optional)")
                .default(false)
                .interact()
                .map_err(|e| Error::Config(format!("Input cancelled: {e}")))?;

            if has_key {
                let key = Password::with_theme(&self.theme)
                    .with_prompt("API Key")
                    .interact()
                    .map_err(|e| Error::Config(format!("Input cancelled: {e}")))?;

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
            SourceType::Supabase | SourceType::PgVector => "Table name",
            SourceType::Weaviate => "Class name",
            SourceType::Pinecone => "Index name",
            _ => "Collection name",
        };

        Input::with_theme(&self.theme)
            .with_prompt(prompt)
            .interact_text()
            .map_err(|e| Error::Config(format!("Input cancelled: {e}")))
    }

    /// Prompts for destination path.
    fn prompt_destination(&self) -> Result<String> {
        Input::with_theme(&self.theme)
            .with_prompt("VelesDB data path")
            .default("./velesdb_data".to_string())
            .interact_text()
            .map_err(|e| Error::Config(format!("Input cancelled: {e}")))
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
            .map_err(|e| Error::Config(format!("Selection cancelled: {e}")))?;

        Ok(selection == 1)
    }

    /// Confirms migration before starting.
    pub fn confirm_migration(&self, schema: &SourceSchema, config: &WizardConfig) -> Result<bool> {
        let bold = Style::new().bold();

        println!();
        println!("{}", bold.apply_to("📊 Migration Preview:"));
        println!("┌─────────────────────────────────────────────────────────────┐");
        println!(
            "│ Source:      {} @ {}",
            schema.collection,
            config.source_type.short_name()
        );
        println!(
            "│ Vectors:     {}",
            schema
                .total_count
                .map_or("unknown".to_string(), |c| format!("{c}"))
        );
        println!("│ Dimension:   {}", schema.dimension);

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
            println!("│ Fields:      {}", fields_str);
        }

        println!("│ Destination: {}/{}", config.dest_path, config.collection);
        println!(
            "│ Compression: {}",
            if config.use_sq8 { "SQ8 (4x)" } else { "Full" }
        );
        println!("└─────────────────────────────────────────────────────────────┘");
        println!();

        Confirm::with_theme(&self.theme)
            .with_prompt("Start migration?")
            .default(true)
            .interact()
            .map_err(|e| Error::Config(format!("Input cancelled: {e}")))
    }
}
