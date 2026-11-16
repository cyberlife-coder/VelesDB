use super::*;

fn scored(node_id: u64, score: Option<f32>) -> MatchResult {
    let mut result = MatchResult::new(node_id, 0, Vec::new());
    result.score = score;
    result
}

/// A NaN score ranks after every number in both directions, whatever its
/// sign.
#[test]
fn sort_by_score_ranks_nan_after_every_number() {
    for nan in [f32::NAN, -f32::NAN] {
        for higher_is_better in [true, false] {
            let mut results = vec![
                scored(1, Some(nan)),
                scored(2, Some(0.5)),
                scored(3, Some(0.7)),
            ];
            Collection::sort_by_score(&mut results, higher_is_better);
            let ids: Vec<u64> = results.iter().map(|r| r.node_id).collect();
            let expected = if higher_is_better {
                [3, 2, 1]
            } else {
                [2, 3, 1]
            };
            assert_eq!(
                ids,
                expected,
                "nan sign {}, higher_is_better {higher_is_better}",
                nan.is_sign_negative()
            );
        }
    }
}
