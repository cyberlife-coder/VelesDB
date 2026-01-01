# VelesDB Mobile

Native bindings for **iOS** (Swift) and **Android** (Kotlin) via [UniFFI](https://mozilla.github.io/uniffi-rs/).

VelesDB Mobile brings microsecond vector search to edge devices - perfect for on-device AI, semantic search, and RAG applications.

## Features

- **Native Performance**: Direct Rust bindings, no FFI overhead
- **Binary Quantization**: 32x memory reduction for constrained devices
- **ARM NEON SIMD**: Optimized for mobile processors (Apple A-series, Snapdragon)
- **Offline-First**: Full functionality without network connectivity
- **Thread-Safe**: Safe to use from multiple threads/queues

## Quick Start

### Swift (iOS)

```swift
import VelesDB

// Open database
let db = try VelesDatabase.open(path: documentsPath + "/velesdb")

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

// Open database
val db = VelesDatabase.open("${context.filesDir}/velesdb")

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
| `open(path)` | Opens or creates a database at the specified path |
| `createCollection(name, dimension, metric)` | Creates a new vector collection |
| `createCollectionWithStorage(name, dimension, metric, storageMode)` | Creates collection with IoT storage optimization |
| `getCollection(name)` | Gets a collection by name (returns nil/null if not found) |
| `listCollections()` | Lists all collection names |
| `deleteCollection(name)` | Deletes a collection |

### VelesCollection

| Method | Description |
|--------|-------------|
| `search(vector, limit)` | Finds k nearest neighbors |
| `upsert(point)` | Inserts or updates a single point |
| `upsertBatch(points)` | Batch insert/update (faster for bulk operations) |
| `delete(id)` | Deletes a point by ID |
| `count()` | Returns the number of points |
| `dimension()` | Returns the vector dimension |

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

VelesDB is licensed under the [Elastic License 2.0 (ELv2)](../../LICENSE).

This is a **source available** license that allows free use for most purposes,
with restrictions on providing VelesDB as a managed service (DBaaS).
