//! CLI subcommand definitions (clap `Subcommand` derive).
//!
//! All top-level `Commands` variants and their nested action enums live here.
//! `main.rs` imports these for dispatch.

use clap::Subcommand;
use clap_complete::Shell;
use std::path::PathBuf;

use crate::cli_types::{IndexTypeArg, MetricArg, StorageModeArg};
use crate::graph;

#[derive(Subcommand)]
pub enum Commands {
    /// Start interactive REPL
    Repl {
        /// Path to database directory
        #[arg(default_value = "./data")]
        path: PathBuf,
    },

    /// Execute a single query
    Query {
        /// Path to database directory
        path: PathBuf,

        /// `VelesQL` query to execute
        query: String,

        /// Output format (table, json)
        #[arg(short, long, default_value = "table")]
        format: String,
    },

    /// Show database info
    Info {
        /// Path to database directory
        path: PathBuf,
    },

    /// List all collections in the database
    List {
        /// Path to database directory
        path: PathBuf,

        /// Output format (table, json)
        #[arg(short, long, default_value = "table")]
        format: String,
    },

    /// Show detailed information about a collection
    Show {
        /// Path to database directory
        path: PathBuf,

        /// Collection name
        collection: String,

        /// Show sample records
        #[arg(short, long, default_value = "0")]
        samples: usize,

        /// Output format (table, json)
        #[arg(short, long, default_value = "table")]
        format: String,
    },

    /// Export a collection to JSON file
    Export {
        /// Path to database directory
        path: PathBuf,

        /// Collection name
        collection: String,

        /// Output file path
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Include vectors in export
        #[arg(long, default_value = "true")]
        include_vectors: bool,
    },

    /// Import vectors from CSV or JSONL file
    Import {
        /// Path to data file (CSV or JSONL)
        file: PathBuf,

        /// Path to database directory
        #[arg(short, long, default_value = "./data")]
        database: PathBuf,

        /// Collection name
        #[arg(short, long)]
        collection: String,

        /// Vector dimension (auto-detected if not specified)
        #[arg(long)]
        dimension: Option<usize>,

        /// Distance metric
        #[arg(long, value_enum, default_value = "cosine")]
        metric: MetricArg,

        /// Storage mode (full, sq8, binary)
        #[arg(long, value_enum, default_value = "full")]
        storage_mode: StorageModeArg,

        /// ID column name (for CSV)
        #[arg(long, default_value = "id")]
        id_column: String,

        /// Vector column name (for CSV)
        #[arg(long, default_value = "vector")]
        vector_column: String,

        /// Batch size for insertion
        #[arg(long, default_value = "1000")]
        batch_size: usize,

        /// Show progress bar
        #[arg(long, default_value = "true")]
        progress: bool,
    },

    /// License management commands
    License {
        #[command(subcommand)]
        action: LicenseAction,
    },

    /// Create a metadata-only collection (no vectors)
    CreateMetadataCollection {
        /// Path to database directory
        path: PathBuf,

        /// Collection name
        name: String,
    },

    /// Get a point by ID
    Get {
        /// Path to database directory
        path: PathBuf,

        /// Collection name
        collection: String,

        /// Point ID to retrieve
        id: u64,

        /// Output format (table, json)
        #[arg(short, long, default_value = "json")]
        format: String,
    },

    /// Perform multi-query search with fusion
    MultiSearch {
        /// Path to database directory
        path: PathBuf,

        /// Collection name
        collection: String,

        /// Query vectors as JSON array of arrays (e.g., '[[1.0, 0.0], [0.0, 1.0]]')
        vectors: String,

        /// Number of results to return
        #[arg(short = 'k', long, default_value = "10")]
        top_k: usize,

        /// Fusion strategy (average, maximum, rrf, weighted)
        #[arg(short, long, default_value = "rrf")]
        strategy: String,

        /// RRF k parameter (only for rrf strategy)
        #[arg(long, default_value = "60")]
        rrf_k: u32,

        /// Output format (table, json)
        #[arg(short, long, default_value = "table")]
        format: String,
    },

    /// Graph operations (EPIC-016 US-050)
    Graph {
        #[command(subcommand)]
        action: graph::GraphAction,
    },

    /// Generate shell completions (EPIC-014 US-007)
    Completions {
        /// Shell type (bash, zsh, fish, powershell, elvish)
        #[arg(value_enum)]
        shell: Shell,
    },

