# VelesDB RAG Desktop App

> **Difficulty: Advanced** | Showcases: Vector search, Tauri desktop integration, RAG pipeline, knowledge graph, real-time events

Build a local **Retrieval-Augmented Generation (RAG)** desktop application using:
- **Tauri 2.0** - Rust-based desktop framework
- **VelesDB** - Lightning-fast vector search (microseconds!)
- **React + TypeScript** - Modern frontend

## What You'll Build

A desktop app that:
1. Ingests text documents (markdown, plain text)
2. Generates embeddings locally with fastembed (AllMiniLML6V2, 384D)
3. Stores vectors in VelesDB via `tauri-plugin-velesdb`
4. Searches semantically with cosine similarity
5. Works 100% offline

## Prerequisites

- [Rust](https://rustup.rs/) (1.89+)
- [Node.js](https://nodejs.org/) (18+)
- [Tauri CLI](https://v2.tauri.app/start/prerequisites/)

```bash
# Install Tauri CLI
cargo install tauri-cli
```

## Quick Start

### 1. Set up the project

```bash
# Navigate to this demo
cd demos/tauri-rag-app

# Install frontend dependencies
npm install

# Run in development
cargo tauri dev
```

### 2. Build for production

```bash
cargo tauri build
```

## Project Structure

```
tauri-rag-app/
├── src/                       # React frontend
│   ├── main.tsx               # React entry point
│   ├── App.tsx                # Main component with tabs
│   └── components/
│       ├── SearchBar.tsx      # Vector search input
│       ├── Results.tsx        # Search results display
│       └── Ingest.tsx         # Text ingestion + clear index
├── src-tauri/                 # Rust backend
│   ├── src/
│   │   ├── main.rs            # Tauri entry point + command registration
│   │   ├── commands.rs        # Tauri commands (ingest, search, stats)
│   │   ├── embeddings.rs      # fastembed AllMiniLML6V2 wrapper (384D)
│   │   └── tests.rs           # Unit tests for chunking + serialization
│   ├── Cargo.toml
│   └── tauri.conf.json
└── package.json
```

## How It Works

### 1. Text Ingestion

The frontend sends raw text to the Tauri backend, which chunks it on paragraph
boundaries, generates 384-dimensional embeddings with fastembed, and stores the
vectors in VelesDB:

```typescript
import { invoke } from '@tauri-apps/api/core';

// Chunk and embed text, then store in VelesDB
const chunks = await invoke<Chunk[]>('ingest_text', {
  text: 'Your document content here...',
  chunkSize: 500  // optional, defaults to 500 bytes
});
```

### 2. Vector Storage (Rust)

The backend uses `tauri-plugin-velesdb` for persistent vector storage with
fastembed for local ML embeddings (AllMiniLML6V2, 384 dimensions):

```rust
use tauri_plugin_velesdb::VelesDbState;
use velesdb_core::{Database, DistanceMetric, Point};
use std::sync::Arc;

// Ensure collection exists with 384 dimensions (AllMiniLML6V2)
state.with_db(|db: Arc<Database>| {
    db.create_vector_collection("rag-docs", 384, DistanceMetric::Cosine)
        .map_err(|e| /* ... */)?;
    Ok(())
})?;

// Store chunks with text in payload
let point = Point::new(id, embedding, Some(serde_json::json!({ "text": chunk })));
coll.upsert_bulk(&[point])?;
```

### 3. Semantic Search

Search embeds the query text and finds the nearest vectors. The `time_ms` field
reports VelesDB search latency only (embedding time excluded):

```rust
// Embed query, then search VelesDB
let query_embedding = embeddings::embed_text(&query).await?;
let results = coll.search(&query_embedding, k)?;

// Extract text from each result's payload
for sr in &results {
    let text = sr.point.payload.as_ref()
        .and_then(|p| p.get("text"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
}
```

### 4. RAG Response

The search results provide context for an LLM (local or API-based):

```rust
let context: Vec<&str> = results.iter()
    .filter_map(|sr| sr.point.payload.as_ref()
        .and_then(|p| p.get("text"))
        .and_then(|v| v.as_str()))
    .collect();

let prompt = format!(
    "Based on the following context, answer the question.\n\n\
    Context:\n{}\n\n\
    Question: {query}",
    context.join("\n\n")
);
```

## Tauri Commands

The app registers six Tauri commands in `main.rs`:

| Command | Parameters | Returns | Description |
|---------|-----------|---------|-------------|
| `ingest_text` | `text: string`, `chunkSize?: number` | `Chunk[]` | Chunk text, embed, store in VelesDB |
| `search` | `query: string`, `k?: number` | `SearchResult` | Semantic search (default top-5) |
| `get_stats` | _(none)_ | `IndexStats` | Total chunks + vector dimension |
| `clear_index` | _(none)_ | `void` | Drop and recreate the collection |
| `get_model_status` | _(none)_ | `ModelStatus` | Check if ML model is loaded |
| `preload_model` | _(none)_ | `void` | Download/init model at startup |

## Frontend Components

### SearchBar.tsx

```tsx
import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

interface SearchBarProps {
  onResults: (results: SearchResult) => void;
}

export function SearchBar({ onResults }: SearchBarProps) {
  const [query, setQuery] = useState('');
  const [loading, setLoading] = useState(false);

  const handleSearch = async () => {
    if (!query.trim()) return;
    setLoading(true);
    try {
      const results = await invoke<SearchResult>('search', { query, k: 5 });
      onResults(results);
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="flex gap-2">
      <input
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        onKeyDown={(e) => e.key === 'Enter' && handleSearch()}
        placeholder="Ask a question about your documents..."
      />
      <button onClick={handleSearch} disabled={loading || !query.trim()}>
        {loading ? 'Searching...' : 'Search'}
      </button>
    </div>
  );
}
```

## Performance

VelesDB provides **microsecond-level** latency:

| Operation | Time |
|-----------|------|
| Insert 10k vectors (384D) | ~25ms total |
| Search 10k vectors | ~1ms |
| Search 100k vectors | ~50ms |

This makes it perfect for:
- **Real-time RAG** - No perceptible delay
- **Offline-first** - Works without internet
- **Privacy** - Data never leaves your machine

## Privacy Benefits

- All data stored locally in VelesDB (persistent, mmap-backed)
- No cloud dependencies
- Use local LLMs (Ollama, llama.cpp) for complete privacy

## Knowledge Graph

Build relationships between documents using the `tauri-plugin-velesdb` graph
commands:

```typescript
import { invoke } from '@tauri-apps/api/core';

// Add relationships between documents
await invoke('plugin:velesdb|add_edge', {
  request: {
    collection: 'documents',
    id: 1,
    source: docA.id,
    target: docB.id,
    label: 'REFERENCES',
    properties: { section: 'introduction' }
  }
});

// Traverse related documents
const related = await invoke('plugin:velesdb|traverse_graph', {
  request: {
    collection: 'documents',
    source: docA.id,
    maxDepth: 2,
    limit: 10
  }
});
```

## Real-time Events

Listen to database changes emitted by `tauri-plugin-velesdb`:

```typescript
import { listen } from '@tauri-apps/api/event';

// Get notified when a collection is modified (upsert/delete)
await listen('velesdb://collection-updated', (event) => {
  const { collection, operation, count } = event.payload;
  console.log(`${collection}: ${operation} (${count} items)`);
  refreshUI();
});
```

## Next Steps

1. **Add local LLM** - Integrate Ollama for complete offline RAG
2. **PDF support** - Use `pdf-extract` crate for PDFs
3. **Chunking strategies** - Implement smart text splitting
4. **Hybrid search** - Combine vector + keyword search (BM25)
5. **Knowledge Graph** - Build document relationships with graph traversal

## License

MIT License
