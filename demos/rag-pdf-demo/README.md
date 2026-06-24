# VelesDB RAG Demo - PDF Question Answering

> **Difficulty: Intermediate** | Showcases: Embedded VelesDB Python SDK, vector search, PDF ingestion, semantic search, FastAPI integration

A complete RAG (Retrieval-Augmented Generation) demo using **VelesDB** for vector storage, with PDF document ingestion and semantic search.

## 🎯 Features

- **PDF Upload & Processing** - Extract text from PDF documents using PyMuPDF
- **Automatic Chunking** - Split documents into optimal chunks (512 chars, 50 overlap)
- **Multilingual Embeddings** - Uses `paraphrase-multilingual-MiniLM-L12-v2` (50+ languages)
- **VelesDB Storage** - Ultra-fast vector search with HNSW algorithm, embedded in-process
- **Semantic Search** - Find relevant passages with cosine similarity
- **Real-time Metrics** - Performance timing displayed in UI
- **FastAPI REST surface** - Simple HTTP endpoints for integration with other apps

## 🏗️ Architecture

```
┌──────────────┐     ┌─────────────────────────────────────┐
│   Frontend   │────▶│            FastAPI Backend          │
│  (Upload UI) │     │  ┌───────────────────────────────┐  │
└──────────────┘     │  │  Embedded VelesDB (PyO3 SDK)  │  │
                     │  │   - HNSW index + payloads     │  │
                     │  │   - persistent on disk        │  │
                     │  └───────────────────────────────┘  │
                     └───────┬─────────────────────┬───────┘
                             │                     │
                      ┌──────▼──────┐       ┌──────▼─────┐
                      │   PyMuPDF   │       │  Sentence  │
                      │ (PDF Parse) │       │Transformers│
                      └─────────────┘       └────────────┘
```

> VelesDB runs **in-process** via the native Python bindings (`pip install velesdb`).
> There is no separate `velesdb-server` to start — the FastAPI app opens the
> database directly on disk and the GIL is released for every Rust call.

## 🚀 Quick Start

### Prerequisites

**Python 3.10+** — that's all. VelesDB is pulled in as a regular Python package; no separate server process required.

### Installation

```bash
cd demos/rag-pdf-demo

# Create virtual environment
python -m venv .venv
.venv\Scripts\activate  # Windows
# source .venv/bin/activate  # Linux/macOS

# Install dependencies
pip install -e ".[dev]"
```

### Run Tests (TDD)

```bash
pytest
```

### Start the Demo

```bash
# Start the API server
uvicorn src.main:app --reload --port 8000

# Open browser (try these URLs in order if first doesn't work):
start http://127.0.0.1:8000        # ✅ Most reliable
start http://localhost:8000        # 🔄 May fail on some systems
```

### 🔧 Troubleshooting Connection Issues

**If you see "ERR_CONNECTION_REFUSED" or "This site can't be reached":**

1. **Try 127.0.0.1 instead of localhost:**
   - Use `http://127.0.0.1:8000` instead of `http://localhost:8000`
   - This bypasses DNS resolution and works on all systems

2. **Check the FastAPI server is running:**
   ```bash
   curl http://127.0.0.1:8000/health
   ```

3. **Verify the port is not blocked:**
   ```bash
   netstat -ano | findstr ":8000"
   ```

4. **Common causes:**
   - **Windows Firewall** blocking localhost connections
   - **Antivirus software** interfering with local ports
   - **Corporate VPN** redirecting traffic
   - **DNS resolution** issues with "localhost"

5. **Alternative URLs to try:**
   - `http://127.0.0.1:8000/docs` - Swagger API documentation
   - `http://127.0.0.1:8000/health` - Health check endpoint

## 📖 API Endpoints

### Upload PDF
```bash
curl -X POST "http://127.0.0.1:8000/documents/upload" \
  -F "file=@document.pdf"
```

### Search Documents
```bash
curl -X POST "http://127.0.0.1:8000/search" \
  -H "Content-Type: application/json" \
  -d '{"query": "What is machine learning?", "top_k": 5}'
```

