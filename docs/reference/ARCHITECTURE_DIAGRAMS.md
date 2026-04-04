# VelesDB Architecture Diagrams — v1.11.1

## 1. Workspace Dependency Graph

```mermaid
graph TD
    CORE[velesdb-core<br/>Engine Library]
    SERVER[velesdb-server<br/>REST API - Axum]
    CLI[velesdb-cli<br/>CLI + REPL]
    PYTHON[velesdb-python<br/>PyO3 Bindings]
    WASM[velesdb-wasm<br/>Browser WASM]
    MOBILE[velesdb-mobile<br/>UniFFI iOS/Android]
    MIGRATE[velesdb-migrate<br/>Migration Tool]
    TAURI[tauri-plugin-velesdb<br/>Desktop Plugin]

    SERVER --> CORE
    CLI --> CORE
    PYTHON --> CORE
    WASM -->|default-features=false| CORE
    MOBILE --> CORE
    MIGRATE --> CORE
    TAURI --> CORE

    style CORE fill:#2d5016,stroke:#4a8c2a,color:#fff
    style SERVER fill:#1a3a5c,stroke:#2980b9,color:#fff
    style CLI fill:#1a3a5c,stroke:#2980b9,color:#fff
    style PYTHON fill:#4a2d6e,stroke:#8e44ad,color:#fff
    style WASM fill:#6e4a2d,stroke:#d35400,color:#fff
    style MOBILE fill:#6e4a2d,stroke:#d35400,color:#fff
    style MIGRATE fill:#1a3a5c,stroke:#2980b9,color:#fff
    style TAURI fill:#1a3a5c,stroke:#2980b9,color:#fff
```

## 2. velesdb-core Internal Architecture

```mermaid
graph TD
    subgraph "Public API Layer"
        DB[Database]
        VC[VectorCollection]
        GC[GraphCollection]
        MC[MetadataCollection]
    end

    subgraph "Query Engine"
        PARSER[VelesQL Parser<br/>pest grammar]
        PLANNER[Query Planner<br/>CBO]
        CACHE[Plan Cache<br/>L1+L2 LRU]
        EXEC[Query Executor]
    end

    subgraph "Index Layer"
        HNSW[HNSW Index<br/>Native impl]
        BM25[BM25 Index<br/>Full-text]
        SEC[Secondary Index<br/>B-tree metadata]
        SPARSE[Sparse Index<br/>Inverted]
        PROP[Property Index<br/>Graph]
        RANGE[Range Index<br/>Graph]
    end

    subgraph "SIMD Kernels"
        AVX512[AVX-512]
        AVX2[AVX2]
        NEON[ARM NEON]
        SCALAR[Scalar fallback]
        DISPATCH[Runtime Dispatch]
    end

    subgraph "Storage Layer"
        MMAP[mmap Storage]
        WAL[WAL + Compaction]
        SNAP[Snapshots]
        COLSTORE[Column Store]
    end

    subgraph "Graph Engine"
        EDGE[ConcurrentEdgeStore<br/>256 shards]
        CSR[CsrSnapshot<br/>ArcSwap lock-free]
        TRAV[Traversal<br/>BFS/DFS/Parallel]
        STREAM[Streaming BFS<br/>Parent-pointer]
    end

    subgraph "Quantization"
        SQ8[SQ8 - 4x]
        BIN[Binary - 32x]
        PQ[Product Quant]
        RABITQ[RaBitQ]
    end

    DB --> VC
    DB --> GC
    DB --> MC
    VC --> HNSW
    VC --> BM25
    VC --> SEC
    VC --> SPARSE
    GC --> EDGE
    GC --> CSR
    GC --> TRAV
    GC --> PROP
    GC --> RANGE
    HNSW --> DISPATCH
    DISPATCH --> AVX512
    DISPATCH --> AVX2
    DISPATCH --> NEON
    DISPATCH --> SCALAR
    HNSW --> MMAP
    HNSW --> SQ8
    HNSW --> BIN
    HNSW --> PQ
    HNSW --> RABITQ
    DB --> PARSER
    PARSER --> PLANNER
    PLANNER --> CACHE
    CACHE --> EXEC
    MMAP --> WAL
    MMAP --> SNAP
    MC --> COLSTORE

    style DB fill:#2d5016,stroke:#4a8c2a,color:#fff
    style HNSW fill:#8b0000,stroke:#ff4444,color:#fff
    style CSR fill:#8b0000,stroke:#ff4444,color:#fff
    style DISPATCH fill:#8b0000,stroke:#ff4444,color:#fff
```

