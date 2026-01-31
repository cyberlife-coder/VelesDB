//! Tests for reinforcement strategies.

#[cfg(test)]
mod tests {
    use super::super::reinforcement::*;

    #[test]
    fn test_fixed_rate_success() {
        let strategy = FixedRate::default();
        let context = ReinforcementContext::new();
        let new_confidence = strategy.update_confidence(0.5, true, &context);
        assert!((new_confidence - 0.6).abs() < 0.001);
    }

    #[test]
    fn test_fixed_rate_failure() {
        let strategy = FixedRate::default();
        let context = ReinforcementContext::new();
        let new_confidence = strategy.update_confidence(0.5, false, &context);
        assert!((new_confidence - 0.45).abs() < 0.001);
    }

    #[test]
    fn test_fixed_rate_clamp_max() {
        let strategy = FixedRate::default();
        let context = ReinforcementContext::new();
        let new_confidence = strategy.update_confidence(0.95, true, &context);
        assert!((new_confidence - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_fixed_rate_clamp_min() {
        let strategy = FixedRate::default();
        let context = ReinforcementContext::new();
        let new_confidence = strategy.update_confidence(0.02, false, &context);
        assert!((new_confidence - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_adaptive_learning_rate() {
        let strategy = AdaptiveLearningRate::default();
        let context = ReinforcementContext::new().with_usage_count(10);
        let new_confidence = strategy.update_confidence(0.5, true, &context);
        assert!(new_confidence > 0.5);
        assert!(new_confidence <= 1.0);
    }

    #[test]
    fn test_temporal_decay() {
        let strategy = TemporalDecay::default();
        let context = ReinforcementContext::new()
            .with_created_at(0)
            .with_last_used(0);
        let new_confidence = strategy.update_confidence(0.5, true, &context);
        assert!(new_confidence > 0.5);
    }

    #[test]
    fn test_contextual_reinforcement() {
        let strategy = ContextualReinforcement::default();
        let context = ReinforcementContext::new()
            .with_usage_count(5)
            .with_success_rate(0.8);
        let new_confidence = strategy.update_confidence(0.5, true, &context);
        assert!(new_confidence > 0.5);
    }

    #[test]
    fn test_composite_strategy() {
        let mut strategy = CompositeStrategy::new();
        strategy = strategy.add_strategy(FixedRate::default(), 1.0);
        let context = ReinforcementContext::new();
        let new_confidence = strategy.update_confidence(0.5, true, &context);
        assert!(new_confidence > 0.5);
    }
}