**Response includes performance metrics:**
```json
{
  "query": "What is machine learning?",
  "results": [...],
  "total_results": 5,
  "search_time_ms": 5.2,
  "embedding_time_ms": 12.1
}
```

### List Documents
```bash
curl "http://127.0.0.1:8000/documents"
```

### Delete Document
```bash
curl -X DELETE "http://127.0.0.1:8000/documents/your-document.pdf"
```

### Load Demo Data
```bash
curl -X POST "http://127.0.0.1:8000/demo/load"
```

### Health Check
```bash
curl "http://127.0.0.1:8000/health"
```

**Response:**
```json
{
  "status": "healthy",
  "velesdb_connected": true,
  "embedding_model": "paraphrase-multilingual-MiniLM-L12-v2",
  "embedding_dimension": 384,
  "documents_count": 3
}
```

## 🔧 Configuration

Environment variables (`.env` file):

```env
EMBEDDING_MODEL=paraphrase-multilingual-MiniLM-L12-v2
EMBEDDING_DIMENSION=384
CHUNK_SIZE=512
CHUNK_OVERLAP=50
```

> The legacy `VELESDB_URL` setting in `src/config.py` is kept as a no-op for
> backward compatibility with older `.env` files — it is ignored by the
> embedded client.

| Parameter | Default Value |
|-----------|---------------|
| Embedding Model | `paraphrase-multilingual-MiniLM-L12-v2` |
| Embedding Dimensions | 384 |
| Chunk Size | 512 characters |
| Chunk Overlap | 50 characters |
| Distance Metric | Cosine similarity |

## 📊 Performance Benchmarks

Benchmarks measured on Windows 11, Python 3.10, VelesDB 1.11.1 (500 iterations — historical baseline from a REST-based earlier version of this demo). The current native-bindings path keeps the VelesDB search cost the same and removes the HTTP layer entirely.

### Layer-by-Layer Latency (historical, REST-era)

| Layer | Mean | P95 | StdDev | Notes |
|-------|------|-----|--------|-------|
| ~~TCP Connection~~ | ~~0.29ms~~ | ~~0.51ms~~ | ~~0.16ms~~ | Removed — embedded SDK |
| ~~HTTP Client Creation~~ | ~~6.41ms~~ | ~~7.35ms~~ | ~~2.20ms~~ | Removed — embedded SDK |
| ~~HTTP Request (persistent)~~ | ~~0.61ms~~ | ~~0.81ms~~ | ~~0.09ms~~ | Removed — embedded SDK |
| **VelesDB Search (sync)** | **0.89ms** | 1.45ms | 0.23ms | ✅ Unchanged, GIL released |
| VelesDB Search (async) | 2.65ms | 3.17ms | 0.28ms | ✅ Pre-native, kept for reference |
| **Full API Search** | **19.10ms** | 24.68ms | 5.80ms | Dominated by embedding model |

### Full API Breakdown

| Component | Mean | StdDev | % of Total |
|-----------|------|--------|------------|
| **Embedding** | 12.09ms | 5.51ms | 63% |
| **VelesDB** | 5.33ms | 0.68ms | 28% |
| **Overhead** | ~1.68ms | - | 9% |

### Document Ingestion

| Component | Latency | Description |
|-----------|---------|-------------|
| **PDF Processing** | ~45ms | PyMuPDF text extraction |
| **Embedding Generation** | ~170ms/chunk | Batch encoding |
| **VelesDB Insert** | ~12ms | Upsert vectors |

### Cold Start vs Warm

| Metric | Cold Start | After Warm-up |
|--------|------------|---------------|
| First Search | ~300ms | - |
| Subsequent Searches | - | ~19ms |
| Model Loading | ~2-3s | Cached |

### Comparison with Other Solutions

| Solution | Search Latency | Notes |
|----------|---------------|-------|
| **VelesDB (this demo)** | **0.89ms** | Embedded PyO3 bindings, HNSW |
| VelesDB (native Rust) | <1ms | Direct integration |
| Pinecone | ~50-100ms | Cloud service |
| Qdrant | ~10-50ms | Self-hosted |
| FAISS | ~1ms | In-memory only |

