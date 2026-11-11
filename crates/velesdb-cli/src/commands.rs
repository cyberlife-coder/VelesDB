//! CLI subcommand definitions (clap `Subcommand` derive).
//!
//! All top-level `Commands` variants and their nested action enums live here.
//! `main.rs` imports these for dispatch.

use clap::{Subcommand, ValueEnum};
use clap_complete::Shell;
use std::path::PathBuf;

use crate::cli_types::{ExportFormatArg, IndexTypeArg, MetricArg, StorageModeArg};
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

    /// SIMD diagnostics: the level detected on this machine
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

    /// Engine configuration the CLI loads (path, show, validate, edit)
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },

    /// Database maintenance (stats, vacuum, snapshot, restore)
    Db {
        #[command(subcommand)]
        action: DbAction,
    },

    /// Start the embedded REST API server in-process (requires the `serve`
    /// cargo feature; flags and VELESDB_* env vars mirror velesdb-server's)
    #[cfg(feature = "serve")]
    Serve {
        /// Data directory for persistent storage
        #[arg(short, long, env = "VELESDB_DATA_DIR")]
        data_dir: Option<String>,

        /// Host address to bind to
        #[arg(long, env = "VELESDB_HOST")]
        host: Option<String>,

        /// Port to listen on
        #[arg(short, long, env = "VELESDB_PORT")]
        port: Option<u16>,

        /// TLS certificate file (PEM)
        #[arg(long, env = "VELESDB_TLS_CERT")]
        tls_cert: Option<String>,

        /// TLS private key file (PEM)
        #[arg(long, env = "VELESDB_TLS_KEY")]
        tls_key: Option<String>,

        /// Rate limit: max requests per second per IP (0 = disabled)
        #[arg(long, env = "VELESDB_RATE_LIMIT")]
        rate_limit: Option<u32>,

        /// Host names to answer to besides localhost, 127.0.0.0/8, [::1] and the
        /// bind host (comma-separated; "*" disables the Host check)
        #[arg(long, env = "VELESDB_ALLOWED_HOSTS", value_delimiter = ',')]
        allowed_hosts: Option<Vec<String>>,
    },

    /// `velesdb serve` placeholder so a CLI-only build fails with a clear
    /// message instead of `unrecognized subcommand`
    #[cfg(not(feature = "serve"))]
    Serve,

    /// Ask the bounded ReAct agent a question about this database
    /// (whitelisted VelesQL tools; Ollama backend if VELESDB_AGENT_URL
    /// answers, else deterministic offline mode)
    Agent {
        /// Path to database directory
        path: PathBuf,

        /// Natural-language question (trailing args, quoting optional)
        #[arg(trailing_var_arg = true)]
        question: Vec<String>,

        /// Output format (table, json)
        #[arg(short, long, default_value = "table")]
        format: String,

        /// Reason→Act cycle cap (1..=32)
        #[arg(long, default_value_t = crate::agent::MaxIterations::default())]
        max_iterations: crate::agent::MaxIterations,

        /// Also write the full transcript as JSON to this file
        #[arg(long, value_name = "FILE")]
        save: Option<PathBuf>,
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

        /// Node-embedding dimension; enables `graph search` on the collection
        #[arg(short, long)]
        dimension: Option<usize>,

        /// Distance metric for node embeddings (cosine, euclidean, dot, hamming, jaccard)
        #[arg(short, long, value_enum, default_value = "cosine")]
        metric: MetricArg,
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

        /// Distance metric — only checked when importing into an existing
        /// collection; a new collection defaults to cosine
        #[arg(long, value_enum)]
        metric: Option<MetricArg>,

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

        /// Show progress bar (pass `--progress false` to disable)
        #[arg(
            long,
            default_value_t = true,
            num_args = 0..=1,
            default_missing_value = "true",
            action = clap::ArgAction::Set
        )]
        progress: bool,
    },

    /// Export a collection to a JSON, JSONL or CSV file
    Export {
        /// Path to database directory
        path: PathBuf,

        /// Collection name
        collection: String,

        /// Output file path (default: `<collection>.<format-ext>`)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Output file format (default: inferred from `--output` extension, else json)
        #[arg(long, value_enum)]
        format: Option<ExportFormatArg>,

        /// VelesQL filter condition applied to exported rows (e.g. "cat = 'tech' AND id < 10")
        #[arg(long = "where")]
        where_clause: Option<String>,

        /// Include vectors in export (pass `--include-vectors false` to omit them)
        #[arg(
            long,
            default_value_t = true,
            num_args = 0..=1,
            default_missing_value = "true",
            action = clap::ArgAction::Set
        )]
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

        /// Target collection for queries without a FROM clause (e.g. bare MATCH),
        /// mirroring the REST `/query` collection field
        #[arg(short, long)]
        collection: Option<String>,

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

        /// Fusion strategy (average/avg, maximum/max, rrf, weighted_rrf, weighted, relative_score/rsf)
        #[arg(short, long, default_value = "rrf")]
        strategy: String,

        /// RRF k parameter (only for rrf and weighted_rrf strategies)
        #[arg(long, default_value = "60")]
        rrf_k: u32,

        /// Per-branch weights for weighted_rrf, comma-separated, one per
        /// query vector (e.g. '0.7,0.3')
        #[arg(long, value_delimiter = ',')]
        branch_weights: Option<Vec<f32>>,

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
    /// Show the SIMD level detected on this machine
    Info,

    /// Point to the SIMD micro-benchmarks (`cargo bench`); measures nothing itself
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

