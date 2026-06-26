//! `velesdb-memory` — MCP memory server binary (stdio transport).
//!
//! Serves the five memory tools over stdio so any MCP client (Claude Code,
//! Cursor, Cline, Zed, …) can use it locally. The store never leaves the
//! machine. Configure the store directory with `VELESDB_MEMORY_PATH` and the
//! embedding backend with `VELESDB_MEMORY_EMBEDDER` (`hash` | `ollama`).

use rmcp::ServiceExt;
use velesdb_memory::mcp::{DynEmbedder, McpServer};
use velesdb_memory::{HashEmbedder, MemoryService, DEFAULT_DIMENSION};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let store_path = std::env::var("VELESDB_MEMORY_PATH")
        .unwrap_or_else(|_| "./velesdb-memory-store".to_owned());

    let embedder = build_embedder()?;
    let service = MemoryService::open(&store_path, embedder)?;

    let running = McpServer::new(service)
        .serve((tokio::io::stdin(), tokio::io::stdout()))
        .await?;
    running.waiting().await?;
    Ok(())
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
