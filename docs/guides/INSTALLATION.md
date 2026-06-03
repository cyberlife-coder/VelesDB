# VelesDB Installation Guide

Complete installation instructions for all platforms and deployment methods.

## 📦 Available Packages

| Platform | Format | Download |
|----------|--------|----------|
| **Windows** | `.zip` portable (`velesdb-x86_64-pc-windows-msvc.zip`) | [GitHub Releases](https://github.com/cyberlife-coder/VelesDB/releases) ✅ |
| **Linux (amd64)** | `.deb` package | [GitHub Releases](https://github.com/cyberlife-coder/VelesDB/releases) ✅ |
| **Linux (amd64)** | `.tar.gz` portable (`velesdb-x86_64-unknown-linux-gnu.tar.gz`) | [GitHub Releases](https://github.com/cyberlife-coder/VelesDB/releases) ✅ |
| **macOS (Intel)** | `.tar.gz` portable (`velesdb-x86_64-apple-darwin.tar.gz`) | [GitHub Releases](https://github.com/cyberlife-coder/VelesDB/releases) ✅ |
| **macOS (Apple Silicon)** | `.tar.gz` portable (`velesdb-aarch64-apple-darwin.tar.gz`) | [GitHub Releases](https://github.com/cyberlife-coder/VelesDB/releases) ✅ |
| **Python** | `pip` | [PyPI](https://pypi.org/project/velesdb/) ✅ |
| **Rust** | `cargo` | [crates.io](https://crates.io/crates/velesdb-core) ✅ |
| **npm** | WASM/SDK | [npm @wiscale](https://www.npmjs.com/org/wiscale) ✅ |
| **Docker** | Container | [Build from source](#-docker-installation) |
| **iOS** | XCFramework | [Build from source](#-mobile-iosandroid) |
| **Android** | AAR/SO | [Build from source](#-mobile-iosandroid) |

> **Note:** A signed MSI Windows installer is on the roadmap but **not yet available**. Until it ships, use the portable `.zip` archive below — it contains the same `velesdb-server.exe` and `velesdb.exe` binaries.

---

## 🪟 Windows Installation

The current Windows release ships as a **portable `.zip` archive** containing the
two binaries (`velesdb-server.exe`, `velesdb.exe`). A signed MSI installer is on
the roadmap but not yet available; use the steps below in the meantime.

### Portable ZIP (current method)

```powershell
# 1. Download
Invoke-WebRequest -Uri "https://github.com/cyberlife-coder/VelesDB/releases/latest/download/velesdb-x86_64-pc-windows-msvc.zip" -OutFile velesdb.zip

# 2. Extract anywhere you have write access (no admin required)
Expand-Archive velesdb.zip -DestinationPath C:\VelesDB

# 3. (Optional) Add to PATH for the current session
$env:PATH += ";C:\VelesDB"

# 4. (Optional) Add permanently via System Properties > Environment Variables,
#    or for the current user:
[Environment]::SetEnvironmentVariable("PATH", "$env:PATH;C:\VelesDB", "User")

# 5. Verify
velesdb --version
velesdb-server --version
```

To uninstall, simply remove the `C:\VelesDB` folder and (if added) the `PATH`
entry.

---

## 🐧 Linux Installation

### DEB Package (Debian/Ubuntu)

```bash
# Download
wget https://github.com/cyberlife-coder/VelesDB/releases/download/v1.16.0/velesdb-1.16.0-amd64.deb

# Install
sudo dpkg -i velesdb-1.16.0-amd64.deb

# Verify
velesdb --version
velesdb-server --version
```

**Installed locations:**
- `/usr/bin/velesdb` - CLI with REPL
- `/usr/bin/velesdb-server` - REST API server
- `/usr/share/doc/velesdb/` - Documentation and examples

#### Uninstall

```bash
sudo dpkg -r velesdb
```

### Portable Tarball

```bash
# Download and extract
wget https://github.com/cyberlife-coder/VelesDB/releases/latest/download/velesdb-x86_64-unknown-linux-gnu.tar.gz
sudo mkdir -p /opt/velesdb
sudo tar -xzf velesdb-x86_64-unknown-linux-gnu.tar.gz -C /opt/velesdb

# Add to PATH
echo 'export PATH=$PATH:/opt/velesdb' >> ~/.bashrc
source ~/.bashrc
```

### One-liner Script

```bash
curl -fsSL https://raw.githubusercontent.com/cyberlife-coder/VelesDB/main/scripts/install.sh | bash
```

---

## 🐍 Python Installation

```bash
pip install velesdb
```

**Usage:**
```python
import velesdb

# Open or create database
db = velesdb.Database("./my_vectors")

# Create collection
collection = db.create_collection("documents", dimension=768, metric="cosine")

# Insert vectors
collection.upsert([
    {"id": 1, "vector": [...], "payload": {"title": "Hello World"}}
])

# Search
results = collection.search_request(velesdb.SearchOptions(vector=query_vector, top_k=10))
```

---

## 🦀 Rust Installation

### As Library

```toml
# Cargo.toml
[dependencies]
velesdb-core = "1.14"
```

### As CLI Tools

```bash
# Install CLI (includes REPL)
cargo install velesdb-cli

# Install Server
cargo install velesdb-server
```

---

## 🐳 Docker Installation

### Pre-built image (recommended)

Each release publishes a multi-architecture image (linux/amd64 + linux/arm64) to the
GitHub Container Registry, so you don't need to build locally:

```bash
# Pull a specific release (recommended for reproducibility)
docker pull ghcr.io/cyberlife-coder/velesdb:1.16.0

# ...or the latest stable release
docker pull ghcr.io/cyberlife-coder/velesdb:latest

docker run -d --name velesdb -p 8080:8080 -v velesdb_data:/data \
  ghcr.io/cyberlife-coder/velesdb:1.16.0
```

### Build locally

```bash
# Clone and build the image locally
git clone https://github.com/cyberlife-coder/VelesDB.git && cd VelesDB
docker build -t velesdb .

# Run with persistent data (named volume)
docker run -d \
  --name velesdb \
  -p 8080:8080 \
  -v velesdb_data:/data \
  velesdb

# With a custom host directory for data
docker run -d \
  --name velesdb \
  -p 8080:8080 \
  -v /path/to/data:/data \
  velesdb

# Verify it's running
curl http://localhost:8080/health
```

The container runs as a non-root `velesdb` user. Data is stored in `/data` inside the container and persists across restarts via the named volume. A built-in health check polls `GET /health` every 30 seconds.

### Docker Compose

From the repository root:

```bash
docker-compose up -d
```

This uses the `docker-compose.yml` included in the repository, which builds the image locally and configures persistent storage with auto-restart:

```yaml
version: '3.8'
services:
  velesdb:
    build: .
    container_name: velesdb
    ports:
      - "8080:8080"
    volumes:
      - velesdb_data:/data
    environment:
      - RUST_LOG=info
      - VELESDB_DATA_DIR=/data
    restart: unless-stopped
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:8080/health"]
      interval: 30s
      timeout: 3s
      retries: 3
      start_period: 5s

volumes:
  velesdb_data:
    driver: local
```

### Environment Variables

| Variable | Default | Description |
|---|---|---|
| `VELESDB_DATA_DIR` | `/data` | Data storage directory |
| `VELESDB_HOST` | `0.0.0.0` | Bind address |
| `VELESDB_PORT` | `8080` | HTTP port |
| `RUST_LOG` | `info` | Log level (`debug`, `info`, `warn`, `error`) |

---

## 🌐 WASM / Browser

```bash
# WASM module for browser
npm install @wiscale/velesdb-wasm

# Full TypeScript SDK
npm install @wiscale/velesdb-sdk

# Tauri plugin bindings
npm install @wiscale/tauri-plugin-velesdb
```

```javascript
import init, { VectorStore } from '@wiscale/velesdb-wasm';

await init();
const store = new VectorStore(768, 'cosine');
store.insert(1n, new Float32Array([...]));
const results = store.search(new Float32Array([...]), 10);
```

---

## 📱 Mobile (iOS/Android)

VelesDB provides native mobile bindings via UniFFI.

### Prerequisites

```bash
# iOS targets
rustup target add aarch64-apple-ios        # Device
rustup target add aarch64-apple-ios-sim    # Simulator (ARM)
rustup target add x86_64-apple-ios         # Simulator (Intel)

# Android targets
rustup target add aarch64-linux-android    # ARM64
rustup target add armv7-linux-androideabi  # ARMv7
rustup target add x86_64-linux-android     # x86_64

# Android NDK tool
cargo install cargo-ndk
```

### Build for iOS

```bash
# Build static library
cargo build --release --target aarch64-apple-ios -p velesdb-mobile

# Generate Swift bindings
cargo run --bin uniffi-bindgen generate \
    --library target/aarch64-apple-ios/release/libvelesdb_mobile.a \
    --language swift \
    --out-dir bindings/swift
```

### Build for Android

```bash
# Build shared libraries for all ABIs
cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 \
    build --release -p velesdb-mobile

# Generate Kotlin bindings
cargo run --bin uniffi-bindgen generate \
    --library target/aarch64-linux-android/release/libvelesdb_mobile.so \
    --language kotlin \
    --out-dir bindings/kotlin
```

### Usage (Swift)

```swift
import VelesDB

let db = try VelesDatabase.open(path: documentsPath + "/velesdb")
try db.createCollection(name: "docs", dimension: 384, metric: .cosine)

let collection = try db.getCollection(name: "docs")!
let results = try collection.search(vector: embedding, limit: 10)
```

### Usage (Kotlin)

```kotlin
val db = VelesDatabase.open("${context.filesDir}/velesdb")
db.createCollection("docs", 384u, DistanceMetric.COSINE)

val collection = db.getCollection("docs")!!
val results = collection.search(embedding, 10u)
```

📖 Full guide: [crates/velesdb-mobile/README.md](../crates/velesdb-mobile/README.md)

---

## ⚙️ Configuration

### Server Configuration

```bash
velesdb-server [OPTIONS]

Options:
  -d, --data-dir <PATH>   Data directory [default: ./data]
      --host <HOST>       Host address [default: 127.0.0.1]
  -p, --port <PORT>       Port number [default: 8080]
```

**Environment variables:**
- `VELESDB_DATA_DIR` - Data directory path
- `VELESDB_HOST` - Bind address
- `VELESDB_PORT` - Port number
- `RUST_LOG` - Logging level (debug, info, warn, error)

### Data Persistence

VelesDB persists all data to disk automatically:

```
<data_dir>/<collection_name>/
├── config.json       # Collection config (dimension, metric, HNSW params)
├── vectors.bin       # mmap-backed vector data
├── vectors.idx       # ID → offset index
├── vectors.wal       # Vector WAL
├── payloads.log      # Append-only payload WAL
├── payloads.snapshot # Optional snapshot
└── hnsw.bin          # HNSW graph index
```

**Data is persistent by default.** Restart the server and your data will be there.

---

## 🔧 Troubleshooting

### Windows: "Command not found"

Ensure VelesDB is in your PATH:
```powershell
# Check PATH
$env:PATH -split ';' | Select-String VelesDB

# Add manually if missing
$env:PATH += ";C:\Program Files\VelesDB\bin"
```

### Linux: Permission denied

```bash
# Make binaries executable
chmod +x /usr/bin/velesdb /usr/bin/velesdb-server
```

### Port already in use

```bash
# Use different port
velesdb-server --port 8081

# Or find and kill existing process
lsof -i :8080
kill <PID>
```

### Docker: Data not persisting

Ensure you're using a named volume:
```bash
docker run -v velesdb_data:/data velesdb
```

---

## 📚 Next Steps

- **[Quick Start](../README.md#-your-first-vector-search)** - Your first vector search
- **[VelesQL Guide](../VELESQL_SPEC.md)** - SQL-like query language
- **[API Reference](../reference/api-reference.md)** - REST API documentation
- **[Benchmarks](../BENCHMARKS.md)** - Performance metrics
- **[Examples](../examples/)** - Sample applications including Tauri RAG app
