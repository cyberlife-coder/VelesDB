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

# Open browser
start http://localhost:8000
```

## 📖 API Endpoints

### Upload PDF
```bash
curl -X POST "http://localhost:8000/documents/upload" \
  -F "file=@document.pdf"
```

### Search Documents
```bash
curl -X POST "http://localhost:8000/search" \
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
curl "http://localhost:8000/documents"
```

### Health Check
```bash
curl "http://localhost:8000/health"
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

Benchmarks measured on Windows 11, Python 3.10, VelesDB 0.3.7:

### Search Latency (after warm-up)

| Component | Latency | Description |
|-----------|---------|-------------|
| **Query Embedding** | ~12ms | sentence-transformers encode |
| **VelesDB Search** | ~5ms | HNSW vector search via REST API |
| **Total Search** | ~20ms | End-to-end query response |

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
| Subsequent Searches | - | ~20ms |
| Model Loading | ~2-3s | Cached |

### Comparison with Other Solutions

| Solution | Search Latency | Notes |
|----------|---------------|-------|
| **VelesDB (this demo)** | ~5ms | REST API, HNSW |
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
