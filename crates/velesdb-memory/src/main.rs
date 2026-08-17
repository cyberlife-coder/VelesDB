//! `velesdb-memory` — MCP memory server binary (stdio transport by default).
//!
//! Serves the memory tools over stdio so any MCP client (Claude Code, Cursor,
//! Cline, Zed, …) can use it locally. The store never leaves the machine.
//! Configure the store directory with `VELESDB_MEMORY_PATH` (default
//! `~/.velesdb-memory`) and the embedding
//! backend with `VELESDB_MEMORY_EMBEDDER` (`hash` | `ollama` | `openai`). Set
//! `VELESDB_MEMORY_EXTRACTOR` to set `remember_extracted`'s default backend
//! (calls may override it): `outline` reads directives you write explicitly
//! and needs no model and no extra feature, while `ollama` and `openai` infer
//! them with a generative model and need `--features extractor-http`.
//!
//! Each role carries its own `_URL`, `_MODEL` and `_API_TOKEN`, and the two are
//! configured independently — embedding on a local Ollama while extracting on
//! an OpenAI-compatible server is a supported combination. `openai` names a
//! *protocol* (oMLX, llama.cpp, LM Studio, vLLM, hosted providers all speak
//! it), so it has no default URL and no default model: reaching a different
//! server is a different URL, never a new backend name. Tokens are read from
//! the environment only, never from the config file. Set
//! `VELESDB_MEMORY_DEFAULT_TTL` (seconds) to expire remembered facts by default.
//! Set `VELESDB_MEMORY_LOG` (`EnvFilter` directives, e.g. `info`) for
//! per-request stderr logging — unset is fully silent; see
//! `velesdb_memory::logging` for the payload-safety contract.
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

#[path = "daemon_backends.rs"]
mod backends;
#[path = "daemon_commands.rs"]
mod commands;
#[path = "daemon_serve.rs"]
mod serve;
#[path = "daemon_startup.rs"]
mod startup;

use commands::{run_compile_stdin, run_export, run_migrate_embeddings};
use serve::{apply_logging, requested_http_bind, serve_selected_transport};
use startup::{build_configured_server, build_configured_service};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if let Some(result) = run_early_command(&args) {
        return result;
    }
    apply_logging();
    #[cfg(unix)]
    let original_parent = std::os::unix::process::parent_id();
    #[cfg(not(unix))]
    let original_parent = 0_u32;
    let configured = build_configured_service(&args)?;
    let http_bind = requested_http_bind(&args);
    let server = build_configured_server(configured)?;
    tokio::runtime::Runtime::new()?.block_on(serve_selected_transport(
        server,
        http_bind,
        original_parent,
    ))
}

/// Commands that must not open the live store or build its embedder.
fn run_early_command(args: &[String]) -> Option<Result<(), Box<dyn std::error::Error>>> {
    match args.get(1).map(String::as_str) {
        Some("--version" | "-V") => {
            println!("velesdb-memory {}", env!("CARGO_PKG_VERSION"));
            Some(Ok(()))
        }
        Some("compile-stdin") => Some(run_compile_stdin(&args[2..])),
        Some("migrate-embeddings") => Some(run_migrate_embeddings(args, &args[2..])),
        Some("export") => Some(run_export(args, &args[2..])),
        _ => None,
    }
}
