#![allow(missing_docs)] // Documentation will be added in follow-up PR
//! Adaptive reinforcement strategies for ProceduralMemory.
//!
//! Provides extensible strategies for updating procedure confidence:
//! - `FixedRate`: Simple fixed increment/decrement (default behavior)
//! - `AdaptiveLearningRate`: Adjusts rate based on usage history
//! - `TemporalDecay`: Older procedures lose relevance over time
//! - `ContextualReinforcement`: Considers usage context for updates

use std::collections::HashMap;

/// Context information for reinforcement decisions.
#[derive(Debug, Clone, Default)]
pub struct ReinforcementContext {
    /// Number of times the procedure has been used.
    pub usage_count: u64,
    /// Timestamp of last usage (Unix seconds).
    pub last_used: u64,
    /// Timestamp when the procedure was created (Unix seconds).
    pub created_at: u64,
    /// Current timestamp (Unix seconds).
    pub current_time: u64,
    /// Recent success rate (0.0 - 1.0).
    pub recent_success_rate: Option<f32>,
    /// Custom context data.
    pub custom: HashMap<String, f64>,
}

impl ReinforcementContext {
    /// Creates a new context with the current timestamp.
    #[must_use]
    pub fn new() -> Self {
        Self {
            current_time: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            ..Default::default()
        }
    }

    /// Sets the usage count.
    #[must_use]
    pub fn with_usage_count(mut self, count: u64) -> Self {
        self.usage_count = count;
        self
    }

    /// Sets the last used timestamp.
    #[must_use]
    pub fn with_last_used(mut self, timestamp: u64) -> Self {
        self.last_used = timestamp;
        self
    }

    /// Sets the creation timestamp.
    #[must_use]
    pub fn with_created_at(mut self, timestamp: u64) -> Self {
        self.created_at = timestamp;
        self
    }

    /// Sets the recent success rate.
    #[must_use]
    pub fn with_success_rate(mut self, rate: f32) -> Self {
        self.recent_success_rate = Some(rate);
        self
    }

    /// Adds custom context data.
    #[must_use]
    pub fn with_custom(mut self, key: &str, value: f64) -> Self {
        self.custom.insert(key.to_string(), value);
        self
    }

    /// Returns the age of the procedure in seconds.
    #[must_use]
    pub fn age_seconds(&self) -> u64 {
        self.current_time.saturating_sub(self.created_at)
    }

    /// Returns the time since last use in seconds.
    #[must_use]
    pub fn time_since_last_use(&self) -> u64 {
        self.current_time.saturating_sub(self.last_used)
    }
}

/// Trait for reinforcement strategies.
///
/// Implement this trait to create custom reinforcement behaviors.
pub trait ReinforcementStrategy: Send + Sync {
    /// Updates the confidence score based on success/failure.
    ///
    /// # Arguments
    ///
    /// * `old_confidence` - Current confidence score (0.0 - 1.0)
    /// * `success` - Whether the procedure execution was successful
    /// * `context` - Additional context for the decision
    ///
    /// # Returns
    ///
    /// New confidence score, clamped to [0.0, 1.0].
    fn update_confidence(
        &self,
        old_confidence: f32,
        success: bool,
        context: &ReinforcementContext,
    ) -> f32;

    /// Returns the name of this strategy.
    fn name(&self) -> &'static str;
}

/// Fixed rate reinforcement strategy.
///
/// Simple strategy with fixed increment on success and decrement on failure.
/// This is the default behavior matching the original implementation.
#[derive(Debug, Clone)]
pub struct FixedRate {
    /// Confidence increase on success.
    pub success_delta: f32,
    /// Confidence decrease on failure (positive value).
    pub failure_delta: f32,
}

impl Default for FixedRate {
    fn default() -> Self {
        Self {
            success_delta: 0.1,
            failure_delta: 0.05,
        }
    }
}

impl FixedRate {
    /// Creates a new fixed rate strategy with custom deltas.
    #[must_use]
    pub fn new(success_delta: f32, failure_delta: f32) -> Self {
        Self {
            success_delta,
            failure_delta,
        }
    }
}

