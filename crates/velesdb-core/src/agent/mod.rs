//! Agent Memory Patterns SDK (EPIC-010)
//!
//! Provides unified memory abstractions for AI agents, supporting:
//! - **Semantic Memory**: Long-term knowledge stored as vector-graph
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
//! use velesdb_core::{Database, agent::AgentMemory};
//!
//! let db = Database::open("agent.db")?;
//! let memory = AgentMemory::new(&db)?;
//!
//! // Store semantic knowledge
//! memory.semantic().store("Paris is the capital of France", embedding)?;
//!
//! // Record an episode
//! memory.episodic().record("User asked about French geography")?;
//!
//! // Learn a procedure
//! memory.procedural().learn("answer_geography", steps)?;
//! ```

mod episodic_memory;
mod error;
mod memory;
#[cfg(test)]
mod memory_tests;
mod procedural_memory;
pub mod reinforcement;
#[cfg(test)]
mod reinforcement_tests;
mod semantic_memory;
pub mod snapshot;
#[cfg(test)]
mod snapshot_tests;
pub mod temporal_index;
#[cfg(test)]
mod temporal_index_tests;
pub mod ttl;
#[cfg(test)]
mod ttl_tests;

pub use memory::{
    AgentMemory, AgentMemoryError, EpisodicMemory, ProceduralMemory, ProcedureMatch,
    SemanticMemory, DEFAULT_DIMENSION,
};
pub use reinforcement::{
    AdaptiveLearningRate, CompositeStrategy, ContextualReinforcement, FixedRate,
    ReinforcementContext, ReinforcementStrategy, TemporalDecay,
};
pub use snapshot::{
    load_snapshot, load_snapshot_from_file, save_snapshot_to_file, MemoryState, SnapshotError,
    SnapshotManager, SnapshotMetadata,
};
pub use temporal_index::{TemporalEntry, TemporalIndex, TemporalIndexStats};
pub use ttl::{EvictionConfig, ExpireResult, MemoryTtl, TtlEntry};
