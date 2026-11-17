//! The score order of a browser MATCH, before any RETURN ORDER BY.

use super::*;

fn scored(node_id: u64, score: Option<f32>) -> MatchCandidate {
    let value = serde_json::json!({});
    MatchCandidate {
        baseline: vec![node_id],
        row: QueryResultRow::synthetic(value.clone()).expect("test: row"),
        value,
        binding: MatchBinding::default(),
        score,
    }
}

/// A NaN or absent score ranks after every number in both directions,
/// whatever the NaN's sign, as in core's MATCH.
#[test]
fn a_nan_or_absent_score_ranks_after_every_number() {
    for nan in [f32::NAN, -f32::NAN] {
        for higher_is_better in [true, false] {
            let mut rows = vec![
                scored(1, Some(nan)),
                scored(2, Some(0.5)),
                scored(3, None),
                scored(4, Some(0.7)),
            ];
            sort_by_score(&mut rows, higher_is_better);
            let ids: Vec<u64> = rows.iter().map(|r| r.baseline[0]).collect();
            let expected = if higher_is_better {
                [4, 2, 1, 3]
            } else {
                [2, 4, 1, 3]
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
