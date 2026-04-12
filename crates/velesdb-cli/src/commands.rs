//! CLI subcommand definitions (clap `Subcommand` derive).
//!
//! All top-level `Commands` variants and their nested action enums live here.
//! `main.rs` imports these for dispatch.

use clap::Subcommand;
use clap_complete::Shell;
use std::path::PathBuf;

use crate::cli_types::{IndexTypeArg, MetricArg, StorageModeArg};
use crate::graph;

/// Top-level CLI commands for VelesDB CLI - High-performance vector database.
///
/// Standalone commands live at the top level; related commands are grouped
/// into sub-enums (`Collection`, `Data`, `Query`) for ergonomic CLI usage.
#[derive(Subcommand)]
pub enum Commands {
    /// Start interactive REPL
    Repl {
        /// Path to database directory
        #[arg(default_value = "./data")]
        path: PathBuf,
    },

    /// Show database info
    Info {
        /// Path to database directory
        path: PathBuf,
    },

    /// Generate shell completions
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

    /// License management commands
    License {
        #[command(subcommand)]
        action: LicenseAction,
    },

    /// Collection management (create, delete, list, show, analyze)
    Collection {
        #[command(subcommand)]
        action: CollectionCommands,
    },

    /// Data operations (import, export, upsert, get, delete, stream-insert)
    Data {
        #[command(subcommand)]
        action: DataCommands,
    },

    /// Query operations (execute, search, explain)
    #[command(name = "query")]
    QueryCmd {
        #[command(subcommand)]
        action: QueryCommands,
    },

    /// Graph operations (EPIC-016 US-050)
    Graph {
        #[command(subcommand)]
        action: graph::GraphAction,
    },

    /// Index management (create, drop, list)
    Index {
        #[command(subcommand)]
        action: IndexAction,
    },
}

// ---------------------------------------------------------------------------
// Collection sub-commands
// ---------------------------------------------------------------------------

/// Commands for managing collections.
#[derive(Subcommand)]
pub enum CollectionCommands {
    /// Create a vector collection with dimension, metric, and storage options
    #[command(name = "create")]
    CreateVector {
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
    #[command(name = "create-graph")]
    CreateGraph {
        /// Path to database directory
        path: PathBuf,

        /// Collection name
        name: String,

        /// Create with schemaless mode (any node/edge types accepted)
        #[arg(long, default_value = "true")]
        schemaless: bool,
    },

    /// Create a metadata-only collection (no vectors)
    #[command(name = "create-metadata")]
    CreateMetadata {
        /// Path to database directory
        path: PathBuf,

        /// Collection name
        name: String,
    },

    /// Delete a collection (vector, graph, or metadata)
    Delete {
        /// Path to database directory
        path: PathBuf,

        /// Collection name
        name: String,

        /// Skip interactive confirmation
        #[arg(long)]
        force: bool,
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
}

// ---------------------------------------------------------------------------
// Data sub-commands
// ---------------------------------------------------------------------------

/// Commands for data operations on collections.
#[derive(Subcommand)]
pub enum DataCommands {
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

    /// Delete points from a vector collection by ID
    #[command(name = "delete")]
    DeletePoints {
        /// Path to database directory
        path: PathBuf,

        /// Collection name
        collection: String,

        /// Point IDs to delete
        #[arg(required = true)]
        ids: Vec<u64>,
    },

    /// Scroll through collection points with cursor-based pagination
    Scroll {
        /// Path to database directory
        path: PathBuf,

        /// Collection name
        collection: String,

        /// Batch size (number of points per page)
        #[arg(short, long, default_value = "20")]
        batch_size: usize,

        /// Starting cursor (point ID to resume after). Omit for first page.
        #[arg(long)]
        cursor: Option<u64>,

        /// Output format (table, json)
        #[arg(short, long, default_value = "json")]
        format: String,
    },

    /// Stream-insert points from stdin (one JSON object per line)
    #[command(name = "stream-insert")]
    StreamInsert {
        /// Path to database directory
        path: PathBuf,

        /// Collection name
        collection: String,

        /// Batch size for micro-batching (points buffered before upsert)
        #[arg(short, long, default_value = "100")]
        batch_size: usize,
    },
}

// ---------------------------------------------------------------------------
// Query sub-commands
// ---------------------------------------------------------------------------

/// Commands for querying collections.
#[derive(Subcommand)]
pub enum QueryCommands {
    /// Execute a single query
    #[command(name = "execute")]
    Execute {
        /// Path to database directory
        path: PathBuf,

        /// `VelesQL` query to execute
        query: String,

        /// Output format (table, json)
        #[arg(short, long, default_value = "table")]
        format: String,
    },

    /// Perform multi-query search with fusion
    #[command(name = "search")]
    Search {
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

    /// Perform batch search: multiple independent queries in parallel
    #[command(name = "batch-search")]
    BatchSearch {
        /// Path to database directory
        path: PathBuf,

        /// Collection name
        collection: String,

        /// Query vectors as JSON array of arrays (e.g., '[[1.0, 0.0], [0.0, 1.0]]')
        vectors: String,

        /// Number of results per query
        #[arg(short = 'k', long, default_value = "10")]
        top_k: usize,

        /// Output format (table, json)
        #[arg(short, long, default_value = "table")]
        format: String,
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
}

// ---------------------------------------------------------------------------
// Existing sub-command enums (unchanged)
// ---------------------------------------------------------------------------

/// SIMD diagnostic actions.
#[derive(Subcommand)]
pub enum SimdAction {
    /// Show current SIMD dispatch configuration
    Info,

    /// Force re-benchmark of all SIMD backends
    Benchmark,
}

/// Index management actions.
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

/// License management actions.
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
