//! Degree-aware storage routing for heterogeneous graph workloads.
//!
//! This module implements adaptive storage strategies based on vertex degree,
//! optimizing for both low-degree nodes (majority) and high-degree hubs.
//!
//! # EPIC-020 US-001: Degree-Aware Storage Router
//!
//! ## Background
//!
//! Real-world graphs exhibit power-law degree distribution:
//! - ~90% of nodes have < 100 edges (low-degree)
//! - ~10% of nodes have > 100 edges (high-degree "hubs")
//!
//! A single storage structure penalizes one or the other case.
//!
//! ## References
//!
//! - RapidStore (arXiv:2507.00839): "To handle degree skewness in real-world graphs,
//!   N(u) is stored differently based on vertex degree"
//! - LSMGraph (arXiv:2411.06392): Multi-Level CSR with vertex-grained versioning

#[cfg(test)]
mod tests;

use std::collections::HashSet;

use super::cart::CARTEdgeIndex;

/// Default threshold for switching from low-degree to high-degree storage.
pub const DEFAULT_DEGREE_THRESHOLD: usize = 100;

/// Threshold for switching from HashSet to C-ART (very high-degree hubs).
pub const CART_THRESHOLD: usize = 1000;

/// Trait for edge index implementations.
///
/// This trait abstracts over different storage strategies for node adjacency lists,
/// allowing the system to select optimal storage based on degree.
pub trait EdgeIndex: Send + Sync {
    /// Inserts a target node ID into the index.
    fn insert(&mut self, target: u64);

    /// Removes a target node ID from the index.
    ///
    /// Returns `true` if the target was present and removed.
    fn remove(&mut self, target: u64) -> bool;

    /// Checks if a target node ID is in the index.
    fn contains(&self, target: u64) -> bool;

    /// Returns an iterator over all target node IDs.
    fn targets(&self) -> Vec<u64>;

    /// Returns the number of targets in the index.
    fn len(&self) -> usize;

    /// Returns true if the index is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Vector-based edge index for low-degree nodes.
///
/// Optimal for nodes with < 100 edges:
/// - Cache-friendly sequential access
/// - Low memory overhead
/// - O(n) operations are fast for small n
#[derive(Debug, Clone, Default)]
pub struct VecEdgeIndex {
    pub(crate) targets: Vec<u64>,
}

impl VecEdgeIndex {
    /// Creates a new empty vector edge index.
    #[must_use]
    pub fn new() -> Self {
        Self {
            targets: Vec::new(),
        }
    }

    /// Creates a vector edge index with pre-allocated capacity.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            targets: Vec::with_capacity(capacity),
        }
    }
}

impl EdgeIndex for VecEdgeIndex {
    fn insert(&mut self, target: u64) {
        if !self.targets.contains(&target) {
            self.targets.push(target);
        }
    }

    fn remove(&mut self, target: u64) -> bool {
        if let Some(pos) = self.targets.iter().position(|&t| t == target) {
            self.targets.swap_remove(pos);
            true
        } else {
            false
        }
    }

    fn contains(&self, target: u64) -> bool {
        self.targets.contains(&target)
    }

    fn targets(&self) -> Vec<u64> {
        self.targets.clone()
    }

    fn len(&self) -> usize {
        self.targets.len()
    }
}

/// HashSet-based edge index for high-degree nodes.
///
/// Optimal for nodes with > 100 edges:
/// - O(1) contains/insert/remove
/// - Higher memory overhead per element
/// - Better scaling for high-degree hubs
#[derive(Debug, Clone, Default)]
pub struct HashSetEdgeIndex {
    pub(crate) targets: HashSet<u64>,
}

impl HashSetEdgeIndex {
    /// Creates a new empty hash set edge index.
    #[must_use]
    pub fn new() -> Self {
        Self {
            targets: HashSet::new(),
        }
    }

    /// Creates a hash set edge index with pre-allocated capacity.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            targets: HashSet::with_capacity(capacity),
        }
    }

    /// Creates from an existing vector index (for promotion).
    #[must_use]
    pub fn from_vec(vec_index: &VecEdgeIndex) -> Self {
        Self {
            targets: vec_index.targets.iter().copied().collect(),
        }
    }
}

impl EdgeIndex for HashSetEdgeIndex {
    fn insert(&mut self, target: u64) {
        self.targets.insert(target);
    }

    fn remove(&mut self, target: u64) -> bool {
        self.targets.remove(&target)
    }

    fn contains(&self, target: u64) -> bool {
        self.targets.contains(&target)
    }

    fn targets(&self) -> Vec<u64> {
        self.targets.iter().copied().collect()
    }

    fn len(&self) -> usize {
        self.targets.len()
    }
}

/// Storage variant for degree-adaptive storage.
#[derive(Debug, Clone)]
pub enum DegreeAdaptiveStorage {
    /// Low-degree storage (Vec-based, < 100 edges)
    LowDegree(VecEdgeIndex),
    /// High-degree storage (HashSet-based, 100-1000 edges)
    HighDegree(HashSetEdgeIndex),
    /// Very high-degree storage (C-ART, > 1000 edges - hub nodes)
    VeryHighDegree(CARTEdgeIndex),
}

impl Default for DegreeAdaptiveStorage {
    fn default() -> Self {
        Self::LowDegree(VecEdgeIndex::new())
    }
}

