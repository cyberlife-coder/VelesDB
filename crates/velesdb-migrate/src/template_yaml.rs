//! Static YAML configuration templates for each supported migration source.

pub(super) const QDRANT_TEMPLATE: &str = r#"# VelesDB Migration Configuration - Qdrant Source
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
  storage_mode: full  # full, sq8, binary, pq, rabitq

options:
  batch_size: 1000
  workers: 4
  dry_run: false
  continue_on_error: false
"#;

pub(super) const PINECONE_TEMPLATE: &str = r#"# VelesDB Migration Configuration - Pinecone Source
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

pub(super) const WEAVIATE_TEMPLATE: &str = r#"# VelesDB Migration Configuration - Weaviate Source
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

pub(super) const MILVUS_TEMPLATE: &str = r#"# VelesDB Migration Configuration - Milvus Source
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

pub(super) const CHROMADB_TEMPLATE: &str = r#"# VelesDB Migration Configuration - ChromaDB Source
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

pub(super) const SUPABASE_TEMPLATE: &str = r#"# VelesDB Migration Configuration - Supabase Source
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

// =============================================================================
// YAML generation functions
// =============================================================================

use super::AutoConfigParams;

pub(super) fn generate_supabase_yaml(
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

pub(super) fn generate_weaviate_yaml(
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

pub(super) fn generate_simple_yaml(
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

pub(super) fn generate_generic_yaml(
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
