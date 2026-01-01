# VelesDB Installation Guide

Complete installation instructions for all platforms and deployment methods.

## 📦 Available Packages

| Platform | Format | Download |
|----------|--------|----------|
| **Windows** | `.msi` installer | [GitHub Releases](https://github.com/cyberlife-coder/VelesDB/releases) |
| **Linux** | `.deb` package | [GitHub Releases](https://github.com/cyberlife-coder/VelesDB/releases) |
| **Windows** | `.zip` portable | [GitHub Releases](https://github.com/cyberlife-coder/VelesDB/releases) |
| **Linux** | `.tar.gz` portable | [GitHub Releases](https://github.com/cyberlife-coder/VelesDB/releases) |
| **Python** | `pip` | [PyPI](https://pypi.org/project/velesdb/) ✅ |
| **Rust** | `cargo` | [crates.io](https://crates.io/crates/velesdb-core) ✅ |
| **npm** | WASM/SDK | [npm @wiscale](https://www.npmjs.com/org/wiscale) ✅ |
| **Docker** | Container | [ghcr.io](https://ghcr.io/cyberlife-coder/velesdb) |
| **iOS** | XCFramework | [Build from source](#-mobile-iosandroid) |
| **Android** | AAR/SO | [Build from source](#-mobile-iosandroid) |

---

## 🪟 Windows Installation

### MSI Installer (Recommended)

The MSI installer provides the easiest installation experience with:
- **VelesDB Server** (`velesdb-server.exe`) - REST API server
- **VelesDB CLI** (`velesdb.exe`) - Command-line interface with REPL
- **Documentation** - Architecture, benchmarks, API docs
- **Examples** - Tauri RAG application example
- **PATH Integration** - Optional system PATH modification

#### Interactive Install

1. Download `velesdb-x.x.x-x86_64.msi` from [Releases](https://github.com/cyberlife-coder/VelesDB/releases)
2. Double-click to run the installer
3. Select features:
   - ✅ **Binaries** (required)
   - ✅ **Documentation** (recommended)
   - ✅ **Examples** (recommended)
   - ✅ **Add to PATH** (recommended)
4. Complete installation

#### Silent Install

```powershell
# Install with PATH modification (default)
msiexec /i velesdb-0.6.0-x86_64.msi /quiet ADDTOPATH=1

# Install without PATH modification
msiexec /i velesdb-0.6.0-x86_64.msi /quiet ADDTOPATH=0

# Install to custom directory
msiexec /i velesdb-0.6.0-x86_64.msi /quiet APPLICATIONFOLDER="D:\VelesDB"
```

#### Uninstall

Via **Control Panel > Programs > Uninstall**, or:

```powershell
msiexec /x velesdb-0.6.0-x86_64.msi /quiet
```

### Portable ZIP

For portable installations without admin rights:

```powershell
# Download and extract
Invoke-WebRequest -Uri "https://github.com/cyberlife-coder/VelesDB/releases/download/v0.6.0/velesdb-windows-x86_64.zip" -OutFile velesdb.zip
Expand-Archive velesdb.zip -DestinationPath C:\VelesDB

# Add to PATH (optional, current session only)
$env:PATH += ";C:\VelesDB"

# Or add permanently via System Properties > Environment Variables
```

---

## 🐧 Linux Installation

### DEB Package (Debian/Ubuntu)

```bash
# Download
wget https://github.com/cyberlife-coder/VelesDB/releases/download/v0.6.0/velesdb-0.6.0-amd64.deb

# Install
sudo dpkg -i velesdb-0.6.0-amd64.deb

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
wget https://github.com/cyberlife-coder/VelesDB/releases/download/v0.6.0/velesdb-linux-x86_64.tar.gz
tar -xzf velesdb-linux-x86_64.tar.gz -C /opt/velesdb

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
results = collection.search(query_vector, top_k=10)
```

---

## 🦀 Rust Installation

### As Library

```toml
# Cargo.toml
[dependencies]
velesdb-core = "0.3"
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

```bash
# Run with persistent data
docker run -d \
  --name velesdb \
  -p 8080:8080 \
  -v velesdb_data:/data \
  ghcr.io/cyberlife-coder/velesdb:latest

# With custom data directory
docker run -d \
  -p 8080:8080 \
  -v /path/to/data:/data \
  ghcr.io/cyberlife-coder/velesdb:latest
```

### Docker Compose

```yaml
version: '3.8'
services:
  velesdb:
    image: ghcr.io/cyberlife-coder/velesdb:latest
    ports:
      - "8080:8080"
    volumes:
      - velesdb_data:/data
    environment:
      - RUST_LOG=info
    restart: unless-stopped

volumes:
  velesdb_data:
```

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
store.insert(1, new Float32Array([...]));
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
  -h, --host <HOST>       Host address [default: 0.0.0.0]
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
<data-dir>/
├── collections/           # Collection metadata
├── <collection-name>/
│   ├── index.hnsw        # HNSW vector index
│   ├── storage.bin       # Vector data
│   ├── payloads.bin      # Metadata/payloads
│   └── wal.bin           # Write-Ahead Log
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

Ensure you're using a volume:
```bash
docker run -v velesdb_data:/data ghcr.io/cyberlife-coder/velesdb:latest
```

---

## 📚 Next Steps

- **[Quick Start](../README.md#-your-first-vector-search)** - Your first vector search
- **[VelesQL Guide](VELESQL_SPEC.md)** - SQL-like query language
- **[API Reference](API_REFERENCE.md)** - REST API documentation
- **[Benchmarks](BENCHMARKS.md)** - Performance metrics
- **[Examples](../examples/)** - Sample applications including Tauri RAG app
