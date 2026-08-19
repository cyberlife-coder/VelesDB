use super::*;

#[test]
fn test_new() {
    let sr = ScoredResult::new(42, 0.95);
    assert_eq!(sr.id, 42);
    assert!((sr.score - 0.95).abs() < f32::EPSILON);
}

#[test]
fn test_from_tuple() {
    let sr: ScoredResult = (10, 0.5).into();
    assert_eq!(sr.id, 10);
    assert!((sr.score - 0.5).abs() < f32::EPSILON);
}

#[test]
fn test_into_tuple() {
    let sr = ScoredResult::new(7, 0.3);
    let (id, score): (u64, f32) = sr.into();
    assert_eq!(id, 7);
    assert!((score - 0.3).abs() < f32::EPSILON);
}

#[test]
fn test_from_scored_doc() {
    let sd = ScoredDoc {
        doc_id: 99,
        score: 1.5,
    };
    let sr: ScoredResult = sd.into();
    assert_eq!(sr.id, 99);
    assert!((sr.score - 1.5).abs() < f32::EPSILON);
}

#[test]
fn test_into_scored_doc() {
    let sr = ScoredResult::new(55, 2.0);
    let sd: ScoredDoc = sr.into();
    assert_eq!(sd.doc_id, 55);
    assert!((sd.score - 2.0).abs() < f32::EPSILON);
}

#[test]
fn test_vec_conversion() {
    let tuples: Vec<(u64, f32)> = vec![(1, 0.1), (2, 0.2), (3, 0.3)];
    let results: Vec<ScoredResult> = tuples.into_iter().map(ScoredResult::from).collect();
    assert_eq!(results.len(), 3);
    assert_eq!(results[0].id, 1);

    let back: Vec<(u64, f32)> = results.into_iter().map(Into::into).collect();
    assert_eq!(back.len(), 3);
    assert_eq!(back[2].0, 3);
}
