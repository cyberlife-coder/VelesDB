//! `velesdb-memory` — MCP memory server binary (stdio transport by default).
//!
//! Serves the memory tools over stdio so any MCP client (Claude Code, Cursor,
//! Cline, Zed, …) can use it locally. The store never leaves the machine.
//! Configure the store directory with `VELESDB_MEMORY_PATH` (default
//! `~/.velesdb-memory`) and the embedding
//! backend with `VELESDB_MEMORY_EMBEDDER` (`hash` | `ollama` | `openai`). Set
//! `VELESDB_MEMORY_EXTRACTOR` to enable the `remember_extracted` tool (text →
//! fact↔topic graph): `outline` reads directives you write explicitly and needs
//! no model and no extra feature, while `ollama` and `openai` infer them with a
//! generative model and need `--features extract`.
//!
//! Each role carries its own `_URL`, `_MODEL` and `_API_TOKEN`, and the two are
//! configured independently — embedding on a local Ollama while extracting on
//! an OpenAI-compatible server is a supported combination. `openai` names a
//! *protocol* (oMLX, llama.cpp, LM Studio, vLLM, hosted providers all speak
//! it), so it has no default URL and no default model: reaching a different
//! server is a different URL, never a new backend name. Tokens are read from
//! the environment only, never from the config file. Set
//! `VELESDB_MEMORY_DEFAULT_TTL` (seconds) to expire remembered facts by default.
//! Set `VELESDB_MEMORY_INGEST_ROOTS` (a `PATH`-list of directories) to let
//! `compile_context`/`explain_compilation` fragments reference a file by
//! `path` instead of inline `content`; unset disables that field entirely.
//! Run with `--version` (or `-V`) to print the binary's version and exit,
//! without opening the store.
//!
//! When built with `--features http`, pass `--http` (or set
//! `VELESDB_MEMORY_HTTP=1`) to serve over the streamable-HTTP transport
//! instead of stdio — letting several MCP clients share ONE process instead
//! of each fighting over the store's single-writer `flock`. See
//! `velesdb_memory::http` and the README's "HTTP transport" section.
//!
//! The HTTP transport serves HTTPS by default, terminated with a locally
//! generated CA + leaf certificate (see `velesdb_memory::tls` — no external
//! `mkcert`/`openssl`/reverse proxy required; some MCP clients, e.g. Claude
//! Desktop's "Add custom connector", refuse any URL that isn't `https://`,
//! even for `127.0.0.1`). Pass `--http-insecure` (or set
//! `VELESDB_MEMORY_HTTP_INSECURE=1`) to fall back to plain HTTP instead —
//! for local debugging, or when a trusted TLS-terminating proxy already
//! sits in front.

use std::time::Duration;

