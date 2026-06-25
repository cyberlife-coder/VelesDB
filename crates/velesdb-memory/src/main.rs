//! `velesdb-memory` — MCP memory server binary (stdio transport).
//!
//! Serves the five memory tools over stdio so any MCP client (Claude Code,
//! Cursor, Cline, Zed, …) can use it locally. The store never leaves the
//! machine. Configure the store directory with `VELESDB_MEMORY_PATH`.

use rmcp::ServiceExt;
use velesdb_memory::mcp::{DynEmbedder, McpServer};
use velesdb_memory::{HashEmbedder, MemoryService, DEFAULT_DIMENSION};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let store_path = std::env::var("VELESDB_MEMORY_PATH")
        .unwrap_or_else(|_| "./velesdb-memory-store".to_owned());

    // Deterministic, fully offline embedder by default. Plug a real on-device
    // model (e.g. fastembed / all-MiniLM-L6-v2, 384-dim) for production recall.
    let embedder: DynEmbedder = Box::new(HashEmbedder::new(DEFAULT_DIMENSION));
    let service = MemoryService::open(&store_path, embedder)?;

    let running = McpServer::new(service)
        .serve((tokio::io::stdin(), tokio::io::stdout()))
        .await?;
    running.waiting().await?;
    Ok(())
}
