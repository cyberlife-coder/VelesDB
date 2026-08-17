//! Opening the store and assembling the configured MCP server: lock
//! retries, embedding provenance, autograph wiring, config-file and TTL
//! application.

use crate::backends::{
    build_embedder, build_migration_target, build_remote_extractor,
    warn_if_extraction_backend_is_unreachable, ConfiguredEmbedder,
};

use std::time::Duration;

use velesdb_memory::mcp::McpServer;
use velesdb_memory::{DynEmbedder, ExtractorSelection, MemoryService, NativeStore};

pub(crate) fn build_configured_server(
    configured: ConfiguredService,
) -> Result<McpServer, Box<dyn std::error::Error>> {
    let ConfiguredService {
        service,
        store_path,
        embedder_model,
        embedder_dimension,
    } = configured;
    let store_path = std::path::PathBuf::from(store_path);
    apply_ingest_roots(apply_default_ttl(
        build_server(service)?
            .with_embedder_identity(embedder_model, embedder_dimension)
            .with_store_dir(&store_path)
            .with_online_migration(&store_path, build_migration_target)?
            .with_extraction_jobs(&store_path)
            .map_err(std::io::Error::other)?,
    )?)
}

/// Attempts before giving up on a locked store and printing the actionable
/// error. Three short tries (with [`LOCK_RETRY_DELAY`] between them) is
/// enough to ride out the handover between one session's process exiting
/// and the next one starting — the case the retry is *for* — without making
/// a genuinely-stuck lock (the leaked-process scenario from #1448) hang
/// startup for long.
pub(crate) const LOCK_RETRY_ATTEMPTS: u32 = 3;

/// Delay between retries of an already-locked store. See
/// [`LOCK_RETRY_ATTEMPTS`] for the reasoning on the total budget.
pub(crate) const LOCK_RETRY_DELAY: Duration = Duration::from_millis(500);

/// Open the native store at `store_path`, retrying briefly through a
/// `DatabaseLocked` error before giving up with actionable stderr guidance.
///
/// Bypasses [`MemoryService::open`] in favor of [`NativeStore::open`] +
/// [`MemoryService::with_store`] because the retry only needs
/// `embedder.dimension()` (a plain `usize`, trivially reusable across
/// attempts) — not the embedder itself — so `embedder` can move into the
/// service exactly once, on the attempt that finally succeeds, with no
/// `Clone` bound required on `E`.
///
/// # Errors
/// Returns any [`MemoryError`] other than `DatabaseLocked` unchanged (e.g. a
/// dimension mismatch against an existing store). On a `DatabaseLocked` that
/// outlives every retry, prints the actionable message and exits the process
/// with a non-zero status instead of returning — that message, not a
/// generic `Result` bubble-up, is the point: a bare
/// `Storage(DatabaseLocked(..))` debug dump gives a user nothing to act on
/// (#1448).
pub(crate) fn open_store_with_actionable_lock_error(
    store_path: &str,
    configured: ConfiguredEmbedder,
) -> Result<MemoryService<DynEmbedder>, Box<dyn std::error::Error>> {
    let ConfiguredEmbedder { embedder, model } = configured;
    let dimension = embedder.dimension();
    // Read BEFORE the store is opened, so a mismatch is refused instead of
    // being reported after the daemon has taken the store's single-writer
    // `flock` — and so `recorded` below genuinely means "recorded before this
    // process touched anything".
    let store_dir = std::path::Path::new(store_path);
    let recorded = velesdb_memory::embedding_provenance::read(store_dir)?;
    velesdb_memory::embedding_provenance::check(recorded.as_ref(), &model, dimension)?;
    let unrecorded = recorded.is_none();

    match open_through_lock_retries(store_path, dimension) {
        Ok(store) => {
            if unrecorded {
                record_embedding_model(store_dir, &store, &model, dimension);
            }
            Ok(MemoryService::with_store(store, embedder))
        }
        // A dimension mismatch surfaces here, from the core. When the store
        // carries no model record, that dimension was the ONLY thing that
        // could be compared, and saying so is what stops a caller reading
        // the core's message as a full compatibility verdict.
        Err(other) if unrecorded => Err(format!(
            "{other}\n{}",
            velesdb_memory::embedding_provenance::unrecorded_model_note(&model)
        )
        .into()),
        Err(other) => Err(other.into()),
    }
}

