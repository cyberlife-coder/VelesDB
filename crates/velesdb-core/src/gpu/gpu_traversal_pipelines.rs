//! Compute-pipeline compilation and bind-group layouts for GPU traversal.
//!
//! Extracted from `gpu_traversal.rs` to keep the orchestrator file under
//! the project's 500-line NLOC ceiling. This module exposes three
//! `compile_*_pipeline` constructors and the matching `create_*_bind_group_layout`
//! helpers; all are `pub(super)` free functions consumed exclusively by
//! [`super::gpu_traversal::GpuTraversalContext::new`].

// Doc comments here describe WGSL bindings and entry points which aren't
// Rust items; backticking every WGSL identifier would hurt readability.
#![allow(clippy::doc_markdown)]

use super::gpu_backend::shaders;

// ---------------------------------------------------------------------------
// Bind-group layout entry helpers
// ---------------------------------------------------------------------------

pub(super) const fn storage_entry(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
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

pub(super) const fn uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
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

// ---------------------------------------------------------------------------
// Bind-group layouts
// ---------------------------------------------------------------------------

pub(super) fn create_expand_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Expand BGL"),
        entries: &[
            storage_entry(0, true),  // csr_offsets
            storage_entry(1, true),  // csr_neighbors
            storage_entry(2, true),  // frontier (read)
            storage_entry(3, false), // candidates (read_write)
            storage_entry(4, false), // visited (read_write)
            storage_entry(5, false), // counters (read_write)
            uniform_entry(6),        // params
        ],
    })
}

pub(super) fn create_select_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Select BGL"),
        entries: &[
            storage_entry(0, true),  // candidate_ids
            storage_entry(1, true),  // candidate_dists
            storage_entry(2, false), // frontier_out (= frontier_b_ids)
            storage_entry(3, false), // frontier_dists (= frontier_b_dists)
            storage_entry(4, false), // counters
            uniform_entry(5),        // params
            storage_entry(6, true),  // frontier_in_ids (= frontier_a_ids, accumulator seed)
            storage_entry(7, true),  // frontier_in_dists (= frontier_a_dists)
        ],
    })
}

/// Creates a bind group layout for traversal distance shaders.
///
/// 5 bindings: query(read), vectors(read), candidate_ids(read),
/// results(read_write), params(uniform).
pub(super) fn create_traversal_distance_bind_group_layout(
    device: &wgpu::Device,
    label: &str,
) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(label),
        entries: &[
            storage_entry(0, true),  // query
            storage_entry(1, true),  // vectors
            storage_entry(2, true),  // candidate_ids (from expand pass)
            storage_entry(3, false), // results (distances)
            uniform_entry(4),        // params
        ],
    })
}

// ---------------------------------------------------------------------------
// Pipeline compilation
// ---------------------------------------------------------------------------

pub(super) fn compile_expand_pipeline(device: &wgpu::Device) -> wgpu::ComputePipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Expand Frontier Shader"),
        source: wgpu::ShaderSource::Wgsl(shaders::EXPAND_FRONTIER_SHADER.into()),
    });

    let layout = create_expand_bind_group_layout(device);
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Expand Pipeline Layout"),
        bind_group_layouts: &[&layout],
        push_constant_ranges: &[],
    });

    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("Expand Frontier Pipeline"),
        layout: Some(&pipeline_layout),
        module: &shader,
        entry_point: Some("expand_frontier"),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    })
}

/// Compiles a traversal-specific distance pipeline with 5 bindings
/// (query, vectors, candidate_ids, results, params).
pub(super) fn compile_traversal_distance_pipeline(
    device: &wgpu::Device,
    shader_source: &str,
    entry_point: &str,
    label: &str,
) -> wgpu::ComputePipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(&format!("{label} Shader")),
        source: wgpu::ShaderSource::Wgsl(shader_source.into()),
    });

    let layout = create_traversal_distance_bind_group_layout(device, &format!("{label} BGL"));
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(&format!("{label} PL")),
        bind_group_layouts: &[&layout],
        push_constant_ranges: &[],
    });

    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(&format!("{label} Pipeline")),
        layout: Some(&pipeline_layout),
        module: &shader,
        entry_point: Some(entry_point),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    })
}

pub(super) fn compile_select_pipeline(device: &wgpu::Device) -> wgpu::ComputePipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Select TopK Shader"),
        source: wgpu::ShaderSource::Wgsl(shaders::SELECT_TOPK_SHADER.into()),
    });

    let layout = create_select_bind_group_layout(device);
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Select Pipeline Layout"),
        bind_group_layouts: &[&layout],
        push_constant_ranges: &[],
    });

    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("Select TopK Pipeline"),
        layout: Some(&pipeline_layout),
        module: &shader,
        entry_point: Some(entry_point_name()),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    })
}

/// Returns the entry-point name for the select shader.
///
/// Wrapped in a const fn so that a future rename lives in one place.
const fn entry_point_name() -> &'static str {
    "select_topk"
}
