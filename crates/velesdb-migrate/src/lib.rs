// Migration tool - pedantic lints reactivated per-module
#![allow(clippy::doc_markdown)] // Product names in crate-level docs (MongoDB, ChromaDB, etc.)
//! # `VelesDB` Migration Tool
//!
//! `velesdb-migrate` is a CLI tool and library for migrating vector data from
//! various vector databases into `VelesDB`.
//!
//! ## Supported Sources
//!
//! | Source | Status | Notes |
//! |--------|--------|-------|
//! | Qdrant | ✅ | Full support via REST API |
//! | Pinecone | ✅ | Full support via REST API |
//! | Weaviate | ✅ | Full support via GraphQL |
//! | Milvus | ✅ | REST API (v2) |
//! | `ChromaDB` | ✅ | Full support via REST API |
//! | pgvector | ✅ | Requires `postgres` feature |
//! | Supabase | ✅ | Via pgvector connector (`PostgREST` API) |
//! | Elasticsearch | ✅ | Dense vector fields via REST API |
//! | MongoDB Atlas | ✅ | Atlas Vector Search via REST API (not self-hosted) |
//! | Redis | ✅ | RediSearch vector fields |
//! | JSON file | ✅ | Local `.json` / `.jsonl` files |
//! | CSV file | ✅ | Local `.csv` files with vector columns |
//!
//! ## Limitations
//!
//! - **Local destination only**: migrations write to a local VelesDB data directory.
//!   Remote server migration (e.g., via HTTP to `velesdb-server`) is not supported.
//! - **MongoDB**: Only Atlas deployments are supported (REST API). Self-hosted
//!   MongoDB with vector search requires manual export to JSON first.
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
