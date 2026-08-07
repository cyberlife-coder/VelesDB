//! Agent Memory Patterns SDK (EPIC-010)
//!
//! Provides unified memory abstractions for AI agents, supporting:
//! - **Semantic Memory**: Long-term knowledge stored as vectors (graph linkage planned)
//! - **Episodic Memory**: Temporal event sequences with context
//! - **Procedural Memory**: Learned patterns and action sequences
//!
//! # Features
//!
//! - **TTL/Eviction**: Automatic expiration and eviction policies
//! - **Snapshots**: Versioned state persistence and rollback
//! - **Temporal Index**: Efficient time-based queries for episodic memory
//! - **Adaptive Reinforcement**: Extensible strategies for procedural learning
//!
//! # Example
//!
//! ```ignore
//! use std::sync::Arc;
//! use velesdb_core::{Database, agent::AgentMemory};
//!
//! let db = Arc::new(Database::open("agent.db")?);
//! let memory = AgentMemory::new(Arc::clone(&db))?; // default dimension 384
//!
//! // Store semantic knowledge
//! memory.semantic().store(1, "Paris is the capital of France", &embedding)?;
//!
//! // Record an episode (timestamp in epoch seconds, optional embedding)
//! memory.episodic().record(2, "User asked about French geography", timestamp, Some(&embedding))?;
//!
//! // Learn a procedure (steps, optional embedding, confidence)
//! memory.procedural().learn(3, "answer_geography", &steps, Some(&embedding), 0.9)?;
//! ```

mod episodic_memory;
#[cfg(test)]
mod episodic_memory_tests;
mod error;
mod memory;
pub(crate) mod memory_helpers;
#[cfg(test)]
mod memory_tests;
mod procedural_memory;
#[cfg(test)]
mod procedural_memory_tests;
pub mod reinforcement;
#[cfg(test)]
mod reinforcement_tests;
mod semantic_memory;
#[cfg(test)]
mod semantic_memory_tests;
pub mod snapshot;
#[cfg(test)]
mod snapshot_tests;
pub mod temporal_index;
#[cfg(test)]
mod temporal_index_tests;
pub mod ttl;
#[cfg(test)]
mod ttl_tests;
#[cfg(test)]
mod velesql_bridge_tests;

pub use memory::{
    AgentMemory, AgentMemoryError, EpisodicMemory, ProceduralMemory, ProcedureMatch,
    SemanticMemory, DEFAULT_DIMENSION,
};

/// At most `cap` relation edges of one memory point, plus the honest signal
/// that the point carries more. Returned by the `relations_bounded` /
/// `incoming_relations_bounded` accessors of every memory kind.
///
/// The signal is a separate field because `edges.len() == cap` cannot carry
/// it: a node with exactly `cap` edges is indistinguishable from a truncated
/// one (#1820). `truncated` compares the node's TOTAL stored degree — read
/// under the same shard guard as the edges — against the scan cap, so it is
/// true exactly when stored edges were left unread. Note that TTL-expired
/// edges inside the scanned window are dropped without being replaced (the
/// O(cap) scan bound outranks returning exactly `cap` live entries), so
/// `edges.len() < cap` with `truncated == true` is a normal outcome.
#[derive(Debug, Clone)]
pub struct BoundedRelations {
    /// At most `cap` edges, in index order, TTL-expired far ends dropped.
    pub edges: Vec<crate::collection::graph::GraphEdge>,
    /// Whether the node's total stored degree exceeded the scan cap.
    pub truncated: bool,
}
pub use reinforcement::{
    power_law_decay, AdaptiveLearningRate, CompositeStrategy, ContextualReinforcement,
    DiminishingReturns, FixedRate, ReinforcementContext, ReinforcementStrategy, TemporalDecay,
};
pub use snapshot::{
    load_snapshot, load_snapshot_from_file, save_snapshot_to_file, MemoryState, SnapshotManager,
    SnapshotMetadata,
};
pub use temporal_index::{TemporalEntry, TemporalIndex, TemporalIndexStats};
pub use ttl::{EvictionConfig, ExpireResult, MemoryKind, MemoryTtl, TtlEntry};
