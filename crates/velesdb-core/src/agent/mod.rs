//! Agent Memory Patterns SDK (EPIC-010)
//!
//! Provides unified memory abstractions for AI agents, supporting:
//! - **Semantic Memory**: Long-term knowledge stored as vector-graph
//! - **Episodic Memory**: Temporal event sequences with context
//! - **Procedural Memory**: Learned patterns and action sequences
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

mod memory;
#[cfg(test)]
mod memory_tests;

pub use memory::{AgentMemory, AgentMemoryError};
