//! Tests for the agent-memory configuration of [`VelesDbState`].

use super::*;
use tempfile::tempdir;

/// Without a configured dimension the handle keeps core's default (384).
#[test]
fn the_memory_dimension_defaults_to_cores() {
    let dir = tempdir().expect("test: temp dir");
    let state = VelesDbState::new(dir.path().to_path_buf());

    assert_eq!(
        state.memory_dimension(),
        velesdb_core::agent::DEFAULT_DIMENSION
    );
    let built = state
        .with_memory(|mem| Ok(mem.semantic().dimension()))
        .expect("test: memory handle");
    assert_eq!(built, velesdb_core::agent::DEFAULT_DIMENSION);
}

/// A configured dimension reaches the handle: a 4-d embedding stores and
/// queries, where the fixed 384-d handle refused it with a dimension
/// mismatch.
#[test]
fn a_configured_memory_dimension_reaches_the_handle() {
    let dir = tempdir().expect("test: temp dir");
    let state = VelesDbState::new(dir.path().to_path_buf()).with_memory_dimension(4);
    let embedding = [0.1_f32, 0.2, 0.3, 0.4];

    let hits = state
        .with_memory(|mem| {
            mem.semantic().store(1, "four dims", &embedding)?;
            Ok(mem.semantic().query(&embedding, 1)?)
        })
        .expect("test: a 4-d fact must store and query");

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].0, 1);
    assert_eq!(
        state
            .with_memory(|mem| Ok(mem.episodic().dimension()))
            .expect("test: memory handle"),
        4
    );
}
