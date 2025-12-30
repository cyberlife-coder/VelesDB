# VelesDB Installation Guide

Complete installation instructions for all platforms and deployment methods.

## 📦 Available Packages

| Platform | Format | Download |
|----------|--------|----------|
| **Windows** | `.msi` installer | [GitHub Releases](https://github.com/cyberlife-coder/VelesDB/releases) |
| **Linux** | `.deb` package | [GitHub Releases](https://github.com/cyberlife-coder/VelesDB/releases) |
| **Windows** | `.zip` portable | [GitHub Releases](https://github.com/cyberlife-coder/VelesDB/releases) |
| **Linux** | `.tar.gz` portable | [GitHub Releases](https://github.com/cyberlife-coder/VelesDB/releases) |
| **Python** | `pip` | [PyPI](https://pypi.org/project/velesdb/) |
| **Rust** | `cargo` | [crates.io](https://crates.io/crates/velesdb-core) |
| **npm** | WASM | [npm](https://www.npmjs.com/package/velesdb-wasm) |
| **Docker** | Container | [ghcr.io](https://ghcr.io/cyberlife-coder/velesdb) |

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
msiexec /i velesdb-0.5.1-x86_64.msi /quiet ADDTOPATH=1

# Install without PATH modification
msiexec /i velesdb-0.5.1-x86_64.msi /quiet ADDTOPATH=0

# Install to custom directory
msiexec /i velesdb-0.5.1-x86_64.msi /quiet APPLICATIONFOLDER="D:\VelesDB"
```

#### Uninstall

Via **Control Panel > Programs > Uninstall**, or:

```powershell
msiexec /x velesdb-0.5.1-x86_64.msi /quiet
```

### Portable ZIP

For portable installations without admin rights:

```powershell
# Download and extract
Invoke-WebRequest -Uri "https://github.com/cyberlife-coder/VelesDB/releases/download/v0.5.1/velesdb-windows-x86_64.zip" -OutFile velesdb.zip
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
wget https://github.com/cyberlife-coder/VelesDB/releases/download/v0.5.1/velesdb-0.5.1-amd64.deb

# Install
sudo dpkg -i velesdb-0.5.1-amd64.deb

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
wget https://github.com/cyberlife-coder/VelesDB/releases/download/v0.5.1/velesdb-linux-x86_64.tar.gz
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
npm install velesdb-wasm
```

```javascript
import init, { VectorStore } from 'velesdb-wasm';

await init();
const store = new VectorStore(768, 'cosine');
store.insert(1, new Float32Array([...]));
const results = store.search(new Float32Array([...]), 10);
```

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