## 3. HNSW Search Pipeline (Hot Path)

```mermaid
flowchart LR
    Q[Query Vector] --> VAL[Validate Dimension]
    VAL --> QUAL{Quality Mode?}
    
    QUAL -->|Perfect| BF[Brute Force SIMD]
    QUAL -->|≤100 vectors| BF
    QUAL -->|Adaptive| ADAPT[Two-Phase Adaptive]
    QUAL -->|AutoTune| AUTO[Auto EF Range]
    QUAL -->|Fast/Balanced/Accurate| STD[Standard Path]
    
    STD --> EF[Compute ef_search]
    EF --> RERANK{Two-Stage?}
    RERANK -->|Yes| HNSW_R[HNSW Search<br/>rerank_k candidates]
    RERANK -->|No| HNSW_K[HNSW Search<br/>k results]
    
    HNSW_R --> GPU{GPU Available?}
    GPU -->|Yes + large batch| GPU_RR[GPU Rerank<br/>wgpu batch distance]
    GPU -->|No| SIMD_RR[SIMD Rerank<br/>ContiguousVectors<br/>+ prefetch]
    
    GPU_RR --> SORT[Sort + Truncate k]
    SIMD_RR --> SORT
    HNSW_K --> SORT
    BF --> SORT
    ADAPT --> SORT
    AUTO --> SORT
    
    SORT --> EMA[Update Latency EMA]
    EMA --> RES[Results]

    style BF fill:#8b0000,color:#fff
    style SIMD_RR fill:#8b0000,color:#fff
    style GPU_RR fill:#4a2d6e,color:#fff
```

## 4. Graph Traversal Pipeline

```mermaid
flowchart TD
    REQ[Traversal Request] --> CFG[Build TraversalConfig<br/>depth, limit, rel_types]
    CFG --> SNAP[Acquire CsrSnapshot<br/>ArcSwap::load - lock-free]
    
    SNAP --> TYPE{Algorithm?}
    TYPE -->|BFS| BFS[BFS with FxHashSet<br/>visited set]
    TYPE -->|DFS| DFS[DFS with stack]
    TYPE -->|Parallel BFS| PBFS[Multi-source BFS<br/>dedup by path signature]
    
    BFS --> PRED{EdgePredicate?}
    DFS --> PRED
    PBFS --> PRED
    
    PRED -->|Label filter| LABEL[Label pushdown<br/>290ns filtered BFS]
    PRED -->|No filter| FULL[Full traversal<br/>3.4µs unfiltered]
    
    LABEL --> PATH[Parent-pointer<br/>path reconstruction<br/>eliminates cloning]
    FULL --> PATH
    
    PATH --> LIMIT[Apply limit + min_depth]
    LIMIT --> RES[TraversalResult<br/>target_id, depth, path]

    style SNAP fill:#8b0000,color:#fff
    style PATH fill:#8b0000,color:#fff
```

## 5. Platform Target Matrix

```mermaid
graph LR
    subgraph "Targets"
        WIN[Windows x86_64<br/>AVX2/AVX-512]
        LINUX[Linux x86_64<br/>AVX2/AVX-512]
        MAC_X[macOS x86_64<br/>AVX2]
        MAC_A[macOS aarch64<br/>NEON]
        IOS[iOS aarch64<br/>NEON]
        ANDROID[Android<br/>arm64/armv7/x86_64]
        BROWSER[Browser<br/>WASM SIMD128]
    end

    subgraph "Crates"
        C_CORE[velesdb-core]
        C_SERVER[velesdb-server]
        C_CLI[velesdb-cli]
        C_PYTHON[velesdb-python]
        C_WASM[velesdb-wasm]
        C_MOBILE[velesdb-mobile]
        C_TAURI[tauri-plugin]
    end

    WIN --- C_CORE
    WIN --- C_SERVER
    WIN --- C_CLI
    WIN --- C_PYTHON
    WIN --- C_TAURI
    LINUX --- C_CORE
    LINUX --- C_SERVER
    LINUX --- C_CLI
    LINUX --- C_PYTHON
    MAC_X --- C_CORE
    MAC_X --- C_SERVER
    MAC_X --- C_CLI
    MAC_X --- C_PYTHON
    MAC_A --- C_CORE
    MAC_A --- C_SERVER
    MAC_A --- C_CLI
    MAC_A --- C_PYTHON
    IOS --- C_MOBILE
    ANDROID --- C_MOBILE
    BROWSER --- C_WASM

    style C_CORE fill:#2d5016,stroke:#4a8c2a,color:#fff
    style C_WASM fill:#6e4a2d,stroke:#d35400,color:#fff
    style C_MOBILE fill:#6e4a2d,stroke:#d35400,color:#fff
```