impl ReinforcementStrategy for FixedRate {
    fn update_confidence(
        &self,
        old_confidence: f32,
        success: bool,
        _context: &ReinforcementContext,
    ) -> f32 {
        let new_confidence = if success {
            old_confidence + self.success_delta
        } else {
            old_confidence - self.failure_delta
        };
        new_confidence.clamp(0.0, 1.0)
    }

    fn name(&self) -> &'static str {
        "FixedRate"
    }
}

/// Adaptive learning rate strategy.
///
/// Adjusts the learning rate based on usage history:
/// - New procedures (low usage) have higher learning rates
/// - Well-established procedures (high usage) have lower learning rates
#[derive(Debug, Clone)]
pub struct AdaptiveLearningRate {
    /// Base learning rate for success.
    pub base_success_rate: f32,
    /// Base learning rate for failure.
    pub base_failure_rate: f32,
    /// Usage count at which learning rate is halved.
    pub half_life_usage: u64,
    /// Minimum learning rate multiplier.
    pub min_rate_multiplier: f32,
}

impl Default for AdaptiveLearningRate {
    fn default() -> Self {
        Self {
            base_success_rate: 0.2,
            base_failure_rate: 0.1,
            half_life_usage: 10,
            min_rate_multiplier: 0.1,
        }
    }
}

impl AdaptiveLearningRate {
    /// Calculates the learning rate multiplier based on usage count.
    fn rate_multiplier(&self, usage_count: u64) -> f32 {
        let half_life = self.half_life_usage.max(1);
        let decay = 0.5_f32.powf(usage_count as f32 / half_life as f32);
        decay.max(self.min_rate_multiplier)
    }
}

impl ReinforcementStrategy for AdaptiveLearningRate {
    fn update_confidence(
        &self,
        old_confidence: f32,
        success: bool,
        context: &ReinforcementContext,
    ) -> f32 {
        let multiplier = self.rate_multiplier(context.usage_count);

        let delta = if success {
            self.base_success_rate * multiplier
        } else {
            -self.base_failure_rate * multiplier
        };

        (old_confidence + delta).clamp(0.0, 1.0)
    }

    fn name(&self) -> &'static str {
        "AdaptiveLearningRate"
    }
}

/// Temporal decay strategy.
///
/// Older procedures gradually lose confidence over time.
/// Useful for domains where knowledge becomes stale.
#[derive(Debug, Clone)]
pub struct TemporalDecay {
    /// Base reinforcement rates.
    pub base: FixedRate,
    /// Half-life for confidence decay in seconds.
    pub decay_half_life: u64,
    /// Maximum decay per update (prevents sudden drops).
    pub max_decay_per_update: f32,
}

impl Default for TemporalDecay {
    fn default() -> Self {
        Self {
            base: FixedRate::default(),
            decay_half_life: 30 * 24 * 60 * 60, // 30 days
            max_decay_per_update: 0.1,
        }
    }
}

impl TemporalDecay {
    /// Creates a new temporal decay strategy.
    #[must_use]
    pub fn new(decay_half_life_days: u64) -> Self {
        Self {
            decay_half_life: decay_half_life_days * 24 * 60 * 60,
            ..Default::default()
        }
    }

    /// Calculates the decay factor based on time since last use.
    fn decay_factor(&self, time_since_last_use: u64) -> f32 {
        let half_life = self.decay_half_life.max(1);
        let decay = 0.5_f32.powf(time_since_last_use as f32 / half_life as f32);
        (1.0 - decay).min(self.max_decay_per_update)
    }
}

impl ReinforcementStrategy for TemporalDecay {
    fn update_confidence(
        &self,
        old_confidence: f32,
        success: bool,
        context: &ReinforcementContext,
    ) -> f32 {
        let time_decay = self.decay_factor(context.time_since_last_use());
        let decayed_confidence = old_confidence * (1.0 - time_decay);

        let new_confidence = if success {
            decayed_confidence + self.base.success_delta
        } else {
            decayed_confidence - self.base.failure_delta
        };

        new_confidence.clamp(0.0, 1.0)
    }

    fn name(&self) -> &'static str {
        "TemporalDecay"
    }
}

