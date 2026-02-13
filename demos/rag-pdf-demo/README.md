# VelesDB RAG Demo - PDF Question Answering

A complete RAG (Retrieval-Augmented Generation) demo using **VelesDB** for vector storage, with PDF document ingestion and semantic search.

## 🎯 Features

- **PDF Upload & Processing** - Extract text from PDF documents using PyMuPDF
- **Automatic Chunking** - Split documents into optimal chunks (512 chars, 50 overlap)
- **Multilingual Embeddings** - Uses `paraphrase-multilingual-MiniLM-L12-v2` (50+ languages)
- **VelesDB Storage** - Ultra-fast vector search with HNSW algorithm
- **Semantic Search** - Find relevant passages with cosine similarity
- **Real-time Metrics** - Performance timing displayed in UI
- **REST API** - Simple API for integration

## 🏗️ Architecture

```
┌──────────────┐     ┌──────────────┐     ┌──────────────┐
│   Frontend   │────▶│   FastAPI    │────▶│   VelesDB    │
│  (Upload UI) │     │   Backend    │     │   Server     │
└──────────────┘     └──────────────┘     └──────────────┘
                            │
                     ┌──────┴──────┐
                     │             │
              ┌──────▼──────┐ ┌────▼─────┐
              │   PyMuPDF   │ │ Sentence │
              │ (PDF Parse) │ │Transformers│
              └─────────────┘ └──────────┘
```

## 🚀 Quick Start

### Prerequisites

1. **VelesDB Server** running on `localhost:8080`:
   ```bash
   velesdb-server --data-dir ./rag-data
   ```
   > **Note:** If the server is started with `VELESDB_API_KEY` set, all requests must include
   > `Authorization: Bearer <key>` header. This demo does not send auth headers, so ensure
   > `VELESDB_API_KEY` is **not** set when running the demo.

2. **Python 3.10+**

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

2. **Check if servers are running:**
   ```bash
   # Check VelesDB Server (port 8080)
   curl http://127.0.0.1:8080
   
   # Check FastAPI Demo (port 8000)
   curl http://127.0.0.1:8000/health
   ```

3. **Verify ports are not blocked:**
   ```bash
   netstat -ano | findstr ":8000"
   netstat -ano | findstr ":8080"
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
VELESDB_URL=http://localhost:8080
EMBEDDING_MODEL=paraphrase-multilingual-MiniLM-L12-v2
EMBEDDING_DIMENSION=384
CHUNK_SIZE=512
CHUNK_OVERLAP=50
```

| Parameter | Default Value |
|-----------|---------------|
| Embedding Model | `paraphrase-multilingual-MiniLM-L12-v2` |
| Embedding Dimensions | 384 |
| Chunk Size | 512 characters |
| Chunk Overlap | 50 characters |
| Distance Metric | Cosine similarity |

## 📊 Performance Benchmarks

Benchmarks measured on Windows 11, Python 3.10, VelesDB 0.3.7 (500 iterations):

### Layer-by-Layer Latency

| Layer | Mean | P95 | StdDev | Stability |
|-------|------|-----|--------|-----------|
| TCP Connection | 0.29ms | 0.51ms | 0.16ms | Variable |
| HTTP Client Creation | 6.41ms | 7.35ms | 2.20ms | Avoid (use persistent) |
| **HTTP Request (persistent)** | **0.61ms** | 0.81ms | 0.09ms | ✅ Stable |
| **VelesDB Search (sync)** | **0.89ms** | 1.45ms | 0.23ms | ✅ Good |
| VelesDB Search (async) | 2.65ms | 3.17ms | 0.28ms | ✅ Stable |
| **Full API Search** | **19.10ms** | 24.68ms | 5.80ms | Acceptable |

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
| **VelesDB (this demo)** | **0.89ms** | REST API, HNSW, persistent client |
| VelesDB (native Rust) | <1ms | Direct integration |
| Pinecone | ~50-100ms | Cloud service |
| Qdrant | ~10-50ms | Self-hosted |
| FAISS | ~1ms | In-memory only |

> **Note**: Run `python benchmark_latency.py` to measure performance on your hardware.

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
│   ├── velesdb_client.py # VelesDB REST client (persistent conn)
│   └── rag_engine.py     # RAG orchestration with timing
├── tests/
│   ├── __init__.py
│   ├── conftest.py       # Fixtures
│   ├── test_pdf_processor.py
│   ├── test_embeddings.py
│   ├── test_velesdb_client.py
│   └── test_rag_engine.py
├── static/
│   └── index.html        # UI with real-time metrics
├── benchmark_latency.py  # Performance benchmarks
├── pyproject.toml
├── .env.example
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

#### Option 1: Delete Collection (Recommended)
```bash
# Delete the entire collection from VelesDB
curl -X DELETE "http://127.0.0.1:8080/collections/rag_documents"

# Or restart with fresh data directory
.\target\release\velesdb-server.exe --data-dir ./rag-data-fresh
```

#### Option 2: Manual File Cleanup
```bash
# Stop VelesDB Server first (Ctrl+C)
# Then delete the data directory
Remove-Item .\rag-data -Recurse -Force

# Restart for fresh start
.\target\release\velesdb-server.exe --data-dir ./rag-data
```

#### Option 3: Individual Document Deletion
```bash
# Via web interface: Click trash icon next to document
# Or via API:
curl -X POST "http://127.0.0.1:8000/documents/delete" \
  -H "Content-Type: application/json" \
  -d '{"document_name": "your-document.pdf"}'
```

### 📊 Storage Usage
- **PDF documents**: Not stored (only processed)
- **Text chunks**: Stored in VelesDB as payloads
- **Embeddings**: 384 dimensions × 4 bytes = ~1.5KB per chunk
- **Index overhead**: ~20-30% additional storage

### 🔍 Check Current Storage
```bash
# Check collection size
curl "http://127.0.0.1:8080/collections/rag_documents"

# Check disk usage
dir .\rag-data\rag_documents
```

## 🔬 Technical Details

### HTTP Client Optimization

The VelesDB client uses a **persistent HTTP connection** to avoid the ~2s overhead of creating a new `httpx.AsyncClient` on each request (DNS resolution, TCP handshake).

```python
# velesdb_client.py - Singleton pattern
class VelesDBClient:
    _client: httpx.AsyncClient | None = None
    
    async def _get_client(self):
        if self._client is None:
            self._client = httpx.AsyncClient(base_url=self.base_url)
        return self._client
```

### Embedding Model

- **Model**: `paraphrase-multilingual-MiniLM-L12-v2`
- **Languages**: 50+ (including French, English, German, etc.)
- **Dimensions**: 384
- **Size**: ~120MB
- **Source**: [Hugging Face](https://huggingface.co/sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2)

## 📝 License

MIT - Free for any use.