/// The lock-retry loop alone: open the store, sitting out up to
/// [`LOCK_RETRY_ATTEMPTS`] × [`LOCK_RETRY_DELAY`] of `DatabaseLocked`.
///
/// A lock that outlives every retry prints the actionable message and exits
/// the process rather than returning — that message, not a generic `Result`
/// bubble-up, is the point: a bare `Storage(DatabaseLocked(..))` debug dump
/// gives a user nothing to act on (#1448). Every other error returns to the
/// caller, which owns the provenance context this loop knows nothing about.
pub(crate) fn open_through_lock_retries(
    store_path: &str,
    dimension: usize,
) -> Result<NativeStore, velesdb_memory::MemoryError> {
    use velesdb_memory::MemoryError;

    let mut last_locked_path: Option<String> = None;
    for attempt in 0..LOCK_RETRY_ATTEMPTS {
        match NativeStore::open(store_path, dimension) {
            Err(MemoryError::Storage(velesdb_core::Error::DatabaseLocked(locked_path))) => {
                last_locked_path = Some(locked_path);
                if attempt + 1 < LOCK_RETRY_ATTEMPTS {
                    std::thread::sleep(LOCK_RETRY_DELAY);
                }
            }
            outcome => return outcome,
        }
    }

    let locked_path = last_locked_path.unwrap_or_else(|| store_path.to_owned());
    eprintln!(
        "[velesdb-memory] another velesdb-memory process holds {locked_path} — \
         kill it (pkill velesdb-memory) or point VELESDB_MEMORY_PATH elsewhere"
    );
    std::process::exit(1);
}

/// Record which embedding model filled this store — **only when it holds no
/// facts**.
///
/// The emptiness test is what makes the record trustworthy, and it is
/// semantic rather than filesystem-shaped on purpose: "the directory looks
/// new" is defeated by a config file sitting beside the store, or by a
/// `.DS_Store` a file browser dropped in it. "No fact is stored" is the thing
/// that actually matters — with zero vectors, there is nothing that could have
/// come from a different model, so writing the record states something true.
/// Over existing data it would not: one open with the wrong model would carve
/// a false provenance that every later check would trust.
///
/// A failed write is a warning, never fatal. The daemon runs perfectly without
/// the record — it only loses the model half of the check — and refusing to
/// start because a metadata file could not be written would cost more than the
/// gap it guards.
pub(crate) fn record_embedding_model(
    store_dir: &std::path::Path,
    store: &NativeStore,
    model: &str,
    dimension: usize,
) {
    use velesdb_memory::embedding_provenance::{write, EmbeddingProvenance};
    use velesdb_memory::MemoryStore as _;

    if store.count() != 0 {
        return;
    }
    if let Err(err) = write(store_dir, &EmbeddingProvenance::new(model, dimension)) {
        if std::env::var_os("VELESDB_MEMORY_QUIET").is_none() {
            eprintln!(
                "[velesdb-memory] could not record the embedding model ({err}) — the store works, \
                 but a later model change will only be checked against the vector dimension"
            );
        }
    }
}

/// Default store location when `VELESDB_MEMORY_PATH` is unset: `~/.velesdb-memory`
/// (the path advertised in `server.json`, the README, and every client-config
/// snippet). A stable home-based path — never a `./`-relative one: an MCP server
/// is launched by its client with an unpredictable working directory, so a
/// cwd-relative default would scatter (or lose) the store between sessions. Falls
/// back to a cwd-relative path only when no home directory can be resolved.
/// Load the config file, then build the store-backed service it describes.
///
/// The config file is read BEFORE the first variable is consulted, because it
/// can set any of them — including the store path. Everything downstream keeps
/// reading the environment exactly as it always did; the file only fills in
/// what the environment left unset, which is what makes the precedence
/// `command line > environment > file > default`.
pub(crate) fn build_configured_service(
    args: &[String],
) -> Result<ConfiguredService, Box<dyn std::error::Error>> {
    apply_config_file(args)?;
    let store_path = std::env::var("VELESDB_MEMORY_PATH").unwrap_or_else(|_| default_store_path());
    let recovery = velesdb_memory::migration::recover_online_migration_startup(
        std::path::Path::new(&store_path),
        build_migration_target,
    )?;
    let configured = embedder_after_startup_recovery(recovery)?;
    // Kept for `memory_status`, which reports the RUNNING embedder: the
    // `ConfiguredEmbedder` itself is consumed by the store-open path below,
    // and the service only ever sees `&[f32]` — this is the one place the
    // resolved identity still exists.
    let embedder_model = configured.model.clone();
    let embedder_dimension = configured.embedder.dimension();
    let service = apply_autograph(open_store_with_actionable_lock_error(
        &store_path,
        configured,
    )?)?;
    Ok(ConfiguredService {
        service,
        store_path,
        embedder_model,
        embedder_dimension,
    })
}

pub(crate) fn embedder_after_startup_recovery(
    recovery: velesdb_memory::migration::OnlineMigrationStartup,
) -> Result<ConfiguredEmbedder, Box<dyn std::error::Error>> {
    match recovery {
        velesdb_memory::migration::OnlineMigrationStartup::None => build_embedder(),
        velesdb_memory::migration::OnlineMigrationStartup::SourceRestored { source_model } => {
            let configured = build_embedder()?;
            if configured.model != source_model {
                return Err(format!(
                    "online migration restored source model '{source_model}', but startup configured '{}'",
                    configured.model
                )
                .into());
            }
            Ok(configured)
        }
        velesdb_memory::migration::OnlineMigrationStartup::TargetActivated { embedder, model } => {
            Ok(ConfiguredEmbedder { embedder, model })
        }
    }
}