    /// SIMD performance diagnostics and benchmarking
    Simd {
        #[command(subcommand)]
        action: SimdAction,
    },

    /// Create a vector collection with dimension, metric, and storage options
    CreateVectorCollection {
        /// Path to database directory
        path: PathBuf,

        /// Collection name
        name: String,

        /// Vector dimension
        #[arg(short, long)]
        dimension: usize,

        /// Distance metric (cosine, euclidean, dot, hamming, jaccard)
        #[arg(short, long, value_enum, default_value = "cosine")]
        metric: MetricArg,

        /// Storage mode (full, sq8, binary, pq, rabitq)
        #[arg(short, long, value_enum, default_value = "full")]
        storage: StorageModeArg,
    },

    /// Create a graph collection
    CreateGraphCollection {
        /// Path to database directory
        path: PathBuf,

        /// Collection name
        name: String,

        /// Create with schemaless mode (any node/edge types accepted)
        #[arg(long, default_value = "true")]
        schemaless: bool,
    },

    /// Delete a collection (vector, graph, or metadata)
    DeleteCollection {
        /// Path to database directory
        path: PathBuf,

        /// Collection name
        name: String,

        /// Skip interactive confirmation
        #[arg(long)]
        force: bool,
    },

    /// Show the query execution plan (EXPLAIN) for a VelesQL query
    Explain {
        /// Path to database directory
        path: PathBuf,

        /// VelesQL query to explain
        query: String,

        /// Output format (tree, json)
        #[arg(short, long, default_value = "tree")]
        format: String,
    },

    /// Analyze a collection and display statistics
    Analyze {
        /// Path to database directory
        path: PathBuf,

        /// Collection name
        collection: String,

        /// Output format (table, json)
        #[arg(short, long, default_value = "table")]
        format: String,
    },

    /// Delete points from a vector collection by ID
    DeletePoints {
        /// Path to database directory
        path: PathBuf,

        /// Collection name
        collection: String,

        /// Point IDs to delete
        #[arg(required = true)]
        ids: Vec<u64>,
    },

    /// Upsert a single point into a vector collection
    Upsert {
        /// Path to database directory
        path: PathBuf,

        /// Collection name
        collection: String,

        /// Point ID
        #[arg(long)]
        id: u64,

        /// Vector as JSON array (e.g., '[0.1, 0.2, 0.3]')
        #[arg(long)]
        vector: Option<String>,

        /// Payload as JSON object (e.g., '{"title": "Hello"}')
        #[arg(long)]
        payload: Option<String>,
    },

    /// Index management (create, drop, list)
    Index {
        #[command(subcommand)]
        action: IndexAction,
    },
}

#[derive(Subcommand)]
pub enum SimdAction {
    /// Show current SIMD dispatch configuration
    Info,

    /// Force re-benchmark of all SIMD backends
    Benchmark,
}

#[derive(Subcommand)]
pub enum IndexAction {
    /// Create an index on a collection field
    Create {
        /// Path to database directory
        path: PathBuf,

        /// Collection name
        collection: String,

        /// Field name to index
        field: String,

        /// Index type (secondary, property, range)
        #[arg(long, value_enum, default_value = "secondary")]
        index_type: IndexTypeArg,

        /// Label (required for property and range index types)
        #[arg(long)]
        label: Option<String>,
    },

    /// Drop an index from a collection
    Drop {
        /// Path to database directory
        path: PathBuf,

        /// Collection name
        collection: String,

        /// Label of the index to drop
        label: String,

        /// Property of the index to drop
        property: String,
    },

    /// List all indexes on a collection
    List {
        /// Path to database directory
        path: PathBuf,

        /// Collection name
        collection: String,

        /// Output format (table, json)
        #[arg(short, long, default_value = "table")]
        format: String,
    },
}

#[derive(Subcommand)]
pub enum LicenseAction {
    /// Show current license status
    Show,

    /// Activate a license key
    Activate {
        /// License key from email (format: base64_payload.base64_signature)
        key: String,
    },

    /// Verify a license key without activating it
    Verify {
        /// License key to verify
        key: String,

        /// Public key for verification (base64 encoded)
        #[arg(short, long)]
        public_key: String,
    },
}