/// Contextual reinforcement strategy.
///
/// Considers multiple context factors for confidence updates:
/// - Recent success rate affects learning direction
/// - Usage frequency affects learning magnitude
/// - Time since last use affects decay
#[derive(Debug, Clone)]
pub struct ContextualReinforcement {
    /// Weight for success rate influence (0.0 - 1.0).
    pub success_rate_weight: f32,
    /// Weight for usage frequency influence (0.0 - 1.0).
    pub usage_weight: f32,
    /// Weight for recency influence (0.0 - 1.0).
    pub recency_weight: f32,
    /// Base learning rate.
    pub base_rate: f32,
}

impl Default for ContextualReinforcement {
    fn default() -> Self {
        Self {
            success_rate_weight: 0.3,
            usage_weight: 0.3,
            recency_weight: 0.4,
            base_rate: 0.15,
        }
    }
}

impl ContextualReinforcement {
    /// Calculates the recency factor (higher for recently used procedures).
    fn recency_factor(&self, time_since_last_use: u64) -> f32 {
        let hours = time_since_last_use as f32 / 3600.0;
        (-hours / 168.0).exp() // Decay over ~1 week
    }

    /// Calculates the usage factor (higher for frequently used procedures).
    fn usage_factor(&self, usage_count: u64) -> f32 {
        let normalized = (usage_count as f32).ln_1p() / 10.0;
        normalized.min(1.0)
    }
}

impl ReinforcementStrategy for ContextualReinforcement {
    fn update_confidence(
        &self,
        old_confidence: f32,
        success: bool,
        context: &ReinforcementContext,
    ) -> f32 {
        let recency = self.recency_factor(context.time_since_last_use());
        let usage = self.usage_factor(context.usage_count);
        let success_rate = context.recent_success_rate.unwrap_or(0.5);

        let context_score = self.recency_weight * recency
            + self.usage_weight * usage
            + self.success_rate_weight * success_rate;

        let effective_rate = self.base_rate * (0.5 + context_score);

        let delta = if success {
            effective_rate
        } else {
            -effective_rate * 0.5
        };

        (old_confidence + delta).clamp(0.0, 1.0)
    }

    fn name(&self) -> &'static str {
        "ContextualReinforcement"
    }
}

/// Composite strategy that combines multiple strategies.
///
/// Useful for complex reinforcement policies.
pub struct CompositeStrategy {
    strategies: Vec<(Box<dyn ReinforcementStrategy>, f32)>,
}

impl CompositeStrategy {
    /// Creates a new composite strategy.
    #[must_use]
    pub fn new() -> Self {
        Self {
            strategies: Vec::new(),
        }
    }

    /// Adds a strategy with a weight.
    ///
    /// Weights are normalized when calculating the final confidence.
    #[must_use]
    pub fn add_strategy<S: ReinforcementStrategy + 'static>(
        mut self,
        strategy: S,
        weight: f32,
    ) -> Self {
        self.strategies.push((Box::new(strategy), weight));
        self
    }
}

impl Default for CompositeStrategy {
    fn default() -> Self {
        Self::new()
    }
}

impl ReinforcementStrategy for CompositeStrategy {
    fn update_confidence(
        &self,
        old_confidence: f32,
        success: bool,
        context: &ReinforcementContext,
    ) -> f32 {
        if self.strategies.is_empty() {
            return old_confidence;
        }

        let total_weight: f32 = self.strategies.iter().map(|(_, w)| w).sum();
        if total_weight == 0.0 {
            return old_confidence;
        }

        let weighted_sum: f32 = self
            .strategies
            .iter()
            .map(|(strategy, weight)| {
                strategy.update_confidence(old_confidence, success, context) * weight
            })
            .sum();

        (weighted_sum / total_weight).clamp(0.0, 1.0)
    }

    fn name(&self) -> &'static str {
        "CompositeStrategy"
    }
}

/// Default strategy factory.
///
/// Returns the default reinforcement strategy (FixedRate).
#[must_use]
pub fn default_strategy() -> Box<dyn ReinforcementStrategy> {
    Box::new(FixedRate::default())
}