/// A built service plus the resolved configuration `memory_status` reports:
/// which embedder actually runs, and where the store (and its provenance
/// record) lives. Exists because the service deliberately does not know its
/// embedder's name — only this binary does.
pub(crate) struct ConfiguredService {
    service: MemoryService<DynEmbedder>,
    store_path: String,
    embedder_model: String,
    embedder_dimension: usize,
}

/// Attach the extraction backend to the SERVICE when
/// `VELESDB_MEMORY_AUTOGRAPH=1` (`[graph] autograph = true`), so every
/// `remember` also wires the entities, typed edges and attributes its text
/// states.
///
/// Distinct from [`build_server`]'s extractor, which powers the explicit
/// `remember_extracted` tool. Both can be on; they share one backend and one
/// setting pair, and `remember_extracted` deliberately does not re-extract.
///
/// Asking for autograph without an extraction backend is a startup error, not
/// a silent no-op: the operator turned on a feature, and a daemon that
/// answers by doing nothing is how you spend a week wondering why the graph
/// is empty.
pub(crate) fn apply_autograph(
    service: MemoryService<DynEmbedder>,
) -> Result<MemoryService<DynEmbedder>, Box<dyn std::error::Error>> {
    if std::env::var("VELESDB_MEMORY_AUTOGRAPH").as_deref() != Ok("1") {
        return Ok(service);
    }
    let backend = std::env::var("VELESDB_MEMORY_EXTRACTOR").unwrap_or_default();
    // Same gate as `attach_extractor`, and it must accept the same names: a
    // build where `remember_extracted` works with `outline` but autograph
    // refuses to start would be a contradiction the operator cannot resolve.
    match velesdb_memory::select_extractor(&backend)? {
        ExtractorSelection::Disabled => Err(
            "autograph is on ([graph] autograph = true / VELESDB_MEMORY_AUTOGRAPH=1) but no \
             extraction backend is configured — set [extractor] backend = \"outline\" for the \
             offline deterministic reader (no rebuild, no model to run), or \"ollama\" with a \
             model, or turn autograph off"
                .into(),
        ),
        ExtractorSelection::Ready(extractor) => Ok(service.with_autograph(extractor)),
        // Dispatches on the NAME, exactly like `attach_extractor`. This arm
        // used to ignore it and build Ollama unconditionally; leaving it that
        // way would mean autograph silently talked to a different server than
        // `remember_extracted` on the very same daemon — the contradiction the
        // comment above says an operator cannot resolve.
        ExtractorSelection::NeedsRemoteConfig(backend) => {
            let extractor = build_remote_extractor(backend)?;
            warn_if_extraction_backend_is_unreachable(backend);
            Ok(service.with_autograph(extractor))
        }
    }
}

/// Locate and apply the optional `velesdb-memory.toml`.
///
/// The lookup uses the DEFAULT store directory, never the configured one:
/// the store path is itself one of the settings the file may carry, so
/// resolving the file through it would be circular.
///
/// A missing file is normal and silent. A file that exists but does not parse
/// aborts startup — a daemon quietly running on defaults the operator believes
/// they overrode is a worse outcome than a loud failure at boot.
pub(crate) fn apply_config_file(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let explicit = args
        .iter()
        .position(|arg| arg == "--config")
        .and_then(|at| args.get(at + 1))
        .map(String::as_str);
    // The EFFECTIVE store, not the default one. `VELESDB_MEMORY_PATH` moves the
    // store, and the config file lives beside it — looking it up in the default
    // directory instead means a caller who moved the store silently reads a
    // config from a store they are not using. That is how a test spawning this
    // binary with its own scratch store picked up the developer's personal
    // `~/.velesdb-memory/velesdb-memory.toml`.
    let store_dir = std::env::var("VELESDB_MEMORY_PATH").unwrap_or_else(|_| default_store_path());
    let Some(path) =
        velesdb_memory::config::resolve_path(explicit, Some(std::path::Path::new(&store_dir)))
    else {
        return Ok(());
    };
    let loaded = velesdb_memory::config::load(&path)?;
    let applied = velesdb_memory::config::apply(&loaded.values);
    if !applied.is_empty() && std::env::var_os("VELESDB_MEMORY_QUIET").is_none() {
        eprintln!(
            "velesdb-memory: {} setting(s) from {}",
            applied.len(),
            path.display()
        );
    }
    Ok(())
}

