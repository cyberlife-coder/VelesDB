# GPU Acceleration Guide

VelesDB supports optional GPU acceleration for batch vector operations via the `gpu` feature.

## Requirements

### Hardware
- GPU with Vulkan, Metal, or DirectX 12 support
- Minimum 2GB VRAM recommended for large datasets

### Platform Support

| Platform | Backend | Notes |
|----------|---------|-------|
| Windows | DirectX 12 / Vulkan | Requires up-to-date drivers |
| macOS | Metal | macOS 10.15+ |
| Linux | Vulkan | Mesa 21.0+ or proprietary drivers |
| WebAssembly | WebGPU | Chrome 113+ / Firefox 121+ |

## Installation

Enable the `gpu` feature in your `Cargo.toml`:

```toml
[dependencies]
velesdb-core = { version = "1.1", features = ["gpu"] }
```

## Usage

### Check GPU Availability

```rust
use velesdb_core::gpu::GpuAccelerator;

if GpuAccelerator::is_available() {
    println!("GPU acceleration available!");
} else {
    println!("Falling back to CPU SIMD");
}
```

### Batch Vector Operations

```rust
use velesdb_core::gpu::GpuAccelerator;

// Create accelerator (returns None if no GPU)
if let Some(gpu) = GpuAccelerator::new() {
    let query = vec![1.0, 0.0, 0.0];
    let vectors = vec![
        1.0, 0.0, 0.0,  // Vector 1
        0.0, 1.0, 0.0,  // Vector 2
        0.5, 0.5, 0.0,  // Vector 3
    ];
    let dimension = 3;

    // Batch cosine similarity
    let similarities = gpu.batch_cosine_similarity(&vectors, &query, dimension);
    
    // Batch Euclidean distance
    let distances = gpu.batch_euclidean_distance(&vectors, &query, dimension);
    
    // Batch dot product
    let dots = gpu.batch_dot_product(&vectors, &query, dimension);
}
```

### GPU Trigram Operations

```rust
use velesdb_core::index::trigram::gpu::GpuTrigramAccelerator;

if let Ok(gpu) = GpuTrigramAccelerator::new() {
    // Batch extract trigrams from documents
    let docs = vec!["hello world", "foo bar", "test document"];
    let trigram_sets = gpu.batch_extract_trigrams(&docs);
    
    // Batch search patterns
    let patterns = vec!["hello", "test"];
    let results = gpu.batch_search(&patterns, &inverted_index);
}
```

## Performance Guidelines

### When to Use GPU

| Operation | CPU Best | GPU Best | Recommendation |
|-----------|----------|----------|----------------|
| Single query | < 10K vectors | > 100K vectors | Use CPU for small datasets |
| Batch queries | < 50K total | > 100K total | GPU shines with batching |
| Trigram search | < 100K docs | > 500K docs | GPU for massive text search |
| Trigram extraction | < 10K docs | > 50K docs | GPU for bulk indexing |

### Crossover Points

Based on benchmarks (768-dimensional vectors):

```
CPU SIMD vs GPU Crossover:
- Cosine similarity: ~50K vectors
- Euclidean distance: ~30K vectors  
- Dot product: ~40K vectors
```

### Memory Considerations

- GPU memory is limited - batch large datasets
- Each f32 takes 4 bytes VRAM
- 1M vectors × 768 dims = ~3GB VRAM

## Fallback Behavior

VelesDB automatically falls back to CPU SIMD when:
- No GPU is available
- GPU feature is not enabled
- Dataset is too small for GPU benefit

```rust
// Auto-selection based on workload
use velesdb_core::index::trigram::gpu::TrigramComputeBackend;

let backend = TrigramComputeBackend::auto_select(doc_count, pattern_count);
match backend {
    TrigramComputeBackend::CpuSimd => println!("Using CPU SIMD"),
    #[cfg(feature = "gpu")]
    TrigramComputeBackend::Gpu => println!("Using GPU"),
}
```

## Troubleshooting

### GPU Not Detected

1. Update graphics drivers
2. Verify Vulkan/Metal support: `vulkaninfo` or system profiler
3. Check wgpu backend compatibility

### Performance Issues

1. Ensure batch sizes are large enough (> 1000 vectors)
2. Monitor VRAM usage
3. Consider data layout (contiguous memory)

## API Reference

### `GpuAccelerator`

| Method | Description |
|--------|-------------|
| `new()` | Create accelerator (None if unavailable) |
| `is_available()` | Check GPU availability (cached) |
| `batch_cosine_similarity()` | Batch cosine similarities |
| `batch_euclidean_distance()` | Batch Euclidean distances |
| `batch_dot_product()` | Batch dot products |

### `GpuTrigramAccelerator`

| Method | Description |
|--------|-------------|
| `new()` | Create trigram accelerator |
| `is_available()` | Check GPU availability |
| `batch_search()` | Search multiple patterns |
| `batch_extract_trigrams()` | Extract trigrams from documents |
