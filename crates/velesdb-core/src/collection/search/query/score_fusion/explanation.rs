//! Score Explanation API (EPIC-049 US-005).
//!
//! Provides detailed human-readable explanations of score computations.

use super::{ScoreBreakdown, ScoreFusionMethod};
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
    /// Component name (e.g., "vector_similarity").
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
    pub fn explain(&self, strategy: &ScoreFusionMethod) -> ScoreExplanation {
        let total_components = self.count_components();
        let default_weight = if total_components > 0 {
            1.0 / total_components as f32
        } else {
            1.0
        };

        let mut components = self.build_weighted_components(default_weight);
        self.append_boost_components(&mut components);

        let human_readable = Self::generate_human_readable(self.final_score, &components);

        ScoreExplanation {
            final_score: self.final_score,
            strategy: strategy.as_str().to_string(),
            components,
            human_readable,
        }
    }

    /// Builds weighted score components (vector, graph, path).
    fn build_weighted_components(&self, weight: f32) -> Vec<ComponentExplanation> {
        let scored = [
            (
                "vector_similarity",
                self.vector_similarity,
                "Cosine similarity to query vector",
            ),
            (
                "graph_distance",
                self.graph_distance,
                "Normalized graph proximity",
            ),
            (
                "path_score",
                self.path_score,
                "Path relevance (decay + rel types)",
            ),
        ];
        scored
            .into_iter()
            .filter_map(|(name, value, desc)| {
                let v = value?;
                Some(ComponentExplanation {
                    name: name.to_string(),
                    value: v,
                    weight: Some(weight),
                    contribution: v * weight,
                    description: format!("{desc}: {v:.3}"),
                })
            })
            .collect()
    }

    /// Appends multiplicative boost components (metadata, recency, custom).
    fn append_boost_components(&self, components: &mut Vec<ComponentExplanation>) {
        let boosts = [
            ("metadata_boost", self.metadata_boost, "Metadata multiplier"),
            ("recency_boost", self.recency_boost, "Recency multiplier"),
        ];
        for (name, value, desc) in boosts {
            if let Some(v) = value {
                components.push(ComponentExplanation {
                    name: name.to_string(),
                    value: v,
                    weight: None,
                    contribution: 0.0,
                    description: format!("{desc}: {v:.2}x"),
                });
            }
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
