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
        "pgvector" => Some(PGVECTOR_TEMPLATE),
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

fn generate_supabase_yaml(
    params: &AutoConfigParams<'_>,
    count_str: &str,
    dimension: usize,
    vector_col: &str,
    id_col: &str,
    fields_list: &str,
) -> String {
    format!(
        r#"# VelesDB Migration Configuration - AUTO-GENERATED
# Source: Supabase
# Detected: {count_str} vectors, {dimension}D

source:
  type: supabase
  url: {url}
  api_key: ${{SUPABASE_SERVICE_KEY}}  # Set env var for security
  table: {collection}
  vector_column: {vector_col}
  id_column: {id_col}
  payload_columns:
{fields_list}

destination:
  path: {dest}
  collection: {collection}
  dimension: {dimension}
  metric: cosine
  storage_mode: full

options:
  batch_size: 500
  workers: 2
  continue_on_error: false
"#,
        url = params.url,
        collection = params.collection,
        dest = params.dest_path.display(),
    )
}

fn generate_weaviate_yaml(
    params: &AutoConfigParams<'_>,
    count_str: &str,
    dimension: usize,
    api_key_line: &str,
    fields_list: &str,
) -> String {
    format!(
        r#"# VelesDB Migration Configuration - AUTO-GENERATED
# Source: Weaviate
# Detected: {count_str} objects, {dimension}D

source:
  type: weaviate
  url: {url}
  class_name: {collection}
{api_key_line}
  properties:  # Detected properties:
{fields_list}

destination:
  path: {dest}
  collection: {collection}
  dimension: {dimension}
  metric: cosine
  storage_mode: full

options:
  batch_size: 1000
"#,
        url = params.url,
        collection = params.collection,
        dest = params.dest_path.display(),
    )
}

fn generate_simple_yaml(
    source_label: &str,
    source_type: &str,
    params: &AutoConfigParams<'_>,
    count_str: &str,
    dimension: usize,
    extra_line: &str,
) -> String {
    let api_section = if extra_line.is_empty() {
        String::new()
    } else {
        format!("\n{extra_line}")
    };

    format!(
        r#"# VelesDB Migration Configuration - AUTO-GENERATED
# Source: {source_label}
# Detected: {count_str} vectors, {dimension}D

source:
  type: {source_type}
  url: {url}
  collection: {collection}{api_section}
  payload_fields: []  # Empty = all fields

destination:
  path: {dest}
  collection: {collection}
  dimension: {dimension}
  metric: cosine
  storage_mode: full

options:
  batch_size: 1000
  workers: 4
"#,
        url = params.url,
        collection = params.collection,
        dest = params.dest_path.display(),
    )
}

fn generate_generic_yaml(
    params: &AutoConfigParams<'_>,
    count_str: &str,
    dimension: usize,
) -> String {
    format!(
        r#"# VelesDB Migration Configuration - AUTO-GENERATED
# Source: {source_type}
# Detected: {count_str} vectors, {dimension}D

source:
  type: {source_type_lower}
  url: {url}
  collection: {collection}

destination:
  path: {dest}
  collection: {collection}
  dimension: {dimension}
  metric: cosine
  storage_mode: full

options:
  batch_size: 1000
"#,
        source_type = params.source_type,
        source_type_lower = params.source_type.to_lowercase(),
        url = params.url,
        collection = params.collection,
        dest = params.dest_path.display(),
    )
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

const QDRANT_TEMPLATE: &str = r#"# VelesDB Migration Configuration - Qdrant Source
source:
  type: qdrant
  url: http://localhost:6333
  collection: your_collection
  # api_key: your-api-key  # Optional

destination:
  path: ./velesdb_data
  collection: migrated_docs
  dimension: 768
  metric: cosine  # cosine, euclidean, or dot
  storage_mode: full  # full, sq8, or binary

options:
  batch_size: 1000
  workers: 4
  dry_run: false
  continue_on_error: false
"#;

const PINECONE_TEMPLATE: &str = r#"# VelesDB Migration Configuration - Pinecone Source
source:
  type: pinecone
  api_key: your-pinecone-api-key
  environment: us-east-1-aws
  index: your-index-name
  # namespace: optional-namespace

destination:
  path: ./velesdb_data
  collection: migrated_docs
  dimension: 768
  metric: cosine

options:
  batch_size: 100  # Pinecone has lower batch limits
  workers: 2
"#;

const WEAVIATE_TEMPLATE: &str = r#"# VelesDB Migration Configuration - Weaviate Source
source:
  type: weaviate
  url: http://localhost:8080
  class_name: Document
  # api_key: your-api-key  # Optional
  properties:
    - title
    - content

destination:
  path: ./velesdb_data
  collection: migrated_docs
  dimension: 768
  metric: cosine

options:
  batch_size: 1000
"#;

const MILVUS_TEMPLATE: &str = r#"# VelesDB Migration Configuration - Milvus Source
source:
  type: milvus
  url: http://localhost:19530
  collection: your_collection
  # username: root
  # password: milvus

destination:
  path: ./velesdb_data
  collection: migrated_docs
  dimension: 768
  metric: cosine

options:
  batch_size: 1000
"#;

const CHROMADB_TEMPLATE: &str = r#"# VelesDB Migration Configuration - ChromaDB Source
source:
  type: chromadb
  url: http://localhost:8000
  collection: your_collection
  # tenant: default_tenant
  # database: default_database

destination:
  path: ./velesdb_data
  collection: migrated_docs
  dimension: 768
  metric: cosine

options:
  batch_size: 1000
"#;

const PGVECTOR_TEMPLATE: &str = r#"# VelesDB Migration Configuration - pgvector Source
# Requires: velesdb-migrate --features postgres
source:
  type: pgvector
  connection_string: postgres://user:password@localhost:5432/database
  table: embeddings
  vector_column: embedding
  id_column: id
  payload_columns:
    - title
    - content
  # filter: "created_at > '2024-01-01'"

destination:
  path: ./velesdb_data
  collection: migrated_docs
  dimension: 768
  metric: cosine

options:
  batch_size: 1000
"#;

const SUPABASE_TEMPLATE: &str = r#"# VelesDB Migration Configuration - Supabase Source
source:
  type: supabase
  url: https://your-project.supabase.co
  api_key: your-service-role-key
  table: documents
  vector_column: embedding
  id_column: id
  payload_columns:
    - title
    - content

destination:
  path: ./velesdb_data
  collection: migrated_docs
  dimension: 768
  metric: cosine

options:
  batch_size: 1000
"#;
