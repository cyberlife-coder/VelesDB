# Testing VelesDB-Migrate with Real Data

This guide explains how to test `velesdb-migrate` against your real data.

## Configuration

### Required environment variables

```powershell
# Supabase - Required
$env:SUPABASE_URL = "https://YOUR_PROJECT.supabase.co"
$env:SUPABASE_SERVICE_KEY = "your-service-role-key"
$env:SUPABASE_TABLE = "your_table_name"

# Optional - Column names (defaults shown)
$env:SUPABASE_VECTOR_COL = "embedding"
$env:SUPABASE_ID_COL = "id"
```

## Integration tests

### Running the tests with real data

```powershell
# From the workspace root
cd /path/to/velesdb-core

# Deterministic local E2E tests against velesdb-core (JSON/CSV + checkpoint + workers)
cargo test -p velesdb-migrate --test pipeline_e2e

# All integration tests
cargo test -p velesdb-migrate --test integration_test -- --ignored --nocapture

# A specific test
cargo test -p velesdb-migrate --test integration_test test_supabase_connection -- --ignored --nocapture
cargo test -p velesdb-migrate --test integration_test test_dimension_detection_accuracy -- --ignored --nocapture
```

### Available tests

### Local E2E tests

| Test | Description |
|------|-------------|
| `pipeline_e2e` | Validates real writes into `velesdb-core`, checkpoint resume, `dry_run`, `continue_on_error` and `workers` consistency |

### Tests against real sources

| Test | Description |
|------|-------------|
| `test_supabase_connection` | Verifies connection and schema detection |
| `test_supabase_extract_batch` | Extracts one batch of vectors |
| `test_supabase_extract_and_validate_batch` | Extracts a batch and validates every vector |
| `test_dimension_detection_accuracy` | Verifies dimension-detection accuracy |

## Benchmarks

### Running the benchmarks

```powershell
# Local benchmarks (no network access)
cargo bench -p velesdb-migrate

# Against real Supabase data (requires env vars)
$env:SUPABASE_URL = "https://..."
$env:SUPABASE_SERVICE_KEY = "..."
cargo bench -p velesdb-migrate
```

### Available benchmarks

| Benchmark | Description |
|-----------|-------------|
| `parse_pgvector_1536d` | Parsing a 1536-dimension pgvector string |
| `pgvector_parse_by_dimension` | Parsing across dimensions (384-3072) |
| `vector_normalize_1536d` | Vector normalization |
| `vector_dot_product_1536d` | Dot product |
| `process_batch_100x1536d` | Processing a batch of 100 vectors |
| `batch_size_impact` | Impact of batch size (10-1000) |
| `serialize_payload` / `deserialize_payload` | Payload (de)serialization |
| `supabase_schema_detection` | Schema detection (network) |
| `supabase_batch_extraction` | Batch extraction (network) |

### Inspecting the results

```powershell
# Results land in target/criterion/
# Open the HTML report
start target\criterion\report\index.html
```

## Full test script

### Usage

```powershell
# Set the variables
$env:SUPABASE_URL = "https://YOUR_PROJECT.supabase.co"
$env:SUPABASE_SERVICE_KEY = "your-service-role-key"

# Run the test script
.\crates\velesdb-migrate\scripts\test-with-real-data.ps1 -All

# Or individual options
.\crates\velesdb-migrate\scripts\test-with-real-data.ps1 -IntegrationTests
.\crates\velesdb-migrate\scripts\test-with-real-data.ps1 -Benchmarks
.\crates\velesdb-migrate\scripts\test-with-real-data.ps1 -FullMigration
```

## Example expected results

### Supabase connection test

```
✅ Connected to Supabase!
   Collection: your_table_name
   Dimension: 1536
   Total count: Some(10000)
   Fields: 8
```

### pgvector parsing benchmark

```
parse_pgvector_1536d    time:   [150.32 µs 151.45 µs 152.67 µs]

pgvector_parse_by_dimension/dimension/384
                        time:   [38.21 µs 38.56 µs 38.93 µs]
pgvector_parse_by_dimension/dimension/768
                        time:   [76.45 µs 77.12 µs 77.84 µs]
pgvector_parse_by_dimension/dimension/1536
                        time:   [152.34 µs 153.21 µs 154.12 µs]
```

### Supabase extraction benchmark

```
supabase_schema_detection
                        time:   [245.3 ms 267.8 ms 291.2 ms]

supabase_batch_extraction/batch_size/10
                        time:   [312.5 ms 334.2 ms 356.8 ms]
supabase_batch_extraction/batch_size/100
                        time:   [456.7 ms 489.3 ms 523.1 ms]
```

## Debugging

### Verbose output

```powershell
# Add RUST_LOG for more detail
$env:RUST_LOG = "debug"
cargo test -p velesdb-migrate --test integration_test -- --ignored --nocapture
```

### Checking the connection manually

```powershell
# Test with detect
.\target\release\velesdb-migrate.exe detect `
    --source supabase `
    --url $env:SUPABASE_URL `
    --collection $env:SUPABASE_TABLE `
    --api-key $env:SUPABASE_SERVICE_KEY `
    --output test.yaml
```

## Pre-release checklist

- [ ] Unit tests pass: `cargo test -p velesdb-migrate`
- [ ] Integration tests pass against real data
- [ ] Benchmarks run and results documented
- [ ] Dimension detection works for every source
- [ ] Full migration tested end to end