use rmcp::ServiceExt;
use velesdb_memory::mcp::McpServer;
use velesdb_memory::{DynEmbedder, ExtractorSelection, MemoryService, NativeStore};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();

    // Handled before anything else touches the filesystem or the embedder:
    // `--version`/`-V` must work even when the store path is unwritable or
    // absent (e.g. a fresh dev running it once to sanity-check the install),
    // so it short-circuits ahead of the store open below.
    if args
        .get(1)
        .is_some_and(|arg| arg == "--version" || arg == "-V")
    {
        println!("velesdb-memory {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    // Short-circuits for the same reason as `--version`: `compile-stdin` is
    // the hook-facing surface (see `integrations/agent-hooks`). A PostToolUse
    // hook runs SYNCHRONOUSLY after every tool call, in a process of its own,
    // while the agent's own MCP server already holds the store's
    // single-writer `flock` — so this path must never open the store. It
    // doesn't have to: the compiler
    // (`velesdb_memory::context::ContextCompiler::compile`) is pure — no
    // store, no index, no clock — and the `context` feature is
    // `persistence`-free by design.
    if args.get(1).is_some_and(|arg| arg == "compile-stdin") {
        return run_compile_stdin(&args[2..]);
    }

    // Short-circuits for a third reason: this one must NOT open the store even
    // though it inspects one. A diagnosis reads a verified copy and never calls
    // `Database::open` on the live directory, which is what lets an operator run
    // it while their daemon is up (#1762's protocol allows exactly that, read
    // only). Falling through to `build_configured_service` would take the
    // single-writer `flock` and refuse against the very daemon whose store the
    // operator is asking about.
    if args.get(1).is_some_and(|arg| arg == "migrate-embeddings") {
        return run_migrate_embeddings(&args, &args[2..]);
    }

    // Captured FIRST — before the (possibly seconds-long) embedder probe and
    // store open — so a client that exits during our own startup still
    // reparents us AFTER the baseline, and the watchdog sees the change. A
    // baseline taken later would read the already-reparented pid and go
    // permanently inert (review finding on #1449).
    #[cfg(unix)]
    let original_parent = std::os::unix::process::parent_id();
    #[cfg(not(unix))]
    let original_parent = 0_u32;
    // All synchronous setup (config file, env probing, blocking HTTP to
    // Ollama, disk open) happens in here, before the async runtime starts, so
    // we never block a tokio worker thread on a synchronous operation.
    let service = build_configured_service(&args)?;

    // Read AFTER the config file has been applied, since the file can set
    // `VELESDB_MEMORY_HTTP`. Same manual-parsing style as `--version` above
    // (no `clap` for a two-flag CLI) — the transport choice only affects how
    // the server is *served*, further down, since store opening (and its
    // `flock`) is identical either way.
    let http_bind = requested_http_bind(&args);
    let server = apply_ingest_roots(apply_default_ttl(build_server(service)?)?)?;

    tokio::runtime::Runtime::new()?.block_on(async move {
        match http_bind {
            #[cfg(feature = "http")]
            Some(request) => serve_http(server, request).await,
            #[cfg(not(feature = "http"))]
            Some(_never) => unreachable!(
                "requested_http_bind only returns Some when built with --features http"
            ),
            None => {
                // The orphan watchdog only makes sense for stdio: it exists to
                // detect a *client process* dying without closing our stdin
                // (#1448). An HTTP daemon has no such single-client lifecycle
                // to watch — it's meant to outlive any one client — so it is
                // never spawned in HTTP mode.
                spawn_orphan_watchdog(original_parent);
                let running = server
                    .serve((tokio::io::stdin(), tokio::io::stdout()))
                    .await?;
                running.waiting().await?;
                Ok::<(), Box<dyn std::error::Error>>(())
            }
        }
    })
}

/// A resolved `--http`/`VELESDB_MEMORY_HTTP=1` request: where to bind, and
/// whether TLS should be skipped (`--http-insecure` /
/// `VELESDB_MEMORY_HTTP_INSECURE=1` — see [`requested_http_bind`]).
#[cfg(feature = "http")]
struct HttpServeRequest {
    bind_addr: String,
    insecure: bool,
}

/// Detect the streamable-HTTP transport request (`--http` flag or
/// `VELESDB_MEMORY_HTTP=1`) and resolve how it should be served, BEFORE the
/// store is opened. Returns `None` for the default stdio transport.
///
/// Without the `http` feature, `--http`/`VELESDB_MEMORY_HTTP=1` is rejected
/// with an actionable message instead of silently falling back to stdio —
/// the binary was built without the code to honor the request at all.
#[cfg(feature = "http")]
fn requested_http_bind(args: &[String]) -> Option<HttpServeRequest> {
    let http_flag = args.iter().any(|arg| arg == "--http");
    let http_env = std::env::var("VELESDB_MEMORY_HTTP").as_deref() == Ok("1");
    if !http_flag && !http_env {
        return None;
    }

    let port_override = args
        .iter()
        .position(|arg| arg == "--http-port")
        .and_then(|flag_index| args.get(flag_index + 1));

    let default_bind = std::env::var("VELESDB_MEMORY_HTTP_BIND")
        .unwrap_or_else(|_| velesdb_memory::http::DEFAULT_HTTP_BIND.to_owned());

    let bind_addr = match port_override {
        Some(port) => match default_bind.rsplit_once(':') {
            Some((host, _existing_port)) => format!("{host}:{port}"),
            None => format!("127.0.0.1:{port}"),
        },
        None => default_bind,
    };

    // The router (`velesdb_memory::http::router`) authenticates no one: any
    // caller that can reach the socket gets full `remember`/`recall`/`relate`
    // access to the store. That's only safe because the default bind is
    // loopback-only. `VELESDB_MEMORY_HTTP_BIND` lets the *port* be
    // overridden freely, but overriding the *host* to something reachable
    // off-box would turn an unauthenticated local daemon into an
    // unauthenticated network service — so that requires an explicit,
    // separate opt-in rather than falling out of a bind-address typo.
    if !is_loopback_host(&bind_addr)
        && std::env::var("VELESDB_MEMORY_HTTP_ALLOW_REMOTE").as_deref() != Ok("1")
    {
        eprintln!(
            "[velesdb-memory] refusing to bind the HTTP transport to '{bind_addr}': it is not a \
             loopback address, and the streamable-HTTP transport has no authentication — anyone \
             who can reach that socket gets full read/write access to the store. Set \
             VELESDB_MEMORY_HTTP_ALLOW_REMOTE=1 to override (put an authenticating reverse proxy \
             in front first)."
        );
        std::process::exit(1);
    }

    // `--http-insecure` / `VELESDB_MEMORY_HTTP_INSECURE=1` is the explicit
    // opt-out of HTTPS-by-default (see the crate-level doc comment above and
    // `velesdb_memory::tls`'s module docs for why HTTPS is the default at
    // all) — an "insecure escape hatch, loud at startup" flag, same shape as
    // `VELESDB_MEMORY_HTTP_ALLOW_REMOTE` above. Kept as its own flag rather
    // than folded into that one: that one is about *who* can reach the
    // socket, this one is about *whether the bytes on the wire are
    // encrypted* — independent axes, and conflating them would make an
    // operator who only wants one silently get the other too.
    let insecure_flag = args.iter().any(|arg| arg == "--http-insecure");
    let insecure_env = std::env::var("VELESDB_MEMORY_HTTP_INSECURE").as_deref() == Ok("1");
    let insecure = insecure_flag || insecure_env;

    Some(HttpServeRequest {
        bind_addr,
        insecure,
    })
}

/// Whether `bind_addr`'s host component (`host:port` or `[ipv6]:port`)
/// resolves to a loopback address. Used to gate non-local HTTP binds behind
/// an explicit opt-in — see `requested_http_bind` above. An unparseable host
/// (e.g. a hostname like `mcp.example.com` rather than a literal IP) is
/// treated as non-loopback: `TcpListener::bind` does its own DNS resolution
/// later, so this is a conservative pre-check, not the only one.
#[cfg(feature = "http")]
fn is_loopback_host(bind_addr: &str) -> bool {
    let host = bind_addr
        .rsplit_once(':')
        .map_or(bind_addr, |(host, _port)| host)
        .trim_start_matches('[')
        .trim_end_matches(']');
    host.parse::<std::net::IpAddr>()
        .is_ok_and(|ip| ip.is_loopback())
}

/// See the `http`-feature variant above. Without `http`, no bind address can
/// ever be resolved — the binary has no HTTP transport built in — so a
/// `--http`/`VELESDB_MEMORY_HTTP=1` request fails fast with guidance instead
/// of being silently ignored (which would otherwise look like the server
/// just hung, or served the wrong transport).
#[cfg(not(feature = "http"))]
fn requested_http_bind(args: &[String]) -> Option<String> {
    let http_flag = args.iter().any(|arg| arg == "--http");
    let http_env = std::env::var("VELESDB_MEMORY_HTTP").as_deref() == Ok("1");
    if http_flag || http_env {
        eprintln!(
            "[velesdb-memory] --http / VELESDB_MEMORY_HTTP=1 requires a binary built with \
             `--features http` (e.g. `cargo install velesdb-memory --features http`) — \
             this binary was built without it"
        );
        std::process::exit(1);
    }
    None
}

/// Serve the MCP server over the streamable-HTTP transport (multi-client
/// mode): binds `request.bind_addr`, mounts [`velesdb_memory::http::router`],
/// and runs until either the process receives Ctrl-C or the returned future
/// is dropped (e.g. process termination) — a background daemon (launchd,
/// systemd) is expected to just kill the process on stop, which is safe: the
/// store's `flock` is released by the kernel on exit regardless (see the
/// orphan-watchdog docs above).
///
/// HTTPS by default (a locally-generated CA + leaf cert — see
/// `velesdb_memory::tls`), unless `request.insecure` opts out
/// (`--http-insecure` / `VELESDB_MEMORY_HTTP_INSECURE=1`), in which case
/// this serves plain HTTP via `axum::serve` exactly as before HTTPS
/// support was added.
#[cfg(feature = "http")]
async fn serve_http(
    server: McpServer,
    request: HttpServeRequest,
) -> Result<(), Box<dyn std::error::Error>> {
    let HttpServeRequest {
        bind_addr,
        insecure,
    } = request;

    let ct = tokio_util::sync::CancellationToken::new();
    let app = velesdb_memory::http::router(server, ct.child_token());
    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;

    spawn_shutdown_signals(ct.clone());

    if insecure {
        eprintln!(
            "[velesdb-memory] WARNING: --http-insecure / VELESDB_MEMORY_HTTP_INSECURE=1 is set — \
             serving PLAIN HTTP (no TLS) on http://{bind_addr}/mcp. Every request is readable by \
             anyone who can reach that socket (loopback-only by default — see \
             VELESDB_MEMORY_HTTP_ALLOW_REMOTE above). Use this only for local debugging, or when \
             a trusted TLS-terminating proxy already sits in front."
        );
        eprintln!("[velesdb-memory] HTTP server listening on http://{bind_addr}/mcp");
        axum::serve(listener, app)
            .with_graceful_shutdown(async move { ct.cancelled_owned().await })
            .await?;
        return Ok(());
    }

    let tls_dir = velesdb_memory::tls::tls_dir_from_env();
    let material = velesdb_memory::tls::ensure_tls_material(&tls_dir)?;
    let acceptor = velesdb_memory::tls::tls_acceptor_from_material(&material)?;
    eprintln!("[velesdb-memory] HTTPS server listening on https://{bind_addr}/mcp");
    eprintln!(
        "[velesdb-memory] Local CA: {} — a client only needs to trust this once (see \
         ./scripts/install-memory-daemon.sh, which does this automatically on macOS); every \
         future leaf certificate this daemon issues is signed by the same CA and is trusted \
         automatically after that.",
        material.ca_cert_path.display()
    );

    velesdb_memory::http::serve_tls(app, listener, acceptor, ct).await;
    Ok(())
}

/// How often the orphan watchdog re-checks its parent pid. The MCP stdio
/// transport only observes disconnects via stdin EOF, which a client that
/// leaks its child process (the #1448 scenario) never delivers — so this is
/// the *only* signal that would otherwise catch that leak. 2s keeps the
/// worst-case self-exit latency low (a handful of polls) without burning
/// meaningful CPU on an idle server.
#[cfg(unix)]
const ORPHAN_CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

/// Detect a dead parent and self-exit, releasing the store's `flock` even
/// when stdin is artificially kept open (a leaked child process, #1448).
///
/// A normal MCP stdio client closes the child's stdin on disconnect, which
/// the existing EOF path already handles. But a client that merely forgets
/// to reap/close its child (observed in practice: a headless `claude -p`
/// run left its server running) never closes that pipe — the server then
/// legitimately keeps serving forever, holding the single-writer store lock
/// and making every later session fail `Storage(DatabaseLocked)`.
///
/// This has no other shutdown trigger to lean on, so it polls: capture the
/// parent pid at startup, and if it ever changes, the parent is gone (Unix
/// re-parents orphans to init/launchd, pid 1 or the user's launchd pid —
/// never the original parent), so exit. `std::os::unix::process::parent_id`
/// is pure `std`, avoiding a new dependency (e.g. `libc::getppid`) for a
/// single syscall.
///
/// Process exit (even via `std::process::exit`, which skips destructors)
/// still releases the store's `flock`: that lock is a kernel-held resource
/// tied to the process's open file descriptors, which the kernel closes —
/// and therefore unlocks — unconditionally on process exit, confirmed by
/// the investigation on #1448 ("released by the kernel even on SIGKILL").
#[cfg(unix)]
fn spawn_orphan_watchdog(original_parent: u32) {
    use std::os::unix::process::parent_id;

    tokio::spawn(async move {
        loop {
            tokio::time::sleep(ORPHAN_CHECK_INTERVAL).await;
            let current_parent = parent_id();
            if current_parent != original_parent {
                eprintln!(
                    "[velesdb-memory] parent process (pid {original_parent}) is gone \
                     (now reparented under pid {current_parent}) — exiting to release \
                     the store lock rather than leak a zombie session (#1448)"
                );
                std::process::exit(0);
            }
        }
    });
}

/// Windows has no equivalent of `parent_id()` re-parenting to detect a dead
/// parent this cheaply, so this hardening is Unix-only for now — behavior on
/// Windows is unchanged (still relies on the stdin-EOF path).
#[cfg(not(unix))]
fn spawn_orphan_watchdog(_original_parent: u32) {}

/// Attempts before giving up on a locked store and printing the actionable
/// error. Three short tries (with [`LOCK_RETRY_DELAY`] between them) is
/// enough to ride out the handover between one session's process exiting
/// and the next one starting — the case the retry is *for* — without making
/// a genuinely-stuck lock (the leaked-process scenario from #1448) hang
/// startup for long.
const LOCK_RETRY_ATTEMPTS: u32 = 3;

/// Delay between retries of an already-locked store. See
/// [`LOCK_RETRY_ATTEMPTS`] for the reasoning on the total budget.
const LOCK_RETRY_DELAY: Duration = Duration::from_millis(500);

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
fn open_store_with_actionable_lock_error(
    store_path: &str,
    configured: ConfiguredEmbedder,
) -> Result<MemoryService<DynEmbedder>, Box<dyn std::error::Error>> {
    use velesdb_memory::MemoryError;

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

    let mut last_locked_path: Option<String> = None;
    for attempt in 0..LOCK_RETRY_ATTEMPTS {
        match NativeStore::open(store_path, dimension) {
            Ok(store) => {
                if unrecorded {
                    record_embedding_model(store_dir, &store, &model, dimension);
                }
                return Ok(MemoryService::with_store(store, embedder));
            }
            Err(MemoryError::Storage(velesdb_core::Error::DatabaseLocked(locked_path))) => {
                last_locked_path = Some(locked_path);
                if attempt + 1 < LOCK_RETRY_ATTEMPTS {
                    std::thread::sleep(LOCK_RETRY_DELAY);
                }
            }
            // A dimension mismatch surfaces here, from the core. When the store
            // carries no model record, that dimension was the ONLY thing that
            // could be compared, and saying so is what stops a caller reading
            // the core's message as a full compatibility verdict.
            Err(other) if unrecorded => {
                return Err(format!(
                    "{other}\n{}",
                    velesdb_memory::embedding_provenance::unrecorded_model_note(&model)
                )
                .into())
            }
            Err(other) => return Err(other.into()),
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
fn record_embedding_model(
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
fn build_configured_service(
    args: &[String],
) -> Result<MemoryService<DynEmbedder>, Box<dyn std::error::Error>> {
    apply_config_file(args)?;
    let store_path = std::env::var("VELESDB_MEMORY_PATH").unwrap_or_else(|_| default_store_path());
    let configured = build_embedder()?;
    apply_autograph(open_store_with_actionable_lock_error(
        &store_path,
        configured,
    )?)
}

/// Run `migrate-embeddings`: diagnose a store against the configured target
/// embedder and print the regime it resolves to.
///
/// `argv` is the whole command line, so `--config` keeps working here exactly
/// as it does for the daemon; `flags` is what follows the subcommand.
///
/// Exits `2` on a refusal rather than returning `Ok`. A command that printed
/// `REFUSE` and exited `0` would be read as success by every script wrapping
/// it, and the whole point of the refusal is that something must not proceed.
///
/// # Errors
/// An unparsable invocation, a non-dry-run request (not built yet — see
/// #1762), an unreachable embedder, or a store that cannot be read or copied.
fn run_migrate_embeddings(
    argv: &[String],
    flags: &[String],
) -> Result<(), Box<dyn std::error::Error>> {
    use velesdb_memory::migration;

    let options = migration::parse_migrate_args(flags)?;

    apply_config_file(argv)?;
    let store_path = migrate_store_path(&options);
    // The target's identity comes from the embedder the daemon WOULD build, not
    // from a flag: a model name an operator typed is a claim, and the dimension
    // has to be the one the embedder actually produces.
    let ConfiguredEmbedder { embedder, model } = build_embedder()?;
    let target = migration::TargetContract {
        model,
        dimension: embedder.dimension(),
        strategy: options.strategy,
    };
    let scratch = migrate_scratch_parent(&options, &store_path)?;

    if options.dry_run {
        let report = migration::dry_run(
            &store_path,
            &scratch,
            &target,
            options.destination.as_deref(),
        )?;
        print!("{}", migration::render(&report));
        if migration::refuses(&report) {
            std::process::exit(2);
        }
        return Ok(());
    }
    run_migrate_rebuild(&options, &store_path, &scratch, &target, embedder.as_ref())
}

/// The non-dry-run tail of `migrate-embeddings`: rebuild, validate, switch.
///
/// The chain enters wherever the journal stands — a re-run after any crash
/// resumes rather than failing on the stage the crash already completed —
/// and each stage that ran reports itself; one that was journalled as done
/// stays silent rather than misreporting work this run did not do.
fn run_migrate_rebuild(
    options: &velesdb_memory::migration::MigrateOptions,
    store_path: &std::path::Path,
    scratch: &std::path::Path,
    target: &velesdb_memory::migration::TargetContract,
    embedder: &dyn velesdb_memory::Embedder,
) -> Result<(), Box<dyn std::error::Error>> {
    use velesdb_memory::migration;

    let destination = migration::require_destination(options)?;
    let outcome = migration::migrate(
        store_path,
        scratch,
        target,
        &destination,
        embedder,
        MIGRATE_BATCH,
    )?;
    if let Some(executed) = &outcome.executed {
        print!("{}", migration::render(&executed.report));
        println!(
            "rebuild: {} facts written, {} already present, {} edges, journal at {}",
            executed.rebuild.facts,
            executed.rebuild.collisions,
            executed.rebuild.edges,
            executed.workspace.display(),
        );
    }
    if let Some(validated) = &outcome.validated {
        println!(
            "validated: {} facts and {} edges compared, {} divergence(s) explained by expiry",
            validated.facts, validated.edges, validated.explained_by_expiry,
        );
    }
    println!("activated: {}", outcome.switched.activated.display());
    println!("{}", migration::migration_complete_notice());
    Ok(())
}

/// The rebuild's batch size: the fact export's own proven default.
const MIGRATE_BATCH: usize = 1024;

/// The store `migrate-embeddings` operates on: `--store`, else exactly where
/// the daemon would look (`VELESDB_MEMORY_PATH`, else the advertised default).
fn migrate_store_path(options: &velesdb_memory::migration::MigrateOptions) -> std::path::PathBuf {
    options.store.clone().unwrap_or_else(|| {
        std::path::PathBuf::from(
            std::env::var("VELESDB_MEMORY_PATH").unwrap_or_else(|_| default_store_path()),
        )
    })
}

/// Where the diagnosis stages its verified copy: `--scratch`, else beside the
/// store — see `migration::default_scratch_parent` for why not the temp dir.
///
/// # Errors
/// The store path has no usable parent and `--scratch` was not given.
fn migrate_scratch_parent(
    options: &velesdb_memory::migration::MigrateOptions,
    store_path: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    if let Some(dir) = options.scratch.clone() {
        return Ok(dir);
    }
    // Canonicalize first so a relative store path ("./store") yields a real
    // parent rather than the empty string. A store that does not exist falls
    // through unchanged — the diagnosis then fails on it with its own, more
    // precise message.
    let resolved = std::fs::canonicalize(store_path).unwrap_or_else(|_| store_path.to_path_buf());
    velesdb_memory::migration::default_scratch_parent(&resolved)
}

/// An embedder together with the identifier of the model behind it.
///
/// The model travels with the embedder because only the code that *built* it
/// knows its name: the [`velesdb_memory::Embedder`] trait exposes a dimension
/// and nothing else, deliberately — a trait implemented by callers should not
/// have to answer questions about a configuration it may not have. Carrying
/// the name here keeps [`velesdb_memory::embedding_provenance`] usable without
/// widening that trait for every implementor, in this crate and out of it.
struct ConfiguredEmbedder {
    embedder: DynEmbedder,
    /// As configured: `bge-m3`, `all-minilm`, or `hash` for the built-in.
    model: String,
}

/// Cancel `ct` on the signals a supervisor actually sends.
///
/// SIGINT alone is not enough. `launchctl kickstart -k`, `systemctl restart`
/// and `docker stop` all send **SIGTERM**, and an unhandled SIGTERM kills the
/// process outright: the streamable-HTTP sessions clients hold are dropped
/// mid-flight, so the next call on a live session hangs until the client's own
/// timeout instead of reconnecting. Handling it lets `with_graceful_shutdown`
/// close those sessions, which is what turns a restart into a reconnect.
#[cfg(feature = "http")]
fn spawn_shutdown_signals(ct: tokio_util::sync::CancellationToken) {
    let interrupt = ct.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            interrupt.cancel();
        }
    });

    #[cfg(unix)]
    tokio::spawn(async move {
        // `signal()` only fails if the handler cannot be registered at all; a
        // daemon that cannot listen for SIGTERM still serves, it just loses the
        // graceful path, so this is not worth aborting startup for.
        if let Ok(mut term) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            term.recv().await;
            ct.cancel();
        }
    });
    // On Windows there is no SIGTERM; Ctrl-C above is the whole contract.
    #[cfg(not(unix))]
    drop(ct);
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
fn apply_autograph(
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

/// Without the `extract` feature there is no remote extraction backend in this
/// build, so there is nothing to be unreachable and no transport to ask with.
///
/// A no-op rather than a `cfg` at the call site, matching how
/// `build_remote_extractor` is paired a few lines below: the one arm that
/// reaches both is easier to read with the condition next to the reason than
/// wrapped around the code that uses it.
#[cfg(not(feature = "extract"))]
fn warn_if_extraction_backend_is_unreachable(_backend: &str) {}

/// How long startup may spend asking whether the extraction backend is there.
///
/// Short on purpose: this runs before the daemon serves anything, and the
/// answer is worth having only if getting it costs nothing. A stalled server
/// is itself an answer, delivered by this timeout.
#[cfg(feature = "extract")]
const EXTRACTION_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

/// Say once, at startup, when the configured extraction backend cannot be
/// reached (#1751, decision D2).
///
/// A **signal, never a refusal.** Autograph degrading in flight is the correct
/// default — losing the enrichment beats losing the fact — and the arbitration
/// on #1751 forbids turning a successful `remember` into an error. What was
/// wrong is that the degradation was silent *forever*: an extractor broken by
/// a migration looked exactly like a product that does not build a graph.
/// Unreachable is also transient, so refusing to boot over it would replace a
/// silence with an outage.
///
/// Never falls back to another backend. A daemon that quietly answered with a
/// different engine than the one configured is the defect #1751's own gate
/// comment calls a contradiction the operator cannot resolve.
#[cfg(feature = "extract")]
fn warn_if_extraction_backend_is_unreachable(backend: &str) {
    use velesdb_memory::reachability::{probe_openai, warning_line, Reachability};

    // `outline` is offline and deterministic: there is nothing to reach, and a
    // probe would invent a failure mode it does not have.
    if std::env::var_os("VELESDB_MEMORY_QUIET").is_some() {
        return;
    }
    let Some((url, model)) = extraction_endpoint_for_probe(backend) else {
        return;
    };
    let token = env_opt("VELESDB_MEMORY_EXTRACTOR_API_TOKEN");
    let outcome = probe_openai(&url, &model, token.as_deref(), EXTRACTION_PROBE_TIMEOUT);
    if outcome == Reachability::Reachable {
        return;
    }
    if let Some(line) = warning_line("extraction", &url, &model, &outcome) {
        eprintln!("{line}");
    }
}

/// The URL and model the extraction role will actually talk to, or `None` when
/// there is nothing to probe.
///
/// Resolves the same way the builders do — including Ollama's canonical local
/// default — because a probe of a different address than the one that will be
/// used answers a question nobody asked.
#[cfg(feature = "extract")]
fn extraction_endpoint_for_probe(backend: &str) -> Option<(String, String)> {
    let endpoint = extractor_endpoint().ok()?;
    let url = match backend {
        "ollama" => Some(
            endpoint
                .url
                .unwrap_or_else(|| velesdb_memory::extract::DEFAULT_OLLAMA_URL.to_owned()),
        ),
        "openai" => endpoint.url,
        // An unwired name: `build_remote_extractor` has already refused it.
        _ => None,
    }?;
    Some((url, endpoint.model?))
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
fn apply_config_file(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
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

fn default_store_path() -> String {
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
fn apply_default_ttl(server: McpServer) -> Result<McpServer, Box<dyn std::error::Error>> {
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
fn apply_ingest_roots(server: McpServer) -> Result<McpServer, Box<dyn std::error::Error>> {
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
fn apply_ingest_roots(server: McpServer) -> Result<McpServer, Box<dyn std::error::Error>> {
    Ok(server)
}

/// Build the MCP server, attaching the extraction backend named by
/// `VELESDB_MEMORY_EXTRACTOR`.
///
/// This function is now a thin read of the environment on top of
/// [`attach_extractor`]; the choice itself lives in the library so the daemon
/// and the tests run the same code. See [`attach_extractor`] for why that
/// matters here in particular.
fn build_server(
    service: MemoryService<DynEmbedder>,
) -> Result<McpServer, Box<dyn std::error::Error>> {
    let backend = std::env::var("VELESDB_MEMORY_EXTRACTOR").unwrap_or_default();
    attach_extractor(McpServer::new(service), &backend)
}

/// Attach the extraction backend named `backend` to `server`.
///
/// **There is deliberately no `#[cfg(feature = "extract")]` on this function.**
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
fn attach_extractor(
    server: McpServer,
    backend: &str,
) -> Result<McpServer, Box<dyn std::error::Error>> {
    match velesdb_memory::select_extractor(backend)? {
        ExtractorSelection::Disabled => Ok(server),
        ExtractorSelection::Ready(extractor) => Ok(server.with_extractor(extractor)),
        // **This is the seam #1751 turns on.** The arm used to read
        // `NeedsRemoteConfig(_)` and call `build_ollama_extractor()`: the
        // library named the backend the operator asked for, and the daemon
        // threw the name away and built the only client it knew. Adding a
        // second protocol to `select_extractor` would have changed nothing
        // observable — the wrong client would have been built, silently, for
        // every name.
        ExtractorSelection::NeedsRemoteConfig(backend) => {
            Ok(server.with_extractor(build_remote_extractor(backend)?))
        }
    }
}

/// Build the remote extraction backend named `backend`.
///
/// The `other` arm is not decoration. `select_extractor` is the single place
/// that knows which names exist, and it lives in the library while this
/// dispatch lives in the binary — so the two CAN drift. When they do, the
/// operator gets a message naming the gap instead of an Ollama client quietly
/// pointed at a server that speaks something else.
#[cfg(feature = "extract")]
fn build_remote_extractor(
    backend: &str,
) -> Result<velesdb_memory::DynExtractor, Box<dyn std::error::Error>> {
    match backend {
        "ollama" => build_ollama_extractor(),
        "openai" => build_openai_extractor(),
        other => Err(unwired_backend("extraction", other).into()),
    }
}

/// Without the `extract` feature there is no HTTP backend to build, whichever
/// one was asked for. The error names the offline alternative rather than only
/// what is missing: since #1734, `outline` is a real answer in **every** build,
/// so a user who only wanted a graph is one setting away instead of one
/// rebuild away.
#[cfg(not(feature = "extract"))]
fn build_remote_extractor(
    backend: &str,
) -> Result<velesdb_memory::DynExtractor, Box<dyn std::error::Error>> {
    Err(format!(
        "VELESDB_MEMORY_EXTRACTOR={backend} needs a build with `--features extract`; \
         for an offline deterministic graph with no rebuild, set \
         VELESDB_MEMORY_EXTRACTOR=outline instead"
    )
    .into())
}

// --- Where a remote backend's URL, model and credential come from -----------
//
// One shape for both roles, on purpose. The two were configured differently
// for historical reasons only — extraction by role, embedding by product —
// and an operator who has configured one should not have to learn the other
// (#1751, arbitration C1).

/// A remote backend's configuration, read from one role's environment.
#[cfg(any(feature = "ollama", feature = "extract"))]
struct RemoteEndpoint {
    /// Server origin and port, no path. `None` when unset.
    url: Option<String>,
    /// Model identifier the server expects. `None` when unset.
    model: Option<String>,
    /// The credential, already resolved to what the transport puts on the wire.
    auth: velesdb_memory::Auth,
}

#[cfg(any(feature = "ollama", feature = "extract"))]
impl RemoteEndpoint {
    /// The URL and model, both **required** — the `openai` shape.
    ///
    /// Neither has a default, and that is the design rather than an omission:
    /// `openai` names a *protocol*, spoken by oMLX, llama.cpp, LM Studio, vLLM
    /// and a dozen hosted providers. Guessing a URL would pick one of them for
    /// the operator, and guessing a model would send a name no server on that
    /// list is obliged to know. Ollama keeps its defaults because it genuinely
    /// has one canonical local address.
    ///
    /// # Errors
    /// A message naming the exact variable that is missing, per role.
    fn require(self, prefix: &str) -> Result<(String, String, velesdb_memory::Auth), String> {
        let url = self.url.ok_or_else(|| {
            format!(
                "{prefix}=openai requires {prefix}_URL — the server's origin and port, \
                 no path (e.g. http://localhost:8020). There is no default: `openai` \
                 is a protocol, and only you know which server speaks it here."
            )
        })?;
        let model = self.model.ok_or_else(|| {
            format!(
                "{prefix}=openai requires {prefix}_MODEL — the model identifier the server expects"
            )
        })?;
        Ok((url, model, self.auth))
    }
}

/// The embedding role's endpoint, honouring the legacy
/// `VELESDB_MEMORY_OLLAMA_*` aliases (C1).
///
/// # Errors
/// An `_API_TOKEN` that is set but empty.
#[cfg(feature = "ollama")]
fn embedder_endpoint() -> Result<RemoteEndpoint, Box<dyn std::error::Error>> {
    use velesdb_memory::config::{alias_conflict_notice, resolve_alias};

    let url = resolve_alias(
        env_opt("VELESDB_MEMORY_EMBEDDER_URL").as_deref(),
        env_opt("VELESDB_MEMORY_OLLAMA_URL").as_deref(),
    );
    let model = resolve_alias(
        env_opt("VELESDB_MEMORY_EMBEDDER_MODEL").as_deref(),
        env_opt("VELESDB_MEMORY_OLLAMA_MODEL").as_deref(),
    );
    let mut conflicts = Vec::new();
    if url.conflicting {
        conflicts.push(("VELESDB_MEMORY_EMBEDDER_URL", "VELESDB_MEMORY_OLLAMA_URL"));
    }
    if model.conflicting {
        conflicts.push((
            "VELESDB_MEMORY_EMBEDDER_MODEL",
            "VELESDB_MEMORY_OLLAMA_MODEL",
        ));
    }
    // Once, at startup, and only when the two genuinely disagree. Called from
    // `build_remote_embedder`, which runs exactly once per process.
    if let Some(notice) = alias_conflict_notice(&conflicts) {
        if std::env::var_os("VELESDB_MEMORY_QUIET").is_none() {
            eprintln!("{notice}");
        }
    }
    Ok(RemoteEndpoint {
        url: url.value,
        model: model.value,
        auth: role_auth("VELESDB_MEMORY_EMBEDDER_API_TOKEN")?,
    })
}

/// The extraction role's endpoint. No aliases to resolve: these variables were
/// role-named from the start, which is the naming the embedding side is being
/// brought in line with.
///
/// # Errors
/// An `_API_TOKEN` that is set but empty.
#[cfg(feature = "extract")]
fn extractor_endpoint() -> Result<RemoteEndpoint, Box<dyn std::error::Error>> {
    Ok(RemoteEndpoint {
        url: env_opt("VELESDB_MEMORY_EXTRACTOR_URL"),
        model: env_opt("VELESDB_MEMORY_EXTRACTOR_MODEL"),
        auth: role_auth("VELESDB_MEMORY_EXTRACTOR_API_TOKEN")?,
    })
}

/// A variable's value, or `None` when it is unset.
#[cfg(any(feature = "ollama", feature = "extract"))]
fn env_opt(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

/// Read a role's API token and turn it into what the transport will send.
///
/// The token lives in the environment and **nowhere else** — never in the TOML
/// (arbitration B1, enforced by `velesdb_memory::config`'s
/// `deny_unknown_fields` and its redacted refusal).
///
/// Note for the Ollama backends: they take no credential, so a token set
/// alongside `backend = "ollama"` is read here and then goes unused. That is
/// harmless — Ollama authenticates nothing — and refusing it would reject a
/// configuration that works.
///
/// # Errors
/// A variable that is set to an empty or blank value. That is not the same as
/// unset: unset means "send no credential", while empty is a caller whose
/// shell expansion produced nothing, and silently sending no credential would
/// surface as a `401` they cannot explain.
#[cfg(any(feature = "ollama", feature = "extract"))]
fn role_auth(name: &str) -> Result<velesdb_memory::Auth, Box<dyn std::error::Error>> {
    match env_opt(name) {
        None => Ok(velesdb_memory::Auth::None),
        Some(token) if token.trim().is_empty() => Err(format!(
            "{name} is set but empty — unset it entirely to send no credential. An \
             empty token would go out as `Authorization: Bearer `, which a server \
             rejects as a bad credential rather than a missing one."
        )
        .into()),
        Some(token) => Ok(velesdb_memory::Auth::Bearer(token)),
    }
}

/// A backend name the library accepts but this binary has no builder for.
///
/// Reachable only if `select_*` gains a name and the dispatch below is not
/// updated with it — the exact drift a wildcard arm used to hide.
#[cfg(any(feature = "ollama", feature = "extract"))]
fn unwired_backend(role: &str, backend: &str) -> String {
    format!(
        "the {role} backend '{backend}' is accepted by velesdb-memory's selector but \
         the daemon has no builder wired for it — this is a bug in velesdb-memory, \
         not a configuration error; please report it quoting this message"
    )
}

/// Build the Ollama-backed extractor from `VELESDB_MEMORY_EXTRACTOR_URL`
/// (default local) and the required `VELESDB_MEMORY_EXTRACTOR_MODEL`.
#[cfg(feature = "extract")]
fn build_ollama_extractor() -> Result<velesdb_memory::DynExtractor, Box<dyn std::error::Error>> {
    use std::sync::Arc;
    use velesdb_memory::extract::DEFAULT_OLLAMA_URL;
    use velesdb_memory::OllamaExtractor;

    let endpoint = extractor_endpoint()?;
    // A default URL is right HERE and wrong for `openai`: Ollama has one
    // canonical local address, an OpenAI-compatible server has none.
    let url = endpoint
        .url
        .unwrap_or_else(|| DEFAULT_OLLAMA_URL.to_owned());
    let model = endpoint.model.ok_or(
        "VELESDB_MEMORY_EXTRACTOR=ollama requires VELESDB_MEMORY_EXTRACTOR_MODEL \
         (e.g. qwen3.6:35b-mlx)",
    )?;
    Ok(Arc::new(OllamaExtractor::new(url, model)))
}

/// Build the OpenAI-compatible extractor from the extraction role's own
/// `VELESDB_MEMORY_EXTRACTOR_URL`, `_MODEL` and optional `_API_TOKEN`.
#[cfg(feature = "extract")]
fn build_openai_extractor() -> Result<velesdb_memory::DynExtractor, Box<dyn std::error::Error>> {
    use std::sync::Arc;
    use velesdb_memory::OpenAiExtractor;

    let (url, model, auth) = extractor_endpoint()?.require("VELESDB_MEMORY_EXTRACTOR")?;
    Ok(Arc::new(OpenAiExtractor::new(url, model, auth)))
}

/// Select the embedding backend from `VELESDB_MEMORY_EMBEDDER`: `hash`
/// (default) is deterministic and fully offline; `ollama` gives real on-device
/// semantic recall and requires building with `--features ollama`.
///
/// A thin read of the environment on top of
/// [`velesdb_memory::select_embedder`], mirroring what [`build_server`] does
/// for the extraction side. The choice itself lives in the library so the
/// daemon and the tests exercise the same function instead of a seam written
/// for the test.
///
/// `.ok()` maps an unset variable to `None`, which the library reads as "no
/// preference". A variable that IS set keeps its value — including an empty
/// one, which stays a caller error rather than collapsing into the default.
fn build_embedder() -> Result<ConfiguredEmbedder, Box<dyn std::error::Error>> {
    let backend = std::env::var("VELESDB_MEMORY_EMBEDDER");
    // The library message is transport-neutral; the daemon adds the name of the
    // thing the reader actually has to edit.
    let selection = velesdb_memory::select_embedder(backend.as_deref().ok())
        .map_err(|err| format!("VELESDB_MEMORY_EMBEDDER: {err}"))?;
    match selection {
        // Matched by name rather than by "ready implies hash": the startup
        // notice belongs to this one backend, and a future offline backend
        // must not inherit it silently.
        velesdb_memory::EmbedderSelection::Ready("hash", embedder) => {
            warn_hash_embedder_not_semantic();
            Ok(ConfiguredEmbedder {
                embedder,
                model: "hash".to_owned(),
            })
        }
        // For a backend that needs no configuration, the backend name IS the
        // model: there is no separate identifier to carry.
        velesdb_memory::EmbedderSelection::Ready(name, embedder) => Ok(ConfiguredEmbedder {
            embedder,
            model: name.to_owned(),
        }),
        // The name, not a wildcard — see `attach_extractor` for the defect
        // this shape removes on the extraction side. The embedding side never
        // had a second backend to get wrong, which is exactly why it was the
        // easier of the two to leave broken.
        velesdb_memory::EmbedderSelection::NeedsRemoteConfig(backend) => {
            build_remote_embedder(backend)
        }
    }
}

/// Build the remote embedding backend named `backend`.
///
/// Mirrors [`build_remote_extractor`], down to the `other` arm: the selector
/// lives in the library and this dispatch lives in the binary, so a name added
/// to one and not the other must say so rather than fall back to whichever
/// client happens to be first.
#[cfg(feature = "ollama")]
fn build_remote_embedder(backend: &str) -> Result<ConfiguredEmbedder, Box<dyn std::error::Error>> {
    match backend {
        "ollama" => build_ollama_embedder(),
        "openai" => build_openai_embedder(),
        other => Err(unwired_backend("embedding", other).into()),
    }
}

/// Without the `ollama` feature this crate has no HTTP embedding backend at
/// all, whichever one was asked for. The feature's name predates the protocol
/// split and now under-describes what it carries — it is this crate's HTTP
/// dependency for the embedding role, not a vendor.
#[cfg(not(feature = "ollama"))]
fn build_remote_embedder(backend: &str) -> Result<ConfiguredEmbedder, Box<dyn std::error::Error>> {
    Err(format!(
        "the '{backend}' embedder requires building with `--features ollama` \
         (that feature carries the HTTP dependency for both remote embedding \
         backends); VELESDB_MEMORY_EMBEDDER=hash needs no rebuild"
    )
    .into())
}

/// Warn (on **stderr**, never stdout — that carries the MCP JSON-RPC stream)
/// that the default `hash` embedder is deterministic but **not semantic**:
/// `recall` matches on lexical/hash proximity, not meaning. This is the single
/// most common "why is recall bad?" surprise, so make the trade-off explicit
/// and point to the opt-in. Silence it for scripted/offline runs with
/// `VELESDB_MEMORY_QUIET=1`.
fn warn_hash_embedder_not_semantic() {
    if std::env::var_os("VELESDB_MEMORY_QUIET").is_some() {
        return;
    }
    eprintln!(
        "[velesdb-memory] Using the default 'hash' embedder: deterministic and \
         fully offline, but NOT semantic — recall matches surface form, not meaning. \
         For real semantic recall, run an Ollama build with \
         VELESDB_MEMORY_EMBEDDER=ollama (see crates/velesdb-memory/README.md). \
         Set VELESDB_MEMORY_QUIET=1 to silence this notice."
    );
}

/// Build the Ollama-backed embedder, defaulting both the URL and the model —
/// unchanged behaviour, now reached through the role-named variables with the
/// `VELESDB_MEMORY_OLLAMA_*` pair kept working as aliases.
#[cfg(feature = "ollama")]
fn build_ollama_embedder() -> Result<ConfiguredEmbedder, Box<dyn std::error::Error>> {
    use velesdb_memory::{OllamaEmbedder, DEFAULT_OLLAMA_MODEL, DEFAULT_OLLAMA_URL};

    let endpoint = embedder_endpoint()?;
    let url = endpoint
        .url
        .unwrap_or_else(|| DEFAULT_OLLAMA_URL.to_owned());
    let model = endpoint
        .model
        .unwrap_or_else(|| DEFAULT_OLLAMA_MODEL.to_owned());
    Ok(ConfiguredEmbedder {
        embedder: Box::new(OllamaEmbedder::new(&url, &model)?),
        model,
    })
}

/// Build the OpenAI-compatible embedder. Both the URL and the model are
/// required — see [`RemoteEndpoint::require`].
#[cfg(feature = "ollama")]
fn build_openai_embedder() -> Result<ConfiguredEmbedder, Box<dyn std::error::Error>> {
    use velesdb_memory::OpenAiEmbedder;

    let (url, model, auth) = embedder_endpoint()?.require("VELESDB_MEMORY_EMBEDDER")?;
    Ok(ConfiguredEmbedder {
        embedder: Box::new(OpenAiEmbedder::new(url, &model, auth)?),
        model,
    })
}

/// Default token budget of `compile-stdin` when `--budget` is omitted.
///
/// Sized for the job the hook does: a tool result big enough to be worth
/// compiling, compressed to something an agent can still read in full.
// Tous ses consommateurs sont gates sur `context` : sans cette feature la
// constante est reellement morte, et -D warnings en fait une erreur.
#[cfg(feature = "context")]
const DEFAULT_COMPILE_STDIN_BUDGET: u64 = 2_000;

/// Parsed `compile-stdin` invocation.
#[cfg(feature = "context")]
#[derive(Debug, PartialEq, Eq)]
struct CompileStdinOptions {
    token_budget: u64,
    query: String,
}

#[cfg(feature = "context")]
impl Default for CompileStdinOptions {
    fn default() -> Self {
        Self {
            token_budget: DEFAULT_COMPILE_STDIN_BUDGET,
            query: String::new(),
        }
    }
}

/// What `compile-stdin` writes to stdout: one JSON object, so a shell hook
/// gets the compiled text AND the accounting from a single stream (`jq` is
/// already a hard requirement of the hooks).
#[cfg(feature = "context")]
#[derive(serde::Serialize)]
struct CompileStdinOutput {
    content: String,
    tokens_in: u64,
    tokens_out: u64,
    tokens_saved: u64,
    risk: String,
}

/// Parse `compile-stdin`'s flags. Hand-rolled for the same reason as
/// `--version`/`--http` above: two flags do not justify a `clap` dependency
/// in the shipped binary.
///
/// # Errors
/// A message naming the offending flag when it is unknown, when its value is
/// missing, or when `--budget` is not a positive integer.
/// Validate `--budget`'s value. Split out of [`parse_compile_stdin_args`] to
/// keep that loop's branching within the repo's complexity ceiling.
///
/// # Errors
/// When the value is absent, not an integer, or zero — a zero budget fits no
/// fragment at all, so it can only ever produce the empty-compilation failure
/// [`compile_stdin_json`] rejects anyway.
#[cfg(feature = "context")]
fn parse_compile_stdin_budget(value: Option<&String>) -> Result<u64, String> {
    let raw = value.ok_or_else(|| "--budget requires a value".to_owned())?;
    let parsed: u64 = raw
        .parse()
        .map_err(|_| format!("--budget expects a positive integer, got {raw:?}"))?;
    if parsed == 0 {
        return Err("--budget must be greater than 0".to_owned());
    }
    Ok(parsed)
}

#[cfg(feature = "context")]
fn parse_compile_stdin_args(args: &[String]) -> Result<CompileStdinOptions, String> {
    let mut options = CompileStdinOptions::default();
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].as_str();
        let value = args.get(index + 1);
        match flag {
            "--budget" => {
                options.token_budget = parse_compile_stdin_budget(value)?;
                index += 2;
            }
            "--query" => {
                options
                    .query
                    .clone_from(value.ok_or_else(|| "--query requires a value".to_owned())?);
                index += 2;
            }
            other => return Err(format!("unknown compile-stdin flag {other:?}")),
        }
    }
    Ok(options)
}

/// Compile `text` under `options` and render the JSON payload.
///
/// # Errors
/// When `text` is empty, when segmentation hits a [`velesdb_memory::limits`]
/// cap, or when the budget leaves no room for any context.
#[cfg(feature = "context")]
fn compile_stdin_json(
    text: &str,
    options: &CompileStdinOptions,
) -> Result<String, Box<dyn std::error::Error>> {
    use velesdb_memory::context::{
        segment_transcript, CompilePolicy, CompileRequest, ContextCompiler, SegmentationPolicy,
    };

    if text.trim().is_empty() {
        return Err("compile-stdin received empty input on stdin".into());
    }

    let outcome = segment_transcript(text, &SegmentationPolicy::default())?;
    let request = CompileRequest {
        query: options.query.clone(),
        fragments: outcome
            .segments
            .into_iter()
            .map(|segment| segment.fragment)
            .collect(),
        project: None,
        target_model: None,
        token_budget: options.token_budget,
        memory_scope: None,
        policy: None,
    };
    let compiled = ContextCompiler::new(CompilePolicy::default()).compile(&request)?;

    // The compiler externalizes rather than truncates: when no single
    // fragment fits, everything moves behind a retrieval handle and the
    // assembled content is empty. That is a legitimate compilation, but a
    // useless one to return as a *replacement* for real content — surface it
    // as an error so the caller keeps the original instead of shipping an
    // empty string.
    if compiled.content.is_empty() {
        return Err(format!(
            "a budget of {} tokens fits none of the {} input tokens — every fragment was \
             externalized and the compiled context is empty; raise --budget",
            options.token_budget, compiled.insights.tokens_in
        )
        .into());
    }

    let output = CompileStdinOutput {
        content: compiled.content,
        tokens_in: compiled.insights.tokens_in,
        tokens_out: compiled.insights.tokens_out,
        tokens_saved: compiled.insights.tokens_saved,
        risk: format!("{:?}", compiled.risk).to_lowercase(),
    };
    Ok(serde_json::to_string(&output)?)
}

/// Read stdin, compile it, print the JSON payload.
///
/// # Errors
/// Propagates flag-parsing, stdin-read, and compilation failures.
#[cfg(feature = "context")]
fn run_compile_stdin(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    use std::io::Read as _;

    let options = parse_compile_stdin_args(args)?;
    let mut text = String::new();
    std::io::stdin().read_to_string(&mut text)?;
    println!("{}", compile_stdin_json(&text, &options)?);
    Ok(())
}

#[cfg(not(feature = "context"))]
fn run_compile_stdin(_args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    Err("`compile-stdin` requires building with `--features context`".into())
}

#[cfg(all(test, feature = "context"))]
mod compile_stdin_tests {
    use super::{
        compile_stdin_json, parse_compile_stdin_args, CompileStdinOptions,
        DEFAULT_COMPILE_STDIN_BUDGET,
    };

    /// A tool-output-shaped corpus: repetitive log lines, the exact case a
    /// `PostToolUse` hook has to shrink.
    fn noisy_tool_output() -> String {
        use std::fmt::Write as _;

        let mut text = String::new();
        for i in 0..120 {
            let _ = writeln!(
                text,
                "[2026-07-25T01:0{}:00Z] INFO  worker: processing batch {} of 120 — retry=0 status=ok",
                i % 10,
                i
            );
        }
        text
    }

    fn parse(value: &str) -> serde_json::Value {
        serde_json::from_str(value).expect("compile-stdin must emit valid JSON")
    }

    #[test]
    fn tight_budget_actually_shrinks_the_payload() {
        let options = CompileStdinOptions {
            token_budget: 1_500,
            query: "what did the worker do".to_owned(),
        };
        let compiled = parse(&compile_stdin_json(&noisy_tool_output(), &options).unwrap());

        let tokens_in = compiled["tokens_in"].as_u64().unwrap();
        let tokens_out = compiled["tokens_out"].as_u64().unwrap();
        assert!(tokens_in > 0, "tokens_in must be measured, got {tokens_in}");
        assert!(
            tokens_out < tokens_in,
            "a 200-token budget over {tokens_in} tokens of logs must compress: got {tokens_out}"
        );
        assert_eq!(
            compiled["tokens_saved"].as_u64().unwrap(),
            tokens_in - tokens_out
        );
        let content = compiled["content"].as_str().unwrap();
        assert!(
            !content.is_empty(),
            "an empty compilation is worse than no compilation — the caller would replace a \
             real tool result with nothing"
        );
        assert!(
            content.len() < noisy_tool_output().len(),
            "the compiled content must be shorter than the raw tool output"
        );
    }

    /// A budget too small to fit even one fragment makes the compiler
    /// externalize everything and emit an EMPTY context. Returning that as a
    /// success is a trap: `compile-stdin`'s caller (a `PostToolUse` hook) would
    /// swap a real tool result for an empty string. Fail loudly instead, so
    /// the caller falls back to the untouched output.
    #[test]
    fn budget_too_small_for_any_fragment_is_an_error() {
        let options = CompileStdinOptions {
            token_budget: 50,
            query: String::new(),
        };
        let error = compile_stdin_json(&noisy_tool_output(), &options).unwrap_err();
        let message = error.to_string();
        assert!(
            message.contains("budget"),
            "the error must point at the budget, got {message}"
        );
    }

    #[test]
    fn compilation_is_byte_identical_across_runs() {
        let options = CompileStdinOptions {
            token_budget: 1_500,
            query: "worker batches".to_owned(),
        };
        let first = compile_stdin_json(&noisy_tool_output(), &options).unwrap();
        let second = compile_stdin_json(&noisy_tool_output(), &options).unwrap();
        assert_eq!(first, second, "the compiler must be deterministic");
    }

    #[test]
    fn empty_stdin_is_rejected() {
        let error = compile_stdin_json("   \n\t ", &CompileStdinOptions::default()).unwrap_err();
        assert!(
            error.to_string().contains("empty"),
            "the error must name the cause, got {error}"
        );
    }

    #[test]
    fn flags_default_and_override() {
        assert_eq!(
            parse_compile_stdin_args(&[]).unwrap(),
            CompileStdinOptions {
                token_budget: DEFAULT_COMPILE_STDIN_BUDGET,
                query: String::new(),
            }
        );
        let parsed = parse_compile_stdin_args(&[
            "--budget".to_owned(),
            "512".to_owned(),
            "--query".to_owned(),
            "why did it fail".to_owned(),
        ])
        .unwrap();
        assert_eq!(parsed.token_budget, 512);
        assert_eq!(parsed.query, "why did it fail");
    }

    #[test]
    fn malformed_flags_are_rejected() {
        for bad in [
            vec!["--budget".to_owned()],
            vec!["--budget".to_owned(), "zero".to_owned()],
            vec!["--budget".to_owned(), "0".to_owned()],
            vec!["--nope".to_owned()],
        ] {
            assert!(
                parse_compile_stdin_args(&bad).is_err(),
                "must reject {bad:?}"
            );
        }
    }
}

#[cfg(all(test, feature = "http"))]
mod tests {
    use super::is_loopback_host;

    #[test]
    fn loopback_v4_and_v6_are_recognized() {
        assert!(is_loopback_host("127.0.0.1:18090"));
        assert!(is_loopback_host("127.0.0.5:18090"));
        assert!(is_loopback_host("[::1]:18090"));
    }

    #[test]
    fn non_loopback_hosts_are_rejected() {
        assert!(!is_loopback_host("0.0.0.0:18090"));
        assert!(!is_loopback_host("192.168.1.10:18090"));
        assert!(!is_loopback_host("[::]:18090"));
        assert!(!is_loopback_host("mcp.example.com:18090"));
    }
}
