# VelesDB Configuration Reference

> Complete configuration guide for VelesDB server and library.

## Configuration File

VelesDB uses TOML format for configuration. Default location: `velesdb.toml` in the data directory.

```toml
# velesdb.toml - Full configuration example

[search]
default_mode = "balanced"    # fast, balanced, accurate, high_recall, perfect
ef_search = 128              # Override ef_search (optional)
max_results = 1000           # Maximum results per query
query_timeout_ms = 30000     # Query timeout in milliseconds

[hnsw]
m = 32                       # Connections per node (auto if not set)
ef_construction = 400        # Candidate pool during construction
max_layers = 0               # Maximum layers (0 = auto)

[storage]
data_dir = "./data"          # Data directory path
storage_mode = "mmap"        # mmap, memory, disk
mmap_cache_mb = 1024         # Memory-mapped cache size

[limits]
max_dimensions = 4096        # Maximum vector dimensions
max_perfect_mode_vectors = 50000  # Max vectors for perfect mode

[server]
host = "127.0.0.1"           # Bind address
port = 8080                  # HTTP port
workers = 4                  # Worker threads (0 = auto)

[logging]
level = "info"               # trace, debug, info, warn, error
format = "pretty"            # pretty, json
```

## Configuration Sections

### [search] - Search Parameters

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `default_mode` | string | `balanced` | Default search mode preset |
| `ef_search` | integer | (from mode) | Override ef_search value |
| `max_results` | integer | 1000 | Maximum results per query |
| `query_timeout_ms` | integer | 30000 | Query timeout in milliseconds |

### [hnsw] - HNSW Index Parameters

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `m` | integer | auto | Connections per node (higher = better recall, more memory) |
| `ef_construction` | integer | auto | Candidate pool during index build |
| `max_layers` | integer | 0 (auto) | Maximum HNSW layers |

**Recommended M values:**
- **16**: Low memory, ~95% recall
- **32**: Balanced (default for most use cases)
- **48**: High recall, higher memory
- **64**: Maximum recall, highest memory

### [storage] - Storage Configuration

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `data_dir` | string | `./data` | Data directory path |
| `storage_mode` | string | `mmap` | Storage backend mode |
| `mmap_cache_mb` | integer | 1024 | Memory-mapped cache size in MB |

**Storage modes:**
- `mmap`: Memory-mapped files (recommended for production)
- `memory`: In-memory only (fastest, no persistence)
- `disk`: Direct disk I/O (slower, lower memory usage)

### [limits] - Resource Limits

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `max_dimensions` | integer | 4096 | Maximum allowed vector dimensions |
| `max_perfect_mode_vectors` | integer | 50000 | Maximum vectors for perfect mode |

### [server] - HTTP Server

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `host` | string | `127.0.0.1` | Bind address |
| `port` | integer | 8080 | HTTP port |
| `workers` | integer | 0 (auto) | Worker threads |

### [logging] - Logging

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `level` | string | `info` | Log level |
| `format` | string | `pretty` | Output format (`pretty` or `json`) |

## Environment Variables

Configuration can be overridden via environment variables:

| Variable | Description | Example |
|----------|-------------|---------|
| `VELESDB_DATA_DIR` | Data directory | `/var/lib/velesdb` |
| `VELESDB_PORT` | HTTP port | `9090` |
| `VELESDB_HOST` | Bind address | `0.0.0.0` |
| `VELESDB_LOG_LEVEL` | Log level | `debug` |
| `VELESDB_SEARCH_MODE` | Default search mode | `accurate` |
| `VELESDB_LICENSE_PUBLIC_KEY` | License validation key | `MCowBQY...` |

## Configuration Priority

1. Environment variables (highest priority)
2. Configuration file
3. Default values (lowest priority)

## Production Configuration

```toml
# Recommended production settings

[search]
default_mode = "balanced"
max_results = 1000
query_timeout_ms = 30000

[hnsw]
m = 32
ef_construction = 400

[storage]
data_dir = "/var/lib/velesdb"
storage_mode = "mmap"
mmap_cache_mb = 4096

[server]
host = "0.0.0.0"
port = 8080
workers = 0  # Auto-detect CPU cores

[logging]
level = "info"
format = "json"
```

## Memory Estimation

Approximate memory usage per vector:

| Dimensions | Storage Mode | Memory per Vector |
|------------|--------------|-------------------|
| 384 | Full (f32) | ~1.5 KB |
| 768 | Full (f32) | ~3 KB |
| 1536 | Full (f32) | ~6 KB |
| 768 | SQ8 | ~0.8 KB |
| 768 | Binary | ~0.1 KB |

**Total memory formula:**
```
Total = vectors × (dims × bytes_per_dim + overhead)
```

Where:
- `bytes_per_dim`: 4 (full), 1 (SQ8), 0.125 (binary)
- `overhead`: ~100 bytes for HNSW graph per vector

## License

ELv2 (Elastic License 2.0)