## 6. Feature Propagation Matrix (v1.11.1)

```mermaid
graph TD
    subgraph "Core Features"
        F_VEC[Vector Search kNN]
        F_HYB[Hybrid Search]
        F_TXT[Text Search BM25]
        F_FILT[Filtered Search]
        F_BATCH[Batch Search]
        F_MQ[Multi-Query Fusion]
        F_SPARSE[Sparse Vectors]
        F_GRAPH[Graph Edges]
        F_TRAV[Graph Traversal]
        F_PTRAV[Parallel BFS]
        F_GSEARCH[Graph Search]
        F_VELESQL[VelesQL]
        F_IDX[Secondary Indexes]
        F_AGENT[Agent Memory]
        F_QUANT[Quantization]
        F_STREAM[Streaming Insert]
    end

    subgraph "Propagation"
        P_SERVER[server ✅ all]
        P_CLI[cli ✅ all]
        P_PYTHON[python ✅ all]
        P_WASM[wasm ⚠️ no persistence]
        P_MOBILE[mobile ✅ most]
        P_TAURI[tauri ✅ all]
        P_TS[ts-sdk ✅ via REST]
        P_LC[langchain ✅ RAG]
        P_LI[llamaindex ✅ RAG]
    end

    F_VEC --> P_SERVER
    F_VEC --> P_CLI
    F_VEC --> P_PYTHON
    F_VEC --> P_WASM
    F_VEC --> P_MOBILE
    F_VEC --> P_TAURI
    F_VEC --> P_TS
    F_VEC --> P_LC
    F_VEC --> P_LI

    style P_WASM fill:#6e4a2d,stroke:#d35400,color:#fff
```

## 7. NLOC Health Map (v1.11.1 — Post-Refactoring)

| Severity | Count | Files |
|----------|-------|-------|
| ✅ Compliant (<500) | ALL | All 39 previously non-compliant production files refactored to <500 NLOC |
| 🔵 Exempt (SIMD) | 2 | avx512.rs (1294), neon.rs (774) — hand-written SIMD kernels, exempt by design |

### Refactoring Summary (NLOC/CC Resolution Plan)
- P1: Collection god-object migration — 11 files, deprecated HashMap removed
- P2: Critical extractions — mobile/lib.rs 936→264, python/lib.rs 739→98, CLI 8MB→168B, lifecycle.rs 984→490, edge.rs 972→349, backend_adapter.rs 849→191, wasm/lib.rs 743→134
- P3: Parser CC fix, Tauri invoke handler macro, ~30 minor file extractions
- Total: 4675 tests (4447 lib + 228 BDD), 0 failures

### SIMD files (avx512.rs=1294, neon.rs=774) are exempt
These are hand-written SIMD kernels — splitting them would break instruction scheduling and cache locality. They are performance-critical hot paths that were specifically optimized. Codacy should be configured to exclude `simd_native/*.rs` from NLOC checks.

## 8. Quality Gate Status

| Gate | Status | Details |
|------|--------|---------|
| `cargo fmt --all --check` | ✅ | Zero diffs |
| `cargo clippy --workspace -- -D warnings` | ✅ | Zero warnings, 8 crates |
| `cargo check --workspace` | ✅ | All 8 crates compile |
| Production NLOC < 500 | ✅ | All 39 files refactored — 0 violations (2 SIMD exempt) |
| CC ≤ 8 | ✅ | Codacy gate — 0 issues |
| Tests | ✅ | 4675 tests pass (4447 lib + 228 BDD) |
| Recall ≥ 0.95 | ✅ | Contract tests pass |