pub(crate) fn default_store_path() -> String {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|h| !h.is_empty());
    match home {
        Some(home) => std::path::Path::new(&home)
            .join(".velesdb-memory")
            .to_string_lossy()
            .into_owned(),
        None => "./velesdb-memory-store".to_owned(),
    }
}

/// Apply `VELESDB_MEMORY_DEFAULT_TTL` (seconds) as the default expiry for facts
/// stored without their own `ttl_seconds`. Unset means facts are permanent.
pub(crate) fn apply_default_ttl(
    server: McpServer,
) -> Result<McpServer, Box<dyn std::error::Error>> {
    match std::env::var("VELESDB_MEMORY_DEFAULT_TTL") {
        Ok(raw) => {
            let ttl_seconds: u64 = raw.trim().parse().map_err(|_| {
                format!(
                    "VELESDB_MEMORY_DEFAULT_TTL must be a non-negative integer (seconds), got '{raw}'"
                )
            })?;
            Ok(server.with_default_ttl(ttl_seconds))
        }
        Err(_) => Ok(server),
    }
}

/// Apply `VELESDB_MEMORY_INGEST_ROOTS` (V2b-1) — a platform `PATH`-list of
/// directories a `path`-referenced context fragment may read from — enabling
/// the `compile_context`/`explain_compilation` `path` field. Unset or empty
/// leaves path ingestion disabled (every `path` fragment then fails with an
/// explicit error, not a silent no-op). Parsed here, at startup, so a
/// misconfigured root (missing directory, broken symlink) fails fast instead
/// of surfacing on a caller's first `path` fragment.
#[cfg(feature = "context")]
pub(crate) fn apply_ingest_roots(
    server: McpServer,
) -> Result<McpServer, Box<dyn std::error::Error>> {
    match std::env::var("VELESDB_MEMORY_INGEST_ROOTS") {
        Ok(raw) if !raw.trim().is_empty() => {
            let roots = velesdb_memory::context::IngestRoots::parse(&raw)?;
            Ok(server.with_ingest_roots(roots))
        }
        _ => Ok(server),
    }
}

/// Without the `context` feature there is no `IngestRoots` type (or `path`
/// field) to configure. The `Result` return mirrors the `context` arm's
/// signature so the caller is identical for both builds.
#[cfg(not(feature = "context"))]
#[allow(clippy::unnecessary_wraps)]
pub(crate) fn apply_ingest_roots(
    server: McpServer,
) -> Result<McpServer, Box<dyn std::error::Error>> {
    Ok(server)
}

/// Build the MCP server, attaching the extraction backend named by
/// `VELESDB_MEMORY_EXTRACTOR`.
///
/// This function is now a thin read of the environment on top of
/// [`attach_extractor`]; the choice itself lives in the library so the daemon
/// and the tests run the same code. See [`attach_extractor`] for why that
/// matters here in particular.
pub(crate) fn build_server(
    service: MemoryService<DynEmbedder>,
) -> Result<McpServer, Box<dyn std::error::Error>> {
    let backend = std::env::var("VELESDB_MEMORY_EXTRACTOR").unwrap_or_default();
    attach_extractor(McpServer::new(service), &backend)
}

/// Attach the extraction backend named `backend` to `server`.
///
/// **There is deliberately no `#[cfg(feature = "extractor-http")]` on this function.**
/// That gate used to sit on the whole selection, which is what made
/// `OutlineExtractor` unreachable from the MCP server (#1734): the extractor
/// needs no dependency and is linked into every build, but the only code that
/// could choose it was compiled away unless an unrelated HTTP feature was on.
/// Two of the twenty published tools were dead by default as a result —
/// `remember_extracted` refused outright, and `entity` answered `found: false`
/// for every name, entity hubs being born only of extraction.
///
/// Only the arm that genuinely needs the optional dependency stays gated.
///
/// # Errors
/// An unknown backend name, or a network-backed backend whose required
/// configuration is missing or whose feature was not compiled in.
pub(crate) fn attach_extractor(
    server: McpServer,
    backend: &str,
) -> Result<McpServer, Box<dyn std::error::Error>> {
    match velesdb_memory::select_extractor(backend)? {
        ExtractorSelection::Disabled => Ok(server),
        ExtractorSelection::Ready(extractor) => {
            Ok(server.with_named_extractor(backend, extractor)?)
        }
        // **This is the seam #1751 turns on.** The arm used to read
        // `NeedsRemoteConfig(_)` and call `build_ollama_extractor()`: the
        // library named the backend the operator asked for, and the daemon
        // threw the name away and built the only client it knew. Adding a
        // second protocol to `select_extractor` would have changed nothing
        // observable — the wrong client would have been built, silently, for
        // every name.
        ExtractorSelection::NeedsRemoteConfig(backend) => {
            Ok(server.with_named_extractor(backend, build_remote_extractor(backend)?)?)
        }
    }
}
