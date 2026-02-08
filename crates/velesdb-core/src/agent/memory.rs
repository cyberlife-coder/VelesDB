//! AgentMemory - Unified memory interface for AI agents (EPIC-010)
//!
//! Provides three memory subsystems for AI agents:
//! - **SemanticMemory**: Long-term knowledge facts with vector similarity search
//! - **EpisodicMemory**: Event timeline with temporal and similarity queries
//! - **ProceduralMemory**: Learned patterns with confidence scoring
//!
//! # Enhanced Features
//!
//! - **TTL/Eviction**: Automatic expiration and memory consolidation
//! - **Snapshots**: Versioned state persistence and rollback
//! - **Temporal Index**: Efficient O(log N) time-based queries
//! - **Adaptive Reinforcement**: Extensible confidence update strategies

// SAFETY: Numeric casts in agent memory are intentional:
// - u64 <-> i64 casts for timestamps (SystemTime uses u64, DB schema uses i64)
// - Values are always positive (elapsed time) and bounded by reasonable ranges
// - Casts verified by temporal index tests and snapshot functionality
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::cast_sign_loss)]

use crate::Database;
use std::sync::Arc;

pub use super::episodic_memory::EpisodicMemory;
pub use super::error::AgentMemoryError;
pub use super::procedural_memory::{ProceduralMemory, ProcedureMatch};
pub use super::semantic_memory::SemanticMemory;
pub use super::snapshot::{MemoryState, SnapshotManager};
pub use super::temporal_index::TemporalIndex;
pub use super::ttl::{EvictionConfig, ExpireResult, MemoryTtl};

/// Default embedding dimension for memory collections.
pub const DEFAULT_DIMENSION: usize = 384;

/// Unified memory interface for AI agents.
///
/// Provides access to three memory subsystems:
/// - `semantic`: Long-term knowledge (vector-graph storage)
/// - `episodic`: Event timeline with temporal context
/// - `procedural`: Learned patterns and action sequences
///
/// # Enhanced Features
///
/// - TTL management for automatic expiration
/// - Snapshot/restore for state persistence
/// - Temporal indexing for efficient time-based queries
/// - Configurable eviction policies
///
/// Uses lifetime `'a` to borrow the Database without cloning.
pub struct AgentMemory<'a> {
    semantic: SemanticMemory<'a>,
    episodic: EpisodicMemory<'a>,
    procedural: ProceduralMemory<'a>,
    ttl: Arc<MemoryTtl>,
    // Reason: temporal_index will be used for time-based queries in future implementation
    #[allow(dead_code)]
    temporal_index: Arc<TemporalIndex>,
    eviction_config: EvictionConfig,
    snapshot_manager: Option<SnapshotManager>,
}

impl<'a> AgentMemory<'a> {
    /// Creates a new AgentMemory instance from a Database.
    ///
    /// Initializes or connects to the three memory subsystem collections:
    /// - `_semantic_memory`: For knowledge facts
    /// - `_episodic_memory`: For event timeline
    /// - `_procedural_memory`: For learned patterns
    ///
    /// Uses the default embedding dimension (384).
    pub fn new(db: &'a Database) -> Result<Self, AgentMemoryError> {
        Self::with_dimension(db, DEFAULT_DIMENSION)
    }

    /// Creates a new AgentMemory with a custom embedding dimension.
    pub fn with_dimension(db: &'a Database, dimension: usize) -> Result<Self, AgentMemoryError> {
        let ttl = Arc::new(MemoryTtl::new());
        let temporal_index = Arc::new(TemporalIndex::new());

        let semantic = SemanticMemory::new(db, dimension, Arc::clone(&ttl))?;
        let episodic =
            EpisodicMemory::new(db, dimension, Arc::clone(&ttl), Arc::clone(&temporal_index))?;
        let procedural = ProceduralMemory::new(db, dimension, Arc::clone(&ttl))?;

        Ok(Self {
            semantic,
            episodic,
            procedural,
            ttl,
            temporal_index,
            eviction_config: EvictionConfig::default(),
            snapshot_manager: None,
        })
    }

    /// Configures eviction policies for automatic memory cleanup.
    #[must_use]
    pub fn with_eviction_config(mut self, config: EvictionConfig) -> Self {
        self.eviction_config = config;
        self
    }

    /// Enables snapshot management with a storage directory.
    ///
    /// # Arguments
    ///
    /// * `snapshot_dir` - Directory path for storing snapshots
    /// * `max_snapshots` - Maximum number of snapshots to retain
    #[must_use]
    pub fn with_snapshots(mut self, snapshot_dir: &str, max_snapshots: usize) -> Self {
        self.snapshot_manager = Some(SnapshotManager::new(snapshot_dir, max_snapshots));
        self
    }

