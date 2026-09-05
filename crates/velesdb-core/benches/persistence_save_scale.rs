//! What `save()` costs, and how much of it is the vector payload.
//!
//! Run with: `cargo bench --bench persistence_save_scale`
//!
//! # Why this exists
//!
//! Nothing in this repository measured `save()` before this file. Two open
//! questions need it as their baseline, and neither can be answered by
//! reasoning about the code:
//!
//! - Issue #2173 wants to map `{basename}.vectors` directly as the graph's f32
//!   arena, which removes the dump's copy outright. The gain it claims is the
//!   half of `save()` measured here.
//! - `write_vector_data` (`native/graph_io.rs`) writes **one `write_all` per
//!   `f32`**. At 20 000 nodes and 768 dimensions that is 15.4 million calls
//!   through the `BufWriter` for a buffer that is one contiguous `&[f32]`.
//!
//! # Sweep dimension, not node count
//!
//! `save()` writes three things: `{basename}.vectors`, `{basename}.graph`, and
//! the sidecars. Only the first scales with **dimension**; the graph and the
//! sidecars scale with **node count**. So a sweep over node count moves all
//! three at once and cannot attribute a change to any of them, while a sweep
//! over dimension at a fixed node count holds graph and sidecar work constant
//! and leaves the vector payload as the only variable.
//!
//! That is why this benchmark sweeps dimension. The node count is overridable
//! for anyone who wants the other axis, but a difference measured that way is
//! not attributable to the vector dump.
//!
//! # What is being timed, and what is not
//!
//! **Userspace serialization, not durability.** `.vectors` is written through
//! a `BufWriter` and flushed, never `sync_all`'d — only the sidecars go
//! through `atomic_write`'s tmp+fsync+rename. The kernel's writeback therefore
//! falls outside the measured window. That is exactly the right thing to
//! measure for a change to the serialization loop, and exactly the wrong
//! number to quote as "time until the data is durable".
//!
//! **Steady-state re-save, not a first save.** Each iteration saves into the
//! same directory, so `File::create` truncates a file whose pages are already
//! warm. This matches what a long-lived collection actually does; it does not
//! describe the first save onto a cold cache.
//!
//! # Reading the output
//!
//! Each configuration prints its vector payload in MiB. Dividing that by the
//! reported time gives an effective serialization throughput, which is the
//! figure to compare across a change — the absolute times also carry the
//! constant graph and sidecar cost, which no change to the vector dump can
//! move.
//!
//! A payload figure that does not grow linearly with dimension means the
//! configuration was not built as intended, and the run should be discarded
//! rather than interpreted.
//!
//! # What has already been measured, so it is not re-derived
//!
//! `write_vector_data` (`native/graph_io.rs`) used to write one `write_all`
//! per `f32`. Measured on this instrument at 5 000 nodes by 3072d — 58.6 MiB
//! of payload, two separate runs per variant:
//!
//! | variant | run 1 | run 2 |
//! |---|---|---|
//! | one `write_all` per `f32` (before) | 80.5 ms | 89.3 ms |
//! | reusable row buffer, still `to_le_bytes` per value | 89.3 ms | 91.0 ms |
//! | hand-written `from_raw_parts` reinterpret | 61.4 ms | 62.7 ms |
//! | `bytemuck::cast_slice` per vector (shipped) | 56.0 ms | 55.5 ms |
//!
//! The second row is the trap, and it is worth understanding before anyone
//! tries it again: grouping the writes *looks* like the fix and is
//! neutral-to-worse. It removes the per-value `write_all` but keeps the
//! per-value `to_le_bytes` — `extend_from_slice` pays the same capacity check
//! and four-byte copy — and then adds a full row copy into the writer. The
//! cost was relocated and one copy was added.
//!
//! What actually pays is removing the per-value conversion, which on a
//! little-endian target is a reinterpret. That is ~34 % off the whole call and
//! ~2.1x on the dump alone once the ~29 ms graph-and-sidecar constant is
//! subtracted: 1 046 MiB/s becomes 2 186 MiB/s.
//!
//! Read those figures against the noise floor. At 768d the *same binary*
//! spread 20 % between two runs, because the vector dump is a minority of
//! `save()` there and machine noise on the whole call swamps it. At 3072d the
//! shipped variant reproduced to 0.9 %, which is what makes a 34 % claim
//! decidable at all. Anything measured at 768d on a busy machine is not.
//!
//! # Sizing
//!
//! Defaults finish unattended. The regime #2173 argues about is a collection
//! large enough for the copy to matter:
//!
//! ```text
//! VELESDB_SAVE_NODES=100000 \
//! VELESDB_SAVE_DIMS=64,256,768,1536 \
//! cargo bench --bench persistence_save_scale
//! ```
//!
//! Build time is sequential insertion and dominates the run: budget roughly a
//! minute per 200K nodes before any measurement starts.

