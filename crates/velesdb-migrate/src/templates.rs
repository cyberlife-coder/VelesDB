//! YAML configuration templates for migration sources.
//!
//! Provides static YAML templates and auto-generated configuration
//! from detected source schemas.

use velesdb_migrate::connectors::SourceSchema;

/// Returns the YAML template for the given source type, or `None` if unknown.
pub fn get_template(source: &str) -> Option<&'static str> {
    match source.to_lowercase().as_str() {
        "qdrant" => Some(QDRANT_TEMPLATE),
        "pinecone" => Some(PINECONE_TEMPLATE),
        "weaviate" => Some(WEAVIATE_TEMPLATE),
        "milvus" => Some(MILVUS_TEMPLATE),
        "chromadb" => Some(CHROMADB_TEMPLATE),
        "supabase" => Some(SUPABASE_TEMPLATE),
        _ => None,
    }
}

/// Parameters for auto-generating a migration config YAML.
pub struct AutoConfigParams<'a> {
    pub source_type: &'a str,
    pub url: &'a str,
    pub collection: &'a str,
    pub api_key: Option<&'a str>,
    pub dest_path: &'a std::path::Path,
    pub schema: &'a SourceSchema,
}

/// Generates a YAML configuration string from auto-detected schema.
pub fn generate_auto_config(params: &AutoConfigParams<'_>) -> String {
    let dimension = if params.schema.dimension > 0 {
        params.schema.dimension
    } else {
        768
    };

    let detected_vector_col = detect_vector_column(params.schema);
    let detected_id_col = detect_id_column(params.schema);
    let fields_list = build_fields_list(params.schema, &detected_id_col, &detected_vector_col);

    let count_str = params
        .schema
        .total_count
        .map_or_else(|| "?".to_string(), |c| c.to_string());

    let api_key_line = params.api_key.map_or_else(
        || "  # api_key: your-key".to_string(),
        |k| format!("  api_key: {k}"),
    );

    match params.source_type.to_lowercase().as_str() {
        "supabase" => generate_supabase_yaml(
            params,
            &count_str,
            dimension,
            &detected_vector_col,
            &detected_id_col,
            &fields_list,
        ),
        "qdrant" => generate_simple_yaml(
            "Qdrant",
            "qdrant",
            params,
            &count_str,
            dimension,
            &api_key_line,
        ),
        "chromadb" => {
            generate_simple_yaml("ChromaDB", "chromadb", params, &count_str, dimension, "")
        }
        "weaviate" => {
            generate_weaviate_yaml(params, &count_str, dimension, &api_key_line, &fields_list)
        }
        _ => generate_generic_yaml(params, &count_str, dimension),
    }
}

/// Detects the vector column name from schema metadata or field heuristics.
fn detect_vector_column(schema: &SourceSchema) -> String {
    schema.vector_column.clone().unwrap_or_else(|| {
        schema
            .fields
            .iter()
            .find(|f| {
                let lower = f.name.to_lowercase();
                lower.contains("vector") || lower.contains("embedding") || lower.contains("emb")
            })
            .map(|f| f.name.clone())
            .unwrap_or_else(|| "embedding".to_string())
    })
}

/// Detects the ID column name from schema metadata or field heuristics.
fn detect_id_column(schema: &SourceSchema) -> String {
    schema.id_column.clone().unwrap_or_else(|| {
        schema
            .fields
            .iter()
            .find(|f| {
                let lower = f.name.to_lowercase();
                lower.contains("id") || lower == "code" || lower.ends_with("_id")
            })
            .map(|f| f.name.clone())
            .unwrap_or_else(|| "id".to_string())
    })
}

/// Builds the YAML fields list, excluding ID and vector columns.
fn build_fields_list(schema: &SourceSchema, id_col: &str, vector_col: &str) -> String {
    let payload_fields: Vec<_> = schema
        .fields
        .iter()
        .filter(|f| f.name != id_col && f.name != vector_col)
        .collect();

    if payload_fields.is_empty() {
        "    # All metadata fields will be migrated automatically".to_string()
    } else {
        payload_fields
            .iter()
            .map(|f| format!("    - {}", f.name))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Prints the detected schema summary to stdout.
pub fn print_schema_summary(schema: &SourceSchema) {
    println!("\n✅ Schema Detected!");
    println!("┌─────────────────────────────────────────────");
    println!("│ Source Type:  {}", schema.source_type);
    println!("│ Collection:   {}", schema.collection);
    println!(
        "│ Dimension:    {}",
        if schema.dimension > 0 {
            schema.dimension.to_string()
        } else {
            "auto-detect on first batch".to_string()
        }
    );
    println!(
        "│ Total Count:  {}",
        schema
            .total_count
            .map_or_else(|| "unknown".to_string(), |c| format!("{c} vectors"))
    );
    println!("├─────────────────────────────────────────────");

    if !schema.fields.is_empty() {
        println!("│ Detected Metadata Fields:");
        for field in &schema.fields {
            let indexed = if field.indexed { " [indexed]" } else { "" };
            println!("│   • {} ({}){indexed}", field.name, field.field_type);
        }
    } else {
        println!("│ Metadata Fields: (all fields will be migrated)");
    }
    println!("└─────────────────────────────────────────────");
}

use template_yaml::*;
#[path = "template_yaml.rs"]
mod template_yaml;

#[cfg(test)]
#[path = "templates_tests.rs"]
mod tests;
