//! Metadata Boost Functions (EPIC-049 US-003).
//!
//! Provides boost functions that modify scores based on document metadata,
//! including recency decay, field-based boosts, and composite strategies.

// SAFETY: Numeric casts in boost functions are intentional:
// - u64->i64 for timestamps: SystemTime::as_secs() values are within valid range
// - f64->f32 for decay computation: precision loss acceptable for ranking heuristics
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_wrap)]

/// Trait for boost functions that modify scores based on document metadata.
pub trait BoostFunction: Send + Sync {
    /// Computes a boost multiplier for a document.
    ///
    /// Returns a value where:
    /// - 1.0 = no boost (neutral)
    /// - > 1.0 = positive boost (increases score)
    /// - < 1.0 = negative boost (decreases score)
    fn compute(&self, document: &serde_json::Value) -> f32;

    /// Returns the name of this boost function for debugging.
    fn name(&self) -> &'static str;
}

/// Recency boost: favors recent documents with exponential decay.
#[derive(Debug, Clone)]
pub struct RecencyBoost {
    /// Field containing timestamp (RFC3339 or Unix epoch).
    pub field: String,
    /// Decay half-life in days.
    pub half_life_days: f64,
    /// Maximum boost for brand new documents.
    pub max_boost: f32,
}

impl Default for RecencyBoost {
    fn default() -> Self {
        Self {
            field: "created_at".to_string(),
            half_life_days: 30.0,
            max_boost: 1.5,
        }
    }
}

impl RecencyBoost {
    /// Creates a new recency boost.
    #[must_use]
    pub fn new(field: impl Into<String>, half_life_days: f64, max_boost: f32) -> Self {
        Self {
            field: field.into(),
            half_life_days: half_life_days.max(0.1),
            max_boost: max_boost.max(1.0),
        }
    }
}

impl BoostFunction for RecencyBoost {
    fn compute(&self, document: &serde_json::Value) -> f32 {
        let age_days = self.extract_age_days(document);

        match age_days {
            Some(days) if days >= 0.0 => {
                let decay = 0.5_f64.powf(days / self.half_life_days);
                1.0 + (self.max_boost - 1.0) * decay as f32
            }
            _ => 1.0, // No timestamp or future date -> neutral
        }
    }

    fn name(&self) -> &'static str {
        "recency"
    }
}

impl RecencyBoost {
    fn extract_age_days(&self, document: &serde_json::Value) -> Option<f64> {
        let field_value = document.get(&self.field)?;

        // Get current Unix timestamp
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_secs() as i64;

        // Try Unix timestamp (seconds) - most common for APIs
        if let Some(epoch) = field_value.as_i64() {
            return Some((now_secs - epoch) as f64 / 86400.0);
        }

        // Try Unix timestamp as float
        if let Some(epoch) = field_value.as_f64() {
            return Some((now_secs as f64 - epoch) / 86400.0);
        }

        None
    }
}

/// Field boost: boosts based on a numeric metadata field.
#[derive(Debug, Clone)]
pub struct FieldBoost {
    /// Field name containing numeric value.
    pub field: String,
    /// Scale factor (multiplied with field value).
    pub scale: f32,
    /// Minimum boost value (floor).
    pub min_boost: f32,
    /// Maximum boost value (ceiling).
    pub max_boost: f32,
}

impl Default for FieldBoost {
    fn default() -> Self {
        Self {
            field: "importance".to_string(),
            scale: 0.1,
            min_boost: 0.5,
            max_boost: 2.0,
        }
    }
}

impl FieldBoost {
    /// Creates a new field boost.
    #[must_use]
    pub fn new(field: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            ..Default::default()
        }
    }

    /// Builder: set scale factor.
    #[must_use]
    pub fn with_scale(mut self, scale: f32) -> Self {
        self.scale = scale;
        self
    }

    /// Builder: set min/max bounds.
    #[must_use]
    pub fn with_bounds(mut self, min: f32, max: f32) -> Self {
        self.min_boost = min;
        self.max_boost = max;
        self
    }
}

impl BoostFunction for FieldBoost {
    fn compute(&self, document: &serde_json::Value) -> f32 {
        let value = document
            .get(&self.field)
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0) as f32;

        let boost = 1.0 + value * self.scale;
        boost.clamp(self.min_boost, self.max_boost)
    }

    fn name(&self) -> &'static str {
        "field"
    }
}

/// Strategy for combining multiple boost functions.
#[derive(Debug, Clone, Copy, Default)]
pub enum BoostCombination {
    /// Multiply all boosts together.
    #[default]
    Multiply,
    /// Add boosts (subtracting n-1 to keep neutral at 1.0).
    Add,
    /// Take maximum boost.
    Max,
    /// Take minimum boost.
    Min,
}

/// Composite boost: combines multiple boost functions.
#[derive(Default)]
pub struct CompositeBoost {
    boosts: Vec<Box<dyn BoostFunction>>,
    combination: BoostCombination,
}

impl CompositeBoost {
    /// Creates a new composite boost.
    #[must_use]
    pub fn new(combination: BoostCombination) -> Self {
        Self {
            boosts: Vec::new(),
            combination,
        }
    }

    /// Adds a boost function to the composite.
    pub fn add(&mut self, boost: impl BoostFunction + 'static) {
        self.boosts.push(Box::new(boost));
    }

    /// Builder: add a boost function.
    #[must_use]
    pub fn with_boost(mut self, boost: impl BoostFunction + 'static) -> Self {
        self.add(boost);
        self
    }
}

impl BoostFunction for CompositeBoost {
    fn compute(&self, document: &serde_json::Value) -> f32 {
        if self.boosts.is_empty() {
            return 1.0;
        }

        let values: Vec<f32> = self.boosts.iter().map(|b| b.compute(document)).collect();

        match self.combination {
            BoostCombination::Multiply => values.iter().product(),
            BoostCombination::Add => {
                // Sum boosts, subtract (n-1) to keep neutral at 1.0
                values.iter().sum::<f32>() - (values.len() as f32 - 1.0)
            }
            BoostCombination::Max => values.iter().copied().fold(1.0_f32, f32::max),
            BoostCombination::Min => values.iter().copied().fold(f32::MAX, f32::min),
        }
    }

    fn name(&self) -> &'static str {
        "composite"
    }
}