/// `velesdb config` actions. All resolve the file the way every
/// database-opening command does: `--config`/`VELESDB_CONFIG`, else
/// `./velesdb.toml`, else the core defaults.
#[derive(Subcommand)]
pub enum ConfigAction {
    /// Print the config file the CLI reads (nothing on stdout when only the
    /// core defaults apply)
    Path,

    /// Print the effective engine configuration: defaults, then the file,
    /// then the VELESDB_* engine variables
    Show {
        /// Output format (toml, json)
        #[arg(short, long, value_enum, default_value = "toml")]
        format: ConfigFormat,
    },

    /// Validate a config file (default: the one `config path` reports);
    /// exits 1 when it is invalid
    Validate {
        /// File to validate instead of the resolved one
        file: Option<PathBuf>,
    },

    /// Open the config file in $VISUAL or $EDITOR, then validate it
    Edit,
}

/// Output format of `velesdb config show`.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ConfigFormat {
    /// TOML, loadable back with `--config`
    Toml,
    /// JSON
    Json,
}

/// `velesdb db` maintenance actions. All operate offline against a database
/// directory and take its exclusive lock like every other command.
#[derive(Subcommand)]
pub enum DbAction {
    /// Per-collection stats: kind, point/node count, disk usage, last ANALYZE
    Stats {
        /// Path to database directory
        path: PathBuf,

        /// Output format (table, json)
        #[arg(short, long, default_value = "table")]
        format: String,
    },

    /// Vacuum: rebuild the HNSW index of each targeted vector collection,
    /// removing tombstoned entries
    Vacuum {
        /// Path to database directory
        path: PathBuf,

        /// Only this collection (must be a vector collection; default: all)
        #[arg(short, long)]
        collection: Option<String>,

        /// Also rewrite vector storage, reclaiming disk space
        #[arg(long)]
        compact: bool,
    },

    /// Snapshot: flush the database and copy it into a fresh directory.
    /// Opens the database, so it takes the same exclusive lock every other
    /// command does — a running server or REPL on the directory fails it.
    Snapshot {
        /// Path to database directory
        path: PathBuf,

        /// Destination directory (must not exist or be empty)
        #[arg(short, long)]
        output: PathBuf,
    },

    /// Restore: verify a snapshot's manifest, copy it into a fresh directory
    /// and open it to prove it loads
    Restore {
        /// Path to snapshot directory (made by `db snapshot`)
        path: PathBuf,

        /// Destination database directory (must not exist or be empty)
        #[arg(short, long)]
        output: PathBuf,
    },
}
