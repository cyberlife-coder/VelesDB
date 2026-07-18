//! `velesdb-memory` — MCP memory server binary (stdio transport).
//!
//! Serves the memory tools over stdio so any MCP client (Claude Code, Cursor,
//! Cline, Zed, …) can use it locally. The store never leaves the machine.
//! Configure the store directory with `VELESDB_MEMORY_PATH` (default
//! `~/.velesdb-memory`) and the embedding
//! backend with `VELESDB_MEMORY_EMBEDDER` (`hash` | `ollama`). When built with
//! `--features extract`, set `VELESDB_MEMORY_EXTRACTOR=ollama` to enable the
//! `remember_extracted` tool (auto text → fact↔topic graph). Set
//! `VELESDB_MEMORY_DEFAULT_TTL` (seconds) to expire remembered facts by default.

use std::time::Duration;

use rmcp::ServiceExt;
use velesdb_memory::mcp::McpServer;
use velesdb_memory::{DynEmbedder, HashEmbedder, MemoryService, NativeStore, DEFAULT_DIMENSION};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Captured FIRST — before the (possibly seconds-long) embedder probe and
    // store open — so a client that exits during our own startup still
    // reparents us AFTER the baseline, and the watchdog sees the change. A
    // baseline taken later would read the already-reparented pid and go
    // permanently inert (review finding on #1449).
    #[cfg(unix)]
    let original_parent = std::os::unix::process::parent_id();
    #[cfg(not(unix))]
    let original_parent = 0_u32;
    let store_path = std::env::var("VELESDB_MEMORY_PATH").unwrap_or_else(|_| default_store_path());

    // All synchronous setup (env probing, blocking HTTP to Ollama, disk open)
    // runs here, before the async runtime starts, so we never block a tokio
    // worker thread on a synchronous operation.
    let embedder = build_embedder()?;
    let service = open_store_with_actionable_lock_error(&store_path, embedder)?;
    let server = apply_default_ttl(build_server(service)?)?;

    tokio::runtime::Runtime::new()?.block_on(async move {
        spawn_orphan_watchdog(original_parent);
        let running = server
            .serve((tokio::io::stdin(), tokio::io::stdout()))
            .await?;
        running.waiting().await?;
        Ok::<(), Box<dyn std::error::Error>>(())
    })
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
    embedder: DynEmbedder,
) -> Result<MemoryService<DynEmbedder>, Box<dyn std::error::Error>> {
    use velesdb_memory::MemoryError;

    let dimension = embedder.dimension();
    let mut last_locked_path: Option<String> = None;
    for attempt in 0..LOCK_RETRY_ATTEMPTS {
        match NativeStore::open(store_path, dimension) {
            Ok(store) => return Ok(MemoryService::with_store(store, embedder)),
            Err(MemoryError::Storage(velesdb_core::Error::DatabaseLocked(locked_path))) => {
                last_locked_path = Some(locked_path);
                if attempt + 1 < LOCK_RETRY_ATTEMPTS {
                    std::thread::sleep(LOCK_RETRY_DELAY);
                }
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

/// Default store location when `VELESDB_MEMORY_PATH` is unset: `~/.velesdb-memory`
/// (the path advertised in `server.json`, the README, and every client-config
/// snippet). A stable home-based path — never a `./`-relative one: an MCP server
/// is launched by its client with an unpredictable working directory, so a
/// cwd-relative default would scatter (or lose) the store between sessions. Falls
/// back to a cwd-relative path only when no home directory can be resolved.
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

/// Build the MCP server, attaching an extraction backend from
/// `VELESDB_MEMORY_EXTRACTOR` (`ollama`) when built with `--features extract`.
#[cfg(feature = "extract")]
fn build_server(
    service: MemoryService<DynEmbedder>,
) -> Result<McpServer, Box<dyn std::error::Error>> {
    let server = McpServer::new(service);
    match std::env::var("VELESDB_MEMORY_EXTRACTOR").as_deref() {
        Ok("ollama") => Ok(server.with_extractor(build_ollama_extractor()?)),
        Ok("none") | Err(_) => Ok(server),
        Ok(other) => {
            Err(format!("unknown VELESDB_MEMORY_EXTRACTOR '{other}' (expected 'ollama')").into())
        }
    }
}

/// Without the `extract` feature there is no extraction backend to attach. The
/// `Result` return mirrors the `extract` variant's signature so the caller is
/// identical for both builds.
#[cfg(not(feature = "extract"))]
#[allow(clippy::unnecessary_wraps)]
fn build_server(
    service: MemoryService<DynEmbedder>,
) -> Result<McpServer, Box<dyn std::error::Error>> {
    Ok(McpServer::new(service))
}

/// Build the Ollama-backed extractor from `VELESDB_MEMORY_EXTRACTOR_URL`
/// (default local) and the required `VELESDB_MEMORY_EXTRACTOR_MODEL`.
#[cfg(feature = "extract")]
fn build_ollama_extractor() -> Result<velesdb_memory::DynExtractor, Box<dyn std::error::Error>> {
    use std::sync::Arc;
    use velesdb_memory::extract::DEFAULT_OLLAMA_URL;
    use velesdb_memory::OllamaExtractor;

    let url = std::env::var("VELESDB_MEMORY_EXTRACTOR_URL")
        .unwrap_or_else(|_| DEFAULT_OLLAMA_URL.to_owned());
    let model = std::env::var("VELESDB_MEMORY_EXTRACTOR_MODEL").map_err(|_| {
        "VELESDB_MEMORY_EXTRACTOR=ollama requires VELESDB_MEMORY_EXTRACTOR_MODEL \
         (e.g. qwen3.6:35b-mlx)"
    })?;
    Ok(Arc::new(OllamaExtractor::new(url, model)))
}

/// Select the embedding backend from `VELESDB_MEMORY_EMBEDDER`: `hash`
/// (default) is deterministic and fully offline; `ollama` gives real on-device
/// semantic recall and requires building with `--features ollama`.
fn build_embedder() -> Result<DynEmbedder, Box<dyn std::error::Error>> {
    match std::env::var("VELESDB_MEMORY_EMBEDDER").as_deref() {
        Ok("ollama") => build_ollama_embedder(),
        Ok("hash") | Err(_) => {
            warn_hash_embedder_not_semantic();
            Ok(Box::new(HashEmbedder::new(DEFAULT_DIMENSION)))
        }
        Ok(other) => Err(format!(
            "unknown VELESDB_MEMORY_EMBEDDER '{other}' (expected 'hash' or 'ollama')"
        )
        .into()),
    }
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

#[cfg(feature = "ollama")]
fn build_ollama_embedder() -> Result<DynEmbedder, Box<dyn std::error::Error>> {
    use velesdb_memory::{OllamaEmbedder, DEFAULT_OLLAMA_MODEL, DEFAULT_OLLAMA_URL};

    let url = std::env::var("VELESDB_MEMORY_OLLAMA_URL")
        .unwrap_or_else(|_| DEFAULT_OLLAMA_URL.to_owned());
    let model = std::env::var("VELESDB_MEMORY_OLLAMA_MODEL")
        .unwrap_or_else(|_| DEFAULT_OLLAMA_MODEL.to_owned());
    Ok(Box::new(OllamaEmbedder::new(url, model)?))
}

#[cfg(not(feature = "ollama"))]
fn build_ollama_embedder() -> Result<DynEmbedder, Box<dyn std::error::Error>> {
    Err("the 'ollama' embedder requires building with `--features ollama`".into())
}