    /// Returns a reference to the semantic memory subsystem.
    #[must_use]
    pub fn semantic(&self) -> &SemanticMemory<'a> {
        &self.semantic
    }

    /// Returns a reference to the episodic memory subsystem.
    #[must_use]
    pub fn episodic(&self) -> &EpisodicMemory<'a> {
        &self.episodic
    }

    /// Returns a reference to the procedural memory subsystem.
    #[must_use]
    pub fn procedural(&self) -> &ProceduralMemory<'a> {
        &self.procedural
    }

    /// Sets TTL for a semantic memory entry.
    pub fn set_semantic_ttl(&self, id: u64, ttl_seconds: u64) {
        self.ttl.set_ttl(id, ttl_seconds);
    }

    /// Sets TTL for an episodic memory entry.
    pub fn set_episodic_ttl(&self, id: u64, ttl_seconds: u64) {
        self.ttl.set_ttl(id, ttl_seconds);
    }

    /// Sets TTL for a procedural memory entry.
    pub fn set_procedural_ttl(&self, id: u64, ttl_seconds: u64) {
        self.ttl.set_ttl(id, ttl_seconds);
    }

    /// Performs automatic expiration of entries that have exceeded their TTL.
    ///
    /// This method should be called periodically to clean up expired entries.
    /// It also handles consolidation of old episodic memories to semantic memory
    /// based on the configured eviction policy.
    ///
    /// # Returns
    ///
    /// Statistics about the expiration operation.
    pub fn auto_expire(&self) -> Result<ExpireResult, AgentMemoryError> {
        let expired_ids = self.ttl.expire();
        let mut result = ExpireResult::default();

        for id in &expired_ids {
            if self.semantic.delete(*id).is_ok() {
                result.semantic_expired += 1;
            }
            if self.episodic.delete(*id).is_ok() {
                result.episodic_expired += 1;
            }
            if self.procedural.delete(*id).is_ok() {
                result.procedural_expired += 1;
            }
        }

        if self.eviction_config.consolidation_age_threshold > 0 {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            let cutoff = now - self.eviction_config.consolidation_age_threshold as i64;
            result.episodic_consolidated = self.consolidate_old_episodes(cutoff)?;
        }

        Ok(result)
    }

    /// Evicts procedures with confidence below the threshold.
    ///
    /// # Arguments
    ///
    /// * `min_confidence` - Minimum confidence threshold (0.0 - 1.0)
    ///
    /// # Returns
    ///
    /// Number of procedures evicted.
    pub fn evict_low_confidence_procedures(
        &self,
        min_confidence: f32,
    ) -> Result<usize, AgentMemoryError> {
        let all_procedures = self.procedural.list_all()?;
        let mut evicted = 0;

        for proc in all_procedures {
            if proc.confidence < min_confidence {
                self.procedural.delete(proc.id)?;
                evicted += 1;
            }
        }

        Ok(evicted)
    }

    /// Creates a snapshot of the current memory state.
    ///
    /// # Returns
    ///
    /// The version number of the created snapshot.
    pub fn snapshot(&self) -> Result<u64, AgentMemoryError> {
        let manager = self.snapshot_manager.as_ref().ok_or_else(|| {
            AgentMemoryError::SnapshotError("Snapshot manager not configured".to_string())
        })?;

        let state = MemoryState {
            semantic: self.semantic.serialize()?,
            episodic: self.episodic.serialize()?,
            procedural: self.procedural.serialize()?,
            ttl: self.ttl.serialize(),
        };

        Ok(manager.create_versioned_snapshot(&state)?)
    }

    /// Loads the most recent snapshot.
    ///
    /// # Returns
    ///
    /// The version number of the loaded snapshot.
    pub fn load_latest_snapshot(&self) -> Result<u64, AgentMemoryError> {
        let manager = self.snapshot_manager.as_ref().ok_or_else(|| {
            AgentMemoryError::SnapshotError("Snapshot manager not configured".to_string())
        })?;

        let (version, state) = manager.load_latest()?;
        self.restore_state(&state)?;
        Ok(version)
    }

    /// Loads a specific snapshot version.
    pub fn load_snapshot_version(&self, version: u64) -> Result<(), AgentMemoryError> {
        let manager = self.snapshot_manager.as_ref().ok_or_else(|| {
            AgentMemoryError::SnapshotError("Snapshot manager not configured".to_string())
        })?;

        let state = manager.load_version(version)?;
        self.restore_state(&state)?;
        Ok(())
    }

    /// Lists all available snapshot versions.
    pub fn list_snapshot_versions(&self) -> Result<Vec<u64>, AgentMemoryError> {
        let manager = self.snapshot_manager.as_ref().ok_or_else(|| {
            AgentMemoryError::SnapshotError("Snapshot manager not configured".to_string())
        })?;

        Ok(manager.list_versions()?)
    }

    fn restore_state(&self, state: &MemoryState) -> Result<(), AgentMemoryError> {
        self.semantic.deserialize(&state.semantic)?;
        self.episodic.deserialize(&state.episodic)?;
        self.procedural.deserialize(&state.procedural)?;

        if let Some(ttl) = MemoryTtl::deserialize(&state.ttl) {
            self.ttl.replace_from(&ttl);
        } else {
            self.ttl.clear();
        }

        Ok(())
    }

    fn consolidate_old_episodes(&self, cutoff_timestamp: i64) -> Result<usize, AgentMemoryError> {
        let old_events = self.episodic.older_than(cutoff_timestamp, 1000)?;
        let mut consolidated = 0;

        for (id, _description, _timestamp) in old_events {
            if let Some((description, _ts, embedding)) = self.episodic.get_with_embedding(id)? {
                self.semantic.store(id, &description, &embedding)?;
                self.episodic.delete(id)?;
                consolidated += 1;
            }
        }

        Ok(consolidated)
    }
}
