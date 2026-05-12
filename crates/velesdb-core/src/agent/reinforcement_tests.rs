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

    // --- DiminishingReturns (Phase 2) ---

    #[test]
    fn test_diminishing_returns_first_success_equals_fixed_rate() {
        // At success_count=0 the delta should equal base_success_delta.
        let strategy = DiminishingReturns::default();
        let mut ctx = ReinforcementContext::new();
        ctx.custom.insert("success_count".to_string(), 0.0);
        ctx.custom.insert("failure_count".to_string(), 0.0);
        let new_confidence = strategy.update_confidence(0.5, true, &ctx);
        // delta = 0.1 / (1 + 0.1*0) = 0.1 → 0.6
        assert!((new_confidence - 0.6).abs() < 0.001, "got {new_confidence}");
    }

    #[test]
    fn test_diminishing_returns_decreases_with_practice() {
        let strategy = DiminishingReturns::default();
        let mut ctx0 = ReinforcementContext::new();
        ctx0.custom.insert("success_count".to_string(), 0.0);
        ctx0.custom.insert("failure_count".to_string(), 0.0);
        let mut ctx10 = ReinforcementContext::new();
        ctx10.custom.insert("success_count".to_string(), 10.0);
        ctx10.custom.insert("failure_count".to_string(), 0.0);

        let d0 = strategy.update_confidence(0.5, true, &ctx0) - 0.5;
        let d10 = strategy.update_confidence(0.5, true, &ctx10) - 0.5;
        assert!(
            d10 < d0,
            "delta should decrease with practice: {d10} < {d0}"
        );
    }

    #[test]
    fn test_diminishing_returns_failure_decrements() {
        let strategy = DiminishingReturns::default();
        let mut ctx = ReinforcementContext::new();
        ctx.custom.insert("success_count".to_string(), 0.0);
        ctx.custom.insert("failure_count".to_string(), 0.0);
        let new_confidence = strategy.update_confidence(0.5, false, &ctx);
        assert!(new_confidence < 0.5, "failure should lower confidence");
    }

    #[test]
    fn test_diminishing_returns_clamp() {
        let strategy = DiminishingReturns::new(0.5, 0.5, 0.0);
        let mut ctx = ReinforcementContext::new();
        ctx.custom.insert("success_count".to_string(), 0.0);
        ctx.custom.insert("failure_count".to_string(), 0.0);
        let high = strategy.update_confidence(0.9, true, &ctx);
        assert!(high <= 1.0, "should not exceed 1.0");
        let low = strategy.update_confidence(0.1, false, &ctx);
        assert!(low >= 0.0, "should not go below 0.0");
    }

    // --- power_law_decay (Phase 1) ---

    #[test]
    fn test_power_law_decay_zero_elapsed_no_decay() {
        // 0 seconds elapsed → days=0, max(1,0)=1, 1^(-0.5)=1 → confidence unchanged
        let result = power_law_decay(0.8, 0, 0.5);
        assert!((result - 0.8).abs() < 0.001, "got {result}");
    }

    #[test]
    fn test_power_law_decay_four_days_halves_with_d05() {
        // 4 days → max(1, 4)^(-0.5) = 4^(-0.5) = 0.5 → confidence × 0.5
        let secs = 4 * 24 * 3600;
        let result = power_law_decay(1.0, secs, 0.5);
        assert!((result - 0.5).abs() < 0.001, "got {result}");
    }

    #[test]
    fn test_power_law_decay_less_than_one_day_no_extra_decay() {
        // Less than 1 day → max(1, t_days) clamps to 1 → multiplier = 1^(-d) = 1
        let secs_12h = 12 * 3600;
        let result = power_law_decay(0.7, secs_12h, 0.5);
        assert!((result - 0.7).abs() < 0.001, "got {result}");
    }

    #[test]
    fn test_power_law_decay_monotone_decreasing() {
        let d = 0.5;
        let c = 1.0;
        let r1d = power_law_decay(c, 86_400, d); // 1 day
        let r4d = power_law_decay(c, 4 * 86_400, d); // 4 days
        let r16d = power_law_decay(c, 16 * 86_400, d); // 16 days
        assert!(r1d >= r4d, "decay should increase with time");
        assert!(r4d >= r16d, "decay should increase with time");
    }
}
