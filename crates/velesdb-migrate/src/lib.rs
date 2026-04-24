// Migration tool - pedantic lints reactivated per-module
#![allow(clippy::doc_markdown)] // Product names in crate-level docs (ChromaDB, PostgREST, etc.)
//! # `VelesDB` Migration Tool
//!
//! `velesdb-migrate` is a CLI tool and library for migrating vector data from
//! various vector databases into `VelesDB`.
//!
//! ## Supported Sources
//!
//! | Source | Status | Notes |
//! |--------|--------|-------|
//! | Qdrant | ✅ | REST API |
//! | Pinecone | ✅ | REST API |
//! | Weaviate | ✅ | GraphQL |
//! | Milvus | ✅ | REST API (v2) |
//! | `ChromaDB` | ✅ | REST API |
//! | Supabase | ✅ | PostgREST API (pgvector-enabled projects) |
//! | Elasticsearch | ✅ | Dense vector fields via REST API |
//! | Redis | ✅ | RediSearch vector fields |
//! | JSON file | ✅ | Local `.json` / `.jsonl` files |
//! | CSV file | ✅ | Local `.csv` files with vector columns |
//!
//! ## Limitations
//!
//! - **Local destination only**: migrations write to a local VelesDB data directory.
//!   Remote server migration (e.g., via HTTP to `velesdb-server`) is not supported.
//! - **MongoDB**: Not supported. The previous `mongodb` connector relied on the
//!   Atlas Data API, which MongoDB deprecated on 2025-09-30. To migrate
//!   MongoDB data, export it to JSONL with `mongoexport --collection <c> --out
//!   data.jsonl` and use `--source json_file`.
//! - **PostgreSQL (direct pgvector)**: Not supported. Use the Supabase
//!   connector for pgvector-backed Supabase projects, or export to JSON.
//!
//! ## Quick Start
//!
//! ```bash
//! # From Qdrant
//! velesdb-migrate --config migration.yaml
//!
//! # Dry run (preview only)
//! velesdb-migrate --config migration.yaml --dry-run
//! ```
//!
//! ## Configuration Example
//!
//! ```yaml
//! source:
//!   type: qdrant
//!   url: http://localhost:6333
//!   collection: documents
//!
//! destination:
//!   path: ./velesdb_data
//!   collection: docs
//!   dimension: 768
//!   metric: cosine
//!
//! options:
//!   batch_size: 1000
//!   workers: 4
//! ```

#![warn(missing_docs)]

#[allow(clippy::pedantic)]
pub mod config;
#[allow(clippy::pedantic)]
pub mod connectors;
#[allow(clippy::pedantic)]
pub mod error;
#[allow(clippy::pedantic)]
pub mod pipeline;
#[allow(clippy::pedantic)]
pub mod pipeline_graph;
#[allow(clippy::pedantic)]
pub(crate) mod pipeline_points;
#[allow(clippy::pedantic)]
pub mod retry;
#[allow(clippy::pedantic)]
mod source_builders;
#[allow(clippy::pedantic)]
pub mod source_config_builder;
#[allow(clippy::pedantic)]
pub mod transform;
#[allow(clippy::pedantic)]
pub mod wizard;

pub use config::{MigrationConfig, MigrationOptions, SourceConfig};
pub use connectors::{ExtractedBatch, ExtractedPoint, SourceConnector, SourceSchema};
pub use error::{Error, Result};
pub use pipeline::{MigrationStats, Pipeline};
pub use transform::Transformer;
pub use wizard::Wizard;