#![allow(clippy::cast_precision_loss)]

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, SamplingMode};
use std::time::Duration;
use tempfile::TempDir;
use velesdb_core::{DistanceMetric, HnswIndex, VectorIndex};

/// Vector dimensions to sweep, overridable with `VELESDB_SAVE_DIMS`.
const DEFAULT_DIMS: &str = "64,256,768";
/// Node count held constant across the sweep, overridable with
/// `VELESDB_SAVE_NODES`.
const DEFAULT_NODES: usize = 20_000;

/// Generates a random-ish vector, matching `hnsw_benchmark`'s generator so
/// numbers from the two files describe the same kind of data.
fn generate_vector(dim: usize, seed: u64) -> Vec<f32> {
    (0..dim)
        .map(|i| (seed as f32 * 0.1 + i as f32 * 0.01).sin().midpoint(1.0))
        .collect()
}

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn dimensions() -> Vec<usize> {
    std::env::var("VELESDB_SAVE_DIMS")
        .unwrap_or_else(|_| DEFAULT_DIMS.to_string())
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect()
}

/// Bytes of f32 payload `{basename}.vectors` carries for this configuration.
///
/// Derived rather than read back from the file: the point is the ratio against
/// the measured time, and a figure computed from the inputs cannot drift if
/// the header ever grows.
fn payload_bytes(nodes: usize, dim: usize) -> usize {
    nodes * dim * std::mem::size_of::<f32>()
}

fn bench_save_by_payload(c: &mut Criterion) {
    let nodes = env_usize("VELESDB_SAVE_NODES", DEFAULT_NODES);
    let mut group = c.benchmark_group("hnsw_save_by_payload");
    // A save is long and uneven next to a search; flat sampling keeps criterion
    // from extrapolating an iteration count from an unrepresentative warm-up.
    group.sampling_mode(SamplingMode::Flat);
    group.measurement_time(Duration::from_secs(20));
    group.sample_size(10);

    for dim in dimensions() {
        // Built once, outside the measured closure: this benchmark is about
        // the dump, and an index rebuilt per iteration would measure insert.
        //
        // Sequential rather than `insert_batch_parallel` because the parallel
        // path builds a different graph every run. The dump cost does not
        // depend on graph *shape*, but the comparison across a change does:
        // two runs must dump the same thing to be comparable at all.
        let index = HnswIndex::new(dim, DistanceMetric::Cosine).expect("bench: index");
        for i in 0..nodes as u64 {
            index.insert(i, &generate_vector(dim, i));
        }
        index.set_searching_mode();

        // One directory reused across iterations, on purpose: see "steady-state
        // re-save" in the module docs. It outlives the loop body so the files
        // are not deleted between samples.
        let dir = TempDir::new().expect("bench: temp dir");
        let payload_mib = payload_bytes(nodes, dim) as f64 / (1024.0 * 1024.0);
        println!(
            "  [{nodes} nodes x {dim}d] vector payload {payload_mib:.1} MiB — divide by the \
             reported time for serialization throughput; the remainder is the graph and \
             sidecar cost, which a change to the vector dump cannot move"
        );

        group.bench_with_input(BenchmarkId::new("save", dim), &dim, |b, _| {
            b.iter(|| index.save(dir.path()).expect("bench: save"));
        });
    }

    group.finish();
}

criterion_group!(benches, bench_save_by_payload);
criterion_main!(benches);
