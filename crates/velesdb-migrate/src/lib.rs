// Clippy allows for migration tool - pragmatic choices for CLI tool
#![allow(clippy::cast_possible_truncation)] // Offsets are typically small
#![allow(clippy::cast_precision_loss)] // Stats display doesn't need perfect precision
#![allow(clippy::cast_sign_loss)] // Quantization is intentional
#![allow(clippy::too_many_lines)] // Complex async functions are acceptable
#![allow(clippy::missing_errors_doc)] // Internal functions
#![allow(clippy::unnecessary_wraps)] // Consistency with trait bounds
#![allow(clippy::manual_string_new)] // Style preference
#![allow(clippy::float_cmp)] // Test assertions

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
//! | Supabase | ✅ | Via `PostgREST` API |
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
#![warn(clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

pub mod config;
pub mod connectors;
pub mod error;
pub mod pipeline;
pub mod transform;

pub use config::{MigrationConfig, MigrationOptions, SourceConfig};
pub use connectors::{ExtractedBatch, ExtractedPoint, SourceConnector, SourceSchema};
pub use error::{Error, Result};
pub use pipeline::{MigrationStats, Pipeline};
pub use transform::Transformer;
