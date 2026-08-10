# velesdb (Python)

> Embedded vector + graph database for Python: local-first semantic search and explainable agent memory.

[![PyPI](https://img.shields.io/pypi/v/velesdb)](https://pypi.org/project/velesdb/)
[![Python](https://img.shields.io/pypi/pyversions/velesdb)](https://pypi.org/project/velesdb/)
[![License](https://img.shields.io/badge/license-VelesDB_Core_1.0-blue)](./LICENSE)

Licensed under the [VelesDB Core License 1.0](./LICENSE) (source-available). The
compiled wheel embeds the VelesDB engine and is governed by the same license.

## Objective

Vector search usually means running a server: a container, a port, a network
hop on every query, and an ops story you did not ask for. VelesDB's Python SDK
removes all of it — the engine is compiled into the wheel and runs inside your
process, against a directory on disk. You get microsecond-scale similarity
search, hybrid dense + sparse retrieval, graphs and VelesQL without a daemon,
and — when you are building an agent — a memory layer that can explain *why* it
returned what it returned.

If you already run a managed vector service and are happy with it, you do not
have this problem and can stop here.

## Use cases

- A RAG prototype on a laptop that must survive `pip install` and nothing else — no Docker, no cloud account.
- An AI agent that has to remember decisions across process restarts and justify them later (`why()`).
- A desktop or CLI application shipping semantic search inside the app, with the index living next to the user's data.
- A batch job that embeds a corpus once, writes a portable index directory, and searches it in-process.
- An offline or air-gapped environment where sending embeddings to a hosted API is not an option.

## Prerequisites

| Requirement | Minimum version | Note |
|---|---|---|
| Python | 3.9 | `requires-python = ">=3.9"`; a single `cp39-abi3` wheel covers 3.9+ |
| pip | any recent | prebuilt wheels, no compilation |
| NumPy | 1.20 | hard runtime dependency, installed automatically |
| Rust | 1.90 | **only** when building from the sdist / source checkout |
| Embedding model | — | not included: VelesDB stores and searches vectors, it does not generate them |

## Installation

```bash
pip install velesdb
```

Optional extras (all independent, install only what you use):

```bash
pip install "velesdb[embed-sentence-transformers]"  # local embedding adapter
pip install "velesdb[embed-openai]"                 # OpenAI-compatible adapter
pip install "velesdb[pandas]"                       # DataFrame ingestion
pip install "velesdb[polars]"                       # Polars ingestion
```

Building from a source checkout of this repository instead:

```bash
pip install maturin
cd crates/velesdb-python
maturin develop
```

## First success in 60 seconds

```python
# pip install velesdb
import velesdb

db = velesdb.Database("./hello_velesdb_data")                 # created if missing
docs = db.get_or_create_collection("docs", metric="cosine")   # dimension auto-detected

# 4-D vectors whose axes stand for four made-up topics: [tech, food, music, sport]
docs.upsert([
    {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0], "payload": {"title": "Rust release notes"}},
    {"id": 2, "vector": [0.0, 1.0, 0.0, 0.0], "payload": {"title": "Best ramen in Tokyo"}},
    {"id": 3, "vector": [0.6, 0.0, 0.8, 0.0], "payload": {"title": "AI-generated jazz"}},
])

results = docs.search_request(velesdb.SearchOptions(vector=[1.0, 0.0, 0.0, 0.0], top_k=2))
for r in results:
    print(f"score={r['score']:.3f}  {r['payload']['title']}")
```

Expected output — the exact-match document scores 1.000, the partly-tech one 0.600:

```text
score=1.000  Rust release notes
score=0.600  AI-generated jazz
```

Anything else is a failure: an empty output means the upsert did not land (check
that `./hello_velesdb_data` is writable), and a `ModuleNotFoundError` means the
wheel is not installed in the interpreter you are running. The longer version of
this script is
[`examples/python/hello_velesdb.py`](../../examples/python/hello_velesdb.py).

Next step, the agent-memory wedge — the same package, no extra install:

```python
from velesdb import MemoryService              # offline, deterministic, no API key

mem = MemoryService("./agent_memory")          # on-disk store; survives restarts
reason = mem.remember("Robert is recovering from knee surgery")
mem.remember("Booked the aisle seat on Robert's flight", links=[(reason, "because")])

mem.why("why the aisle seat on Robert's flight?")   # walks booking → reason
```

`why()` returns the best-matching memory **plus the connected subgraph** reached
through typed links — context that shares no words with the question, which a
plain vector recall cannot find. See
[PYTHON_AGENT_MEMORY.md](../../docs/guides/PYTHON_AGENT_MEMORY.md).

## Configuration

`Database(path, config=...)` accepts a typed `VelesConfigOptions` covering every
engine section of the core `VelesConfig`. Build it in code or load it from a
`velesdb.toml` (engine-only semantics: a shell-owned `[server]` / `[logging]`
table in a shared file is ignored).

| Section | Type | Controls |
|---|---|---|
| `limits` | `LimitsOptions` | collection and resource ceilings |
| `search` | `SearchConfigOptions` | default search mode, max results |
| `hnsw` | `HnswConfigOptions` | index build/search parameters |
| `storage` | `StorageOptions` | on-disk storage behaviour |
| `quantization` | `QuantizationOptions` | compression settings |

```python
from velesdb import Database, VelesConfigOptions, LimitsOptions, SearchConfigOptions

cfg = VelesConfigOptions(
    limits=LimitsOptions(max_collections=50),
    search=SearchConfigOptions(default_mode="accurate", max_results=100),
)
db = Database("./tenant1", config=cfg)

# Or from TOML (fail-fast: invalid TOML/values raise ValueError,
# a missing file raises FileNotFoundError):
cfg = VelesConfigOptions.from_toml_path("./velesdb.toml")
db = Database("./tenant1", config=cfg)
```

`wal_batch` is intentionally not exposed — the concurrent WAL writer is a
VelesDB Enterprise feature (see
[WRITE_CONCURRENCY.md](../../docs/guides/WRITE_CONCURRENCY.md)).

## Examples

Runnable scripts, not snippets: [`examples/python/`](../../examples/python/)
(`hello_velesdb.py`, `hybrid_queries.py`, `fusion_strategies.py`,
`graph_traversal.py`, `graphrag_langchain.py`, `graphrag_llamaindex.py`,
`multimodel_notebook.py`) and the agent-memory demos in
[`examples/agent_memory/`](../../examples/agent_memory/).

## API / commands

Signatures and docstrings ship inside the wheel as a typed stub
([`python/velesdb/__init__.pyi`](python/velesdb/__init__.pyi), with `py.typed`),
so your IDE and mypy/pyright are the reference. Task-oriented guides:

| Guide | What it covers |
|---|---|
| [PYTHON_API_REFERENCE.md](../../docs/guides/PYTHON_API_REFERENCE.md) | `Database` / `Collection`, sparse + hybrid search, fusion strategies, distance metrics, storage modes, bulk loading, streaming ingestion |
| [PYTHON_AGENT_MEMORY.md](../../docs/guides/PYTHON_AGENT_MEMORY.md) | `MemoryService` (`remember` / `recall` / `why` / `feedback`) and the semantic / episodic / procedural SDK |
| [PYTHON_CONTEXT_COMPILER.md](../../docs/guides/PYTHON_CONTEXT_COMPILER.md) | `compile_context`, provenance handles, working contexts, LangChain and LlamaIndex wiring |
| [PYTHON_GRAPH.md](../../docs/guides/PYTHON_GRAPH.md) | persistent graph collections, `MATCH` queries, in-memory `GraphStore` |
| [PYTHON_VELESQL.md](../../docs/guides/PYTHON_VELESQL.md) | `VelesQL.parse()` / `ParsedStatement` introspection |
| [PYTHON_RAG_PIPELINE.md](../../docs/guides/PYTHON_RAG_PIPELINE.md) | text → embeddings → results, built-in embedding adapters |
| [PYTHON_PERFORMANCE.md](../../docs/guides/PYTHON_PERFORMANCE.md) | throughput tuning (numpy `f32`, `upsert_bulk_numpy`, batching) |
| [PYTHON_ENGINE_BENCHMARKS.md](../../docs/guides/PYTHON_ENGINE_BENCHMARKS.md) | measured engine latency and recall figures |
| [PYTHON_REMOTE_SERVER.md](../../docs/guides/PYTHON_REMOTE_SERVER.md) | talking to a running `velesdb-server` over HTTP |

## Known limits

- **No embedding generation.** VelesDB stores and searches vectors; you bring the model (or use the optional adapters).
- **Embedded only.** There is no Python client class for a remote server — use HTTP against `velesdb-server`.
- **One process per database directory.** A second process opening the same path fails with `DatabaseLockedError` (`[VELES-031]`).
- **`wal_batch` / concurrent WAL writing is not exposed** — Enterprise feature.
- **No GPU in the published wheels.** The `gpu` Cargo feature exists but is not enabled by `[tool.maturin]`; it requires building from source.
- **`Collection.search(...)` is deprecated** since v1.15 (emits `DeprecationWarning`); use `search_request(SearchOptions(...))`.
- **`Collection.get_graph_store()` returns a standalone in-memory graph** that is not connected to the collection; use `Database.create_graph_collection()` for persistence.

## Compatibility

Prebuilt wheels published to PyPI (single `cp39-abi3` wheel per platform, so one
wheel covers Python 3.9 and later):

| Platform | Status | Note |
|---|---|---|
| Linux x86_64 (glibc) | Prebuilt wheel | manylinux2014 — glibc 2.17+ |
| Linux aarch64 (glibc) | Prebuilt wheel | manylinux2014 |
| Linux x86_64 (musl) | Prebuilt wheel | musllinux_1_2 — Alpine images |
| Linux aarch64 (musl) | Prebuilt wheel | musllinux_1_2 |
| macOS arm64 + x86_64 | Prebuilt wheel | single `universal2` wheel |
| Windows x64 | Prebuilt wheel | MSVC |
| Windows arm64 | Prebuilt wheel | `aarch64-pc-windows-msvc` |
| Anything else (PyPy, exotic arches, older glibc) | Source build | sdist fallback, needs Rust 1.90 |

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `ModuleNotFoundError: No module named 'velesdb'` | the compiled extension is not installed in the active interpreter (a source checkout is not importable as-is) | `pip install velesdb`, or `maturin develop` inside `crates/velesdb-python` |
| `DimensionMismatchError: ... expected 768 ... 512` | the vector length differs from the collection's dimension | re-embed with the model that matches the collection, or create the collection with `dimension=None` to auto-detect on first upsert |
| `DatabaseLockedError: [VELES-031] Database is already opened by another process` | another process (or a still-open handle, e.g. a notebook kernel) holds that directory | close the other handle, or give the second process its own directory |
| `RuntimeError` on `collection.stream_insert([...])` | streaming ingestion was never enabled | call `collection.enable_streaming(...)` first |
| `FileNotFoundError` from `VelesConfigOptions.from_toml_path(...)` | config loading is fail-fast by design | check the path; malformed TOML or invalid values raise `ValueError` instead |

All typed exceptions (`DimensionMismatchError`, `CollectionNotFoundError`,
`CollectionExistsError`, `EdgeExistsError`, `DatabaseLockedError`,
`VelesQLSyntaxError`, `VelesQLParameterError`) derive from `velesdb.VelesDBError`,
so `except velesdb.VelesDBError` is a safe catch-all.

---

`velesdb-python v5.0.0` · Last updated: 2026-08-10 · Applies to: velesdb-core 5.0.0 · [Report a docs error](https://github.com/cyberlife-coder/VelesDB/issues)
