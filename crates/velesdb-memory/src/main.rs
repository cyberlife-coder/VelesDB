//! `velesdb-memory` — MCP memory server binary (stdio transport).
//!
//! Serves the memory tools over stdio so any MCP client (Claude Code, Cursor,
//! Cline, Zed, …) can use it locally. The store never leaves the machine.
//! Configure the store directory with `VELESDB_MEMORY_PATH` and the embedding
//! backend with `VELESDB_MEMORY_EMBEDDER` (`hash` | `ollama`). When built with
//! `--features extract`, set `VELESDB_MEMORY_EXTRACTOR=ollama` to enable the
//! `remember_extracted` tool (auto text → fact↔topic graph).

use rmcp::ServiceExt;
use velesdb_memory::mcp::McpServer;
use velesdb_memory::{DynEmbedder, HashEmbedder, MemoryService, DEFAULT_DIMENSION};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let store_path = std::env::var("VELESDB_MEMORY_PATH")
        .unwrap_or_else(|_| "./velesdb-memory-store".to_owned());

    // All synchronous setup (env probing, blocking HTTP to Ollama, disk open)
    // runs here, before the async runtime starts, so we never block a tokio
    // worker thread on a synchronous operation.
    let embedder = build_embedder()?;
    let service = MemoryService::open(&store_path, embedder)?;
    let server = build_server(service)?;

    tokio::runtime::Runtime::new()?.block_on(async move {
        let running = server
            .serve((tokio::io::stdin(), tokio::io::stdout()))
            .await?;
        running.waiting().await?;
        Ok::<(), Box<dyn std::error::Error>>(())
    })
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
        Ok("hash") | Err(_) => Ok(Box::new(HashEmbedder::new(DEFAULT_DIMENSION))),
        Ok(other) => Err(format!(
            "unknown VELESDB_MEMORY_EMBEDDER '{other}' (expected 'hash' or 'ollama')"
        )
        .into()),
    }
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
