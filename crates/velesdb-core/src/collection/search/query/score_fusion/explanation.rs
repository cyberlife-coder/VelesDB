//! Score Explanation API (EPIC-049 US-005).
//!
//! Provides detailed human-readable explanations of score computations.

use super::{ScoreBreakdown, ScoreCombineStrategy};
use serde::{Deserialize, Serialize};

/// Detailed explanation of a score's components.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoreExplanation {
    /// Final computed score.
    pub final_score: f32,
    /// Name of the fusion strategy used.
    pub strategy: String,
    /// Breakdown of individual score components.
    pub components: Vec<ComponentExplanation>,
    /// Human-readable explanation text.
    pub human_readable: String,
}

/// Explanation of a single score component.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentExplanation {
    /// Component name (e.g., "`vector_similarity`").
    pub name: String,
    /// Raw value of the component.
    pub value: f32,
    /// Weight applied to this component (if weighted strategy).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weight: Option<f32>,
    /// Contribution to final score.
    pub contribution: f32,
    /// Human-readable description.
    pub description: String,
}

impl ScoreBreakdown {
    /// Generates a detailed explanation of the score breakdown.
    #[must_use]
    pub fn explain(&self, strategy: &ScoreCombineStrategy) -> ScoreExplanation {
        let mut components = Vec::new();
        let total_components = self.count_components();
        let default_weight = if total_components > 0 {
            1.0 / total_components as f32
        } else {
            1.0
        };

        if let Some(v) = self.vector_similarity {
            components.push(ComponentExplanation {
                name: "vector_similarity".to_string(),
                value: v,
                weight: Some(default_weight),
                contribution: v * default_weight,
                description: format!("Cosine similarity to query vector: {v:.3}"),
            });
        }

        if let Some(g) = self.graph_distance {
            components.push(ComponentExplanation {
                name: "graph_distance".to_string(),
                value: g,
                weight: Some(default_weight),
                contribution: g * default_weight,
                description: format!("Normalized graph proximity: {g:.3}"),
            });
        }

        if let Some(p) = self.path_score {
            components.push(ComponentExplanation {
                name: "path_score".to_string(),
                value: p,
                weight: Some(default_weight),
                contribution: p * default_weight,
                description: format!("Path relevance (decay + rel types): {p:.3}"),
            });
        }

        if let Some(m) = self.metadata_boost {
            components.push(ComponentExplanation {
                name: "metadata_boost".to_string(),
                value: m,
                weight: None, // Multiplicative, not weighted
                contribution: 0.0,
                description: format!("Metadata multiplier: {m:.2}x"),
            });
        }

        if let Some(r) = self.recency_boost {
            components.push(ComponentExplanation {
                name: "recency_boost".to_string(),
                value: r,
                weight: None,
                contribution: 0.0,
                description: format!("Recency multiplier: {r:.2}x"),
            });
        }

        for (name, &boost) in &self.custom_boosts {
            components.push(ComponentExplanation {
                name: format!("custom:{name}"),
                value: boost,
                weight: None,
                contribution: 0.0,
                description: format!("Custom boost '{name}': {boost:.2}x"),
            });
        }

        let human_readable = Self::generate_human_readable(self.final_score, &components);

        ScoreExplanation {
            final_score: self.final_score,
            strategy: strategy.as_str().to_string(),
            components,
            human_readable,
        }
    }

    pub(crate) fn count_components(&self) -> usize {
        let mut count = 0;
        if self.vector_similarity.is_some() {
            count += 1;
        }
        if self.graph_distance.is_some() {
            count += 1;
        }
        if self.path_score.is_some() {
            count += 1;
        }
        count
    }

    fn generate_human_readable(final_score: f32, components: &[ComponentExplanation]) -> String {
        let mut lines = vec![format!("Final score: {final_score:.3}")];

        for c in components {
            if let Some(w) = c.weight {
                lines.push(format!(
                    "  • {}: {:.3} (weight: {:.0}%)",
                    c.name,
                    c.value,
                    w * 100.0
                ));
            } else {
                lines.push(format!("  • {}: {:.2}x (multiplier)", c.name, c.value));
            }
        }

        lines.join("\n")
    }
}
