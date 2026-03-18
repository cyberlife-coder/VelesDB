//! Shared GPU helpers for wgpu buffer readback and bind-group layout creation.
//!
//! RF-DEDUP: Eliminates duplicated GPU boilerplate between `gpu_backend.rs`
//! and `pq_gpu.rs`.

/// Creates a standard 4-entry bind group layout for compute shaders.
///
/// Layout:
/// - binding 0: `storage(read_only)` — input A (e.g., query or vectors)
/// - binding 1: `storage(read_only)` — input B (e.g., vectors or centroids)
/// - binding 2: `storage(read_write)` — output (e.g., results or assignments)
/// - binding 3: `uniform` — params (e.g., dimension, count)
pub(super) fn create_quad_bind_group_layout(
    device: &wgpu::Device,
    label: &str,
) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(label),
        entries: &[
            storage_layout_entry(0, true),
            storage_layout_entry(1, true),
            storage_layout_entry(2, false),
            uniform_layout_entry(3),
        ],
    })
}

/// Creates a bind group entry for a storage buffer.
const fn storage_layout_entry(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

/// Creates a bind group entry for a uniform buffer.
const fn uniform_layout_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

/// Reads back a GPU buffer synchronously via map-async + poll + channel.
///
/// Returns `None` if the map-async operation fails.
///
/// # Arguments
///
/// * `device` — wgpu device used to poll for completion
/// * `staging_buffer` — buffer with `MAP_READ | COPY_DST` usage
/// * `size` — number of bytes to read
pub(super) fn readback_buffer<T: bytemuck::Pod>(
    device: &wgpu::Device,
    staging_buffer: &wgpu::Buffer,
    count: usize,
) -> Option<Vec<T>> {
    let buffer_slice = staging_buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = tx.send(result);
    });
    device.poll(wgpu::Maintain::Wait);

    rx.recv().ok().and_then(Result::ok)?;

    let data = buffer_slice.get_mapped_range();
    let typed: &[T] = bytemuck::cast_slice(&data);
    let result = typed[..count].to_vec();
    drop(data);
    staging_buffer.unmap();

    Some(result)
}

/// Flattens a slice of `Vec<f32>` into a contiguous `Vec<f32>`.
///
/// Used by GPU kernels that require flat data layout.
pub(super) fn flatten_vecs(vecs: &[Vec<f32>], expected_dim: usize) -> Vec<f32> {
    let mut flat = Vec::with_capacity(vecs.len() * expected_dim);
    for v in vecs {
        flat.extend_from_slice(v);
    }
    flat
}
