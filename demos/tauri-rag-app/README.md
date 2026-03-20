# VelesDB RAG Desktop App

> **Difficulty: Advanced** | Showcases: Vector search, Tauri desktop integration, RAG pipeline, knowledge graph, real-time events

Build a local **Retrieval-Augmented Generation (RAG)** desktop application using:
- **Tauri 2.0** - Rust-based desktop framework
- **VelesDB** - Lightning-fast vector search (microseconds!)
- **React + TypeScript** - Modern frontend

## 🎯 What You'll Build

A desktop app that:
1. Ingests documents (markdown, text)
2. Generates embeddings locally
3. Stores vectors in VelesDB
4. Searches semantically with RAG
5. Works 100% offline

## 📋 Prerequisites

- [Rust](https://rustup.rs/) (1.83+)
- [Node.js](https://nodejs.org/) (18+)
- [Tauri CLI](https://v2.tauri.app/start/prerequisites/)

```bash
# Install Tauri CLI
cargo install tauri-cli
```

## 🚀 Quick Start

### 1. Create the project

```bash
# Clone this example
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

## 📁 Project Structure

```
tauri-rag-app/
├── src/                    # React frontend
│   ├── App.tsx            # Main component
│   ├── components/
│   │   ├── SearchBar.tsx  # Vector search input
│   │   ├── Results.tsx    # Search results display
│   │   └── Ingest.tsx     # Document ingestion
│   └── lib/
│       └── velesdb.ts     # VelesDB client wrapper
├── src-tauri/             # Rust backend
│   ├── src/
│   │   ├── main.rs        # Tauri entry point
│   │   ├── commands.rs    # Tauri commands
│   │   └── rag.rs         # RAG logic with VelesDB
│   ├── Cargo.toml
│   └── tauri.conf.json
└── package.json
```

## 🔧 How It Works

### 1. Document Ingestion

```typescript
// Frontend: Send document to Tauri backend
import { invoke } from '@tauri-apps/api/core';

const chunks = await invoke('ingest_document', {
  path: '/path/to/document.md'
});
```

### 2. Vector Storage (Rust)

```rust
use tauri_plugin_velesdb::VelesDB;

// Store embeddings in VelesDB
let db = VelesDB::new(768)?; // 768 dimensions for typical embeddings

for chunk in chunks {
    let embedding = generate_embedding(&chunk.text)?;
    db.insert(chunk.id, &embedding, Some(chunk.text))?;
}
```

### 3. Semantic Search

```rust
// Search with VelesDB (microsecond latency!)
let query_embedding = generate_embedding(&query)?;
let results = db.search(&query_embedding, 5)?; // Top 5 results

// Results contain the most relevant chunks for RAG context
```

### 4. RAG Response

The search results provide context for your LLM (local or API-based):

```rust
let context = results.iter()
    .map(|r| r.payload.as_ref().unwrap())
    .collect::<Vec<_>>()
    .join("\n\n");

let prompt = format!(
    "Based on the following context, answer the question.\n\n\
    Context:\n{context}\n\n\
    Question: {query}"
);
```

## 🎨 Frontend Components

### SearchBar.tsx

```tsx
import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

export function SearchBar({ onResults }) {
  const [query, setQuery] = useState('');

  const handleSearch = async () => {
    const results = await invoke('search', { query, k: 5 });
    onResults(results);
  };

  return (
    <div className="flex gap-2">
      <input
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        placeholder="Ask a question..."
        className="flex-1 px-4 py-2 border rounded-lg"
      />
      <button onClick={handleSearch} className="px-4 py-2 bg-blue-500 text-white rounded-lg">
        Search
      </button>
    </div>
  );
}
```

## ⚡ Performance

VelesDB provides **microsecond-level** latency:

| Operation | Time |
|-----------|------|
| Insert 10k vectors (768D) | ~25ms total |
| Search 10k vectors | ~1ms |
| Search 100k vectors | ~50ms |

This makes it perfect for:
- **Real-time RAG** - No perceptible delay
- **Offline-first** - Works without internet
- **Privacy** - Data never leaves your machine

## 🔒 Privacy Benefits

- All data stored locally (SQLite + VelesDB)
- No cloud dependencies
- Use local LLMs (Ollama, llama.cpp) for complete privacy

## Knowledge Graph (NEW)

Build relationships between documents:

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

## 📡 Real-time Events (NEW)

Listen to database changes:

```typescript
import { listen } from '@tauri-apps/api/event';

// Get notified when documents are added
await listen('velesdb://collection-updated', (event) => {
  console.log(`${event.payload.count} documents added`);
  refreshUI();
});
```

## Next Steps

1. **Add local LLM** - Integrate Ollama for complete offline RAG
2. **PDF support** - Use `pdf-extract` crate for PDFs
3. **Chunking strategies** - Implement smart text splitting
4. **Hybrid search** - Combine vector + keyword search (BM25)
5. **Knowledge Graph** - Build document relationships ⭐ NEW

## 📝 License

MIT License