impl DegreeAdaptiveStorage {
    /// Creates a new low-degree storage.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns true if this is high-degree storage (HashSet or C-ART).
    #[must_use]
    pub fn is_high_degree(&self) -> bool {
        matches!(self, Self::HighDegree(_) | Self::VeryHighDegree(_))
    }

    /// Returns true if this is very high-degree storage (C-ART).
    #[must_use]
    pub fn is_very_high_degree(&self) -> bool {
        matches!(self, Self::VeryHighDegree(_))
    }

    /// Promotes to high-degree storage if currently low-degree.
    pub fn promote_to_high_degree(&mut self) {
        if let Self::LowDegree(vec_index) = self {
            *self = Self::HighDegree(HashSetEdgeIndex::from_vec(vec_index));
        }
    }

    /// Promotes to very high-degree C-ART storage if currently HashSet.
    pub fn promote_to_cart(&mut self) {
        if let Self::HighDegree(hash_index) = self {
            let targets: Vec<u64> = hash_index.targets.iter().copied().collect();
            *self = Self::VeryHighDegree(CARTEdgeIndex::from_targets(&targets));
        }
    }
}

impl EdgeIndex for DegreeAdaptiveStorage {
    fn insert(&mut self, target: u64) {
        match self {
            Self::LowDegree(vec) => vec.insert(target),
            Self::HighDegree(hash) => hash.insert(target),
            Self::VeryHighDegree(cart) => cart.insert(target),
        }
    }

    fn remove(&mut self, target: u64) -> bool {
        match self {
            Self::LowDegree(vec) => vec.remove(target),
            Self::HighDegree(hash) => hash.remove(target),
            Self::VeryHighDegree(cart) => cart.remove(target),
        }
    }

    fn contains(&self, target: u64) -> bool {
        match self {
            Self::LowDegree(vec) => vec.contains(target),
            Self::HighDegree(hash) => hash.contains(target),
            Self::VeryHighDegree(cart) => cart.contains(target),
        }
    }

    fn targets(&self) -> Vec<u64> {
        match self {
            Self::LowDegree(vec) => vec.targets(),
            Self::HighDegree(hash) => hash.targets(),
            Self::VeryHighDegree(cart) => cart.targets(),
        }
    }

    fn len(&self) -> usize {
        match self {
            Self::LowDegree(vec) => vec.len(),
            Self::HighDegree(hash) => hash.len(),
            Self::VeryHighDegree(cart) => cart.len(),
        }
    }
}

/// Degree-aware router that automatically selects storage strategy.
///
/// Monitors edge count per node and promotes to high-degree storage
/// when the threshold is exceeded.
#[derive(Debug, Clone)]
pub struct DegreeRouter {
    threshold: usize,
    pub(crate) storage: DegreeAdaptiveStorage,
    promotions: usize,
}

impl DegreeRouter {
    /// Creates a new degree router with the default threshold (100).
    #[must_use]
    pub fn new() -> Self {
        Self::with_threshold(DEFAULT_DEGREE_THRESHOLD)
    }

    /// Creates a new degree router with a custom threshold.
    #[must_use]
    pub fn with_threshold(threshold: usize) -> Self {
        Self {
            threshold: threshold.max(1),
            storage: DegreeAdaptiveStorage::new(),
            promotions: 0,
        }
    }

    /// Inserts a target, potentially triggering promotion to high-degree storage.
    ///
    /// Promotion thresholds:
    /// - Vec → HashSet: when len > DEFAULT_DEGREE_THRESHOLD (100)
    /// - HashSet → C-ART: when len > CART_THRESHOLD (1000)
    pub fn insert(&mut self, target: u64) {
        self.storage.insert(target);

        // Check for promotion to HashSet
        if !self.storage.is_high_degree() && self.storage.len() > self.threshold {
            self.storage.promote_to_high_degree();
            self.promotions += 1;
        }

        // Check for promotion to C-ART (for very high-degree hubs)
        if !self.storage.is_very_high_degree() && self.storage.len() > CART_THRESHOLD {
            self.storage.promote_to_cart();
            self.promotions += 1;
        }
    }

    /// Removes a target.
    pub fn remove(&mut self, target: u64) -> bool {
        self.storage.remove(target)
    }

    /// Checks if a target exists.
    #[must_use]
    pub fn contains(&self, target: u64) -> bool {
        self.storage.contains(target)
    }

    /// Returns all targets.
    #[must_use]
    pub fn targets(&self) -> Vec<u64> {
        self.storage.targets()
    }

    /// Returns the number of targets.
    #[must_use]
    pub fn len(&self) -> usize {
        self.storage.len()
    }

    /// Returns true if empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.storage.is_empty()
    }

    /// Returns true if using high-degree storage.
    #[must_use]
    pub fn is_high_degree(&self) -> bool {
        self.storage.is_high_degree()
    }

    /// Returns the number of promotions that have occurred.
    #[must_use]
    pub fn promotion_count(&self) -> usize {
        self.promotions
    }

    /// Returns the current threshold.
    #[must_use]
    pub fn threshold(&self) -> usize {
        self.threshold
    }
}

impl Default for DegreeRouter {
    fn default() -> Self {
        Self::new()
    }
}