> **Note**: `benchmark_latency.py` is a legacy REST-era client that targets a running `velesdb-server` on `localhost:8080`; this demo now runs VelesDB embedded (in-process, no server), so the script only works if you separately start a `velesdb-server`.

## 🧪 Testing

```bash
# Run all tests
pytest

# With coverage
pytest --cov=src --cov-report=html

# Specific test
pytest tests/test_embeddings.py -v
```

## 📁 Project Structure

```
rag-pdf-demo/
├── src/
│   ├── __init__.py
│   ├── main.py           # FastAPI app with metrics
│   ├── config.py         # Settings (model, chunks, etc.)
│   ├── models.py         # Pydantic models with timing fields
│   ├── pdf_processor.py  # PDF text extraction (PyMuPDF)
│   ├── embeddings.py     # Sentence-transformers wrapper
│   ├── velesdb_client.py # VelesDB native client (PyO3 bindings)
│   └── rag_engine.py     # RAG orchestration with timing
├── tests/
│   ├── __init__.py
│   ├── conftest.py       # Fixtures
│   ├── test_pdf_processor.py
│   ├── test_embeddings.py
│   ├── test_velesdb_client.py
│   └── test_rag_engine.py
├── static/
│   ├── index.html        # UI with real-time metrics
│   ├── demo_data.json    # Pre-loaded demo documents
│   └── velesdb-icon.png  # VelesDB logo
├── benchmark_latency.py  # Performance benchmarks
├── pyproject.toml
└── README.md
```

## 🗄️ Data Persistence & Cleanup

### ⚠️ Important: Data is Persistent!

**VelesDB stores data on disk by default** - when you stop the demo, **all documents remain indexed** in:
```
./rag-data/rag_documents/
├── config.json      # Collection metadata
├── vectors.dat      # Vector embeddings (16MB+)
├── vectors.idx      # Vector index
└── vectors.wal      # Write-ahead log
```

### 🧹 Complete Cleanup Options

#### Option 1: Manual File Cleanup (Recommended)
```bash
# Stop the FastAPI server first (Ctrl+C)
# Then delete the data directory — VelesDB owns this folder fully
Remove-Item .\rag-data -Recurse -Force      # Windows PowerShell
# rm -rf ./rag-data                          # Linux/macOS
```

The next time you start the demo, VelesDB creates a fresh collection from scratch.

#### Option 2: Individual Document Deletion
```bash
# Via web interface: Click trash icon next to document
# Or via the FastAPI endpoint:
curl -X DELETE "http://127.0.0.1:8000/documents/your-document.pdf"
```

### 📊 Storage Usage
- **PDF documents**: Not stored (only processed)
- **Text chunks**: Stored in VelesDB as payloads
- **Embeddings**: 384 dimensions × 4 bytes = ~1.5KB per chunk
- **Index overhead**: ~20-30% additional storage

### 🔍 Check Current Storage
```bash
# Check disk usage
dir .\rag-data\rag_documents      # Windows PowerShell
# du -sh ./rag-data/rag_documents  # Linux/macOS

# Programmatic stats via the demo API:
curl "http://127.0.0.1:8000/health"
```

## 🔬 Technical Details

### Embedded VelesDB (no HTTP hop)

The VelesDB client wraps the native PyO3 bindings — every search, upsert, and
delete call goes straight into the embedded Rust engine. No TCP socket, no JSON
serialization, no separate server process to monitor or scale.

```python
# velesdb_client.py — native bindings, single in-process database
import velesdb

class VelesDBClient:
    def __init__(self, data_path: str | None = None, **_legacy_kwargs) -> None:
        self._db = velesdb.Database(data_path or self._tmp_path())
        self._collection = None  # opened lazily after a known dimension is set
```

VelesDB releases the GIL during every Rust call, so the FastAPI event loop
stays responsive even when search and ingestion run concurrently.

### Embedding Model

- **Model**: `paraphrase-multilingual-MiniLM-L12-v2`
- **Languages**: 50+ (including French, English, German, etc.)
- **Dimensions**: 384
- **Size**: ~120MB
- **Source**: [Hugging Face](https://huggingface.co/sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2)

## 📝 License

MIT - Free for any use.
