# VelesDB Mobile

Native bindings for **iOS** (Swift) and **Android** (Kotlin) via [UniFFI](https://mozilla.github.io/uniffi-rs/).

VelesDB Mobile brings microsecond vector search to edge devices - perfect for on-device AI, semantic search, and RAG applications.

## Features

- **Native Performance**: Direct Rust bindings, minimal FFI overhead
- **Multi-Query Fusion**: Native MQG with RRF/Weighted strategies
- **Binary Quantization**: 32x memory reduction for constrained devices
- **ARM NEON SIMD**: Optimized for mobile processors (Apple A-series, Snapdragon)
- **Offline-First**: Full functionality without network connectivity
- **Thread-Safe**: Safe to use from multiple threads/queues

## Quick Start

### Swift (iOS)

```swift
import VelesDB

// Open database (UniFFI constructor)
let db = try VelesDatabase(path: documentsPath + "/velesdb")

// Create collection (768D for MiniLM, 384D for all-MiniLM-L6-v2)
try db.createCollection(name: "documents", dimension: 384, metric: .cosine)

// Get collection
guard let collection = try db.getCollection(name: "documents") else {
    fatalError("Collection not found")
}

// Insert vectors
let point = VelesPoint(
    id: 1,
    vector: embedding,  // [Float] from your embedding model
    payload: "{\"title\": \"Hello World\"}"
)
try collection.upsert(point: point)

// Search
let results = try collection.search(vector: queryEmbedding, limit: 10)
for result in results {
    print("ID: \(result.id), Score: \(result.score)")
}
```

### Kotlin (Android)

```kotlin
import com.velesdb.mobile.*

// Open database (UniFFI constructor)
val db = VelesDatabase("${context.filesDir}/velesdb")

// Create collection
db.createCollection("documents", 384u, DistanceMetric.COSINE)

// Get collection
val collection = db.getCollection("documents")
    ?: throw Exception("Collection not found")

// Insert vectors
val point = VelesPoint(
    id = 1uL,
    vector = embedding,  // List<Float> from your embedding model
    payload = """{"title": "Hello World"}"""
)
collection.upsert(point)

// Search (use Dispatchers.IO for async)
val results = withContext(Dispatchers.IO) {
    collection.search(queryEmbedding, 10u)
}
results.forEach { result ->
    println("ID: ${result.id}, Score: ${result.score}")
}
```

## Build Instructions

### Prerequisites

```bash
# Install Rust targets
rustup target add aarch64-apple-ios        # iOS device
rustup target add aarch64-apple-ios-sim    # iOS simulator (ARM)
rustup target add x86_64-apple-ios         # iOS simulator (Intel)

rustup target add aarch64-linux-android    # Android ARM64
rustup target add armv7-linux-androideabi  # Android ARMv7
rustup target add x86_64-linux-android     # Android x86_64

# For Android: Install cargo-ndk
cargo install cargo-ndk
```

### iOS Build

```bash
# Build for device
cargo build --release --target aarch64-apple-ios -p velesdb-mobile

# Build for simulator
cargo build --release --target aarch64-apple-ios-sim -p velesdb-mobile

# Generate Swift bindings
cargo run --bin uniffi-bindgen generate \
    --library target/aarch64-apple-ios/release/libvelesdb_mobile.a \
    --language swift \
    --out-dir bindings/swift

# Create XCFramework (requires macOS)
xcodebuild -create-xcframework \
    -library target/aarch64-apple-ios/release/libvelesdb_mobile.a \
    -headers bindings/swift \
    -library target/aarch64-apple-ios-sim/release/libvelesdb_mobile.a \
    -headers bindings/swift \
    -output VelesDB.xcframework
```

### Android Build

```bash
# Build for all Android ABIs
cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 \
    build --release -p velesdb-mobile

# Generate Kotlin bindings
cargo run --bin uniffi-bindgen generate \
    --library target/aarch64-linux-android/release/libvelesdb_mobile.so \
    --language kotlin \
    --out-dir bindings/kotlin

# Libraries are in:
# - target/aarch64-linux-android/release/libvelesdb_mobile.so
# - target/armv7-linux-androideabi/release/libvelesdb_mobile.so
# - target/x86_64-linux-android/release/libvelesdb_mobile.so
```

## API Reference

### VelesDatabase

| Method | Description |
|--------|-------------|
| `VelesDatabase(path)` | Opens or creates a database at the specified path (constructor) |
| `createCollection(name, dimension, metric)` | Creates a new vector collection |
| `createCollectionWithStorage(name, dimension, metric, storageMode)` | Creates collection with quantized storage |
| `createMetadataCollection(name)` | Creates a metadata-only collection (no vectors) |
| `getCollection(name)` | Gets a collection by name (returns nil/null if not found) |
| `listCollections()` | Lists all collection names |
| `deleteCollection(name)` | Deletes a collection |
| `trainPq(collectionName, config)` | Trains Product Quantization on a collection |

### VelesCollection

| Method | Description |
|--------|-------------|
| `search(vector, limit)` | Finds k nearest neighbors |
| `searchWithFilter(vector, limit, filterJson)` | Search with metadata filter |
| `multiQuerySearch(vectors, limit, strategy)` | Multi-query fusion (MQG) |
| `multiQuerySearchWithFilter(vectors, limit, strategy, filterJson)` | Multi-query fusion with metadata filter |
| `textSearch(query, limit)` | BM25 full-text search |
| `textSearchWithFilter(query, limit, filterJson)` | Text search with filter |
| `hybridSearch(vector, textQuery, limit, vectorWeight)` | Combined vector + text search |
| `hybridSearchWithFilter(vector, textQuery, limit, vectorWeight, filterJson)` | Hybrid search with metadata filter |
| `batchSearch(searches)` | Batch search with individual filters per query |
| `sparseSearch(sparseVector, limit, indexName)` | Sparse-only search using inverted index |
| `hybridSparseSearch(vector, sparseVector, limit, indexName)` | Hybrid dense + sparse search with RRF fusion |
| `query(queryStr, paramsJson)` | Execute VelesQL query |
| `upsert(point)` | Inserts or updates a single point |
| `upsertBatch(points)` | Batch insert/update (faster for bulk operations) |
| `upsertWithSparse(point, sparseVector)` | Inserts a point with an associated sparse vector |
| `delete(id)` | Deletes a point by ID |
| `get(ids)` | Gets points by their IDs (missing IDs silently skipped) |
| `getById(id)` | Gets a single point by ID (returns nil/null if not found) |
| `count()` | Returns the number of points |
| `dimension()` | Returns the vector dimension |
| `isMetadataOnly()` | Checks if this is a metadata-only collection |
| `allIds()` | Returns all point IDs in the collection |
| `flush()` | Flushes data to durable storage |
| `createIndex(fieldName)` | Creates a secondary metadata index |
| `hasSecondaryIndex(fieldName)` | Checks if a secondary index exists |
| `createPropertyIndex(label, property)` | Creates a graph/property index |
| `createRangeIndex(label, property)` | Creates a graph/range index |
| `hasPropertyIndex(label, property)` | Checks if a property index exists |
| `hasRangeIndex(label, property)` | Checks if a range index exists |
| `listIndexes()` | Lists all index definitions |
| `dropIndex(label, property)` | Drops an index |
| `indexesMemoryUsage()` | Returns memory used by indexes (bytes) |
| `analyze()` | Runs ANALYZE and returns fresh statistics |
| `getStats()` | Returns the latest statistics snapshot |

### VelesSemanticMemory

Agent memory for on-device AI. Stores knowledge facts as vectors with similarity search.

| Method | Description |
|--------|-------------|
| `VelesSemanticMemory(db, dimension)` | Creates semantic memory with the given embedding dimension (constructor) |
| `store(id, content, embedding)` | Stores a knowledge fact with its embedding |
| `query(embedding, topK)` | Queries by similarity, returns `SemanticResult` list |
| `remove(id)` | Removes a knowledge fact by ID |
| `clear()` | Clears all knowledge facts |
| `len()` | Returns the number of stored facts |
| `isEmpty()` | Returns true if no facts are stored |
| `dimension()` | Returns the embedding dimension |

### MobileGraphStore

In-memory graph store for mobile knowledge graphs.

| Method | Description |
|--------|-------------|
| `MobileGraphStore()` | Creates a new empty graph store (constructor) |
| `addNode(node)` | Adds a node to the graph |
| `addEdge(edge)` | Adds an edge (returns error if duplicate ID) |
| `getNode(id)` | Gets a node by ID |
| `getEdge(id)` | Gets an edge by ID |
| `hasNode(id)` | Checks if a node exists |
| `hasEdge(id)` | Checks if an edge exists |
| `nodeCount()` | Returns the number of nodes |
| `edgeCount()` | Returns the number of edges |
| `getOutgoing(nodeId)` | Gets outgoing edges from a node |
| `getIncoming(nodeId)` | Gets incoming edges to a node |
| `getOutgoingByLabel(nodeId, label)` | Gets outgoing edges filtered by label |
| `getNeighbors(nodeId)` | Gets neighbor node IDs (1-hop) |
| `getNodesByLabel(label)` | Gets all nodes with a specific label |
| `getEdgesByLabel(label)` | Gets all edges with a specific label |
| `outDegree(nodeId)` | Returns the out-degree of a node |
| `inDegree(nodeId)` | Returns the in-degree of a node |
| `bfsTraverse(sourceId, maxDepth, limit)` | Breadth-first traversal |
| `bfsTraverseParallel(sourceIds, maxDepth, limit)` | Multi-source parallel BFS with deduplication |
| `dfsTraverse(sourceId, maxDepth, limit)` | Depth-first traversal |
| `removeNode(nodeId)` | Removes a node and all connected edges |
| `removeEdge(edgeId)` | Removes an edge by ID |
| `clear()` | Clears all nodes and edges |

### Distance Metrics

| Metric | Description | Use Case |
|--------|-------------|----------|
| `Cosine` | Cosine similarity (1 - cosine_distance) | Text embeddings, normalized vectors |
| `Euclidean` | L2 distance | Image features, unnormalized vectors |
| `DotProduct` | Dot product | Pre-normalized vectors, MaxSim |
| `Hamming` | Hamming distance for binary vectors | Binary embeddings, LSH |
| `Jaccard` | Jaccard similarity for sets | Sparse vectors, tags |

### Storage Modes (IoT/Edge)

| Mode | Compression | Memory/dim | Recall Loss | Use Case |
|------|-------------|------------|-------------|----------|
| `Full` | 1x | 4 bytes | 0% | Best quality |
| `Sq8` | 4x | 1 byte | ~1% | **Recommended for mobile** |
| `Binary` | 32x | 1 bit | ~5-10% | Extreme constraints (IoT) |

```swift
// iOS - Create collection with SQ8 compression (4x memory reduction)
try db.createCollectionWithStorage(
    name: "embeddings",
    dimension: 384,
    metric: .cosine,
    storageMode: .sq8  // 4x less memory, ~1% recall loss
)
```

```kotlin
// Android - Binary quantization for IoT devices (32x compression)
db.createCollectionWithStorage(
    "embeddings", 384u, DistanceMetric.COSINE, StorageMode.BINARY
)
```

### Fusion Strategies

Used with `multiQuerySearch()` for combining results from multiple query vectors.

| Strategy | Description |
|----------|-------------|
| `Average` | Average scores across all queries |
| `Maximum` | Take the maximum score per document |
| `Rrf(k)` | Reciprocal Rank Fusion (default k=60) |
| `Weighted(avgWeight, maxWeight, hitWeight)` | Weighted combination of avg, max, and hit ratio |

### Data Types

| Type | Fields | Description |
|------|--------|-------------|
| `VelesPoint` | `id: UInt64`, `vector: [Float]`, `payload: String?` | A point to insert |
| `SearchResult` | `id: UInt64`, `score: Float` | A search result |
| `SemanticResult` | `id: UInt64`, `score: Float`, `content: String` | Semantic memory result |
| `VelesSparseVector` | `indices: [UInt32]`, `values: [Float]` | Sparse vector (parallel arrays) |
| `IndividualSearchRequest` | `vector: [Float]`, `topK: UInt32`, `filter: String?` | Batch search request |
| `PqTrainConfig` | `m: UInt32`, `k: UInt32`, `opq: Bool` | PQ training configuration |
| `MobileGraphNode` | `id: UInt64`, `label: String`, `propertiesJson: String?`, `vector: [Float]?` | Graph node |
| `MobileGraphEdge` | `id: UInt64`, `source: UInt64`, `target: UInt64`, `label: String`, `propertiesJson: String?` | Graph edge |
| `TraversalResult` | `nodeId: UInt64`, `depth: UInt32` | BFS/DFS traversal result |
| `MobileCollectionStats` | `totalPoints`, `payloadSizeBytes`, `rowCount`, ... | Collection statistics |
| `MobileIndexInfo` | `label`, `property`, `indexType`, `cardinality`, `memoryBytes` | Index metadata |

## Performance Tips

1. **Use SQ8 or Binary Quantization** for memory-constrained devices
2. **Batch inserts** with `upsertBatch()` for 10x faster bulk loading
3. **Use `search()` on background thread** to avoid blocking UI
4. **Pre-allocate** embedding arrays to reduce allocations

## Memory Footprint

| Vectors | Dimension | Storage Mode | Memory |
|---------|-----------|--------------|--------|
| 10,000 | 384 | Full (f32) | ~15 MB |
| 10,000 | 384 | SQ8 | ~4 MB |
| 10,000 | 384 | Binary | ~0.5 MB |
| 100,000 | 768 | Full (f32) | ~300 MB |
| 100,000 | 768 | Binary | ~10 MB |

## License

MIT License (mobile bindings). The core engine (velesdb-core and velesdb-server) is under VelesDB Core License 1.0.

See [LICENSE](./LICENSE) for bindings license, [root LICENSE](../../LICENSE) for core engine.
