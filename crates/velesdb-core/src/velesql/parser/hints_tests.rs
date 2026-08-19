use super::*;

#[test]
fn suggests_keyword_for_simple_typo() {
    assert_eq!(did_you_mean("SELEC * FROM docs", 0), Some("SELECT"));
}

#[test]
fn suggests_keyword_for_transposition() {
    // "FORM" -> "FROM" is an adjacent transposition (Damerau distance 1).
    assert_eq!(did_you_mean("SELECT * FORM docs", 9), Some("FROM"));
}

#[test]
fn no_suggestion_for_correct_keyword() {
    assert_eq!(did_you_mean("FROM docs", 0), None);
}

#[test]
fn no_suggestion_for_short_word() {
    assert_eq!(did_you_mean("ab cd", 0), None);
}

#[test]
fn no_suggestion_for_ordinary_identifier() {
    // Common column/collection names are distance 2 from a keyword and must
    // NOT be reported as typos (regression: user->UPSERT, date->UPDATE).
    assert_eq!(did_you_mean("user", 0), None);
    assert_eq!(did_you_mean("date", 0), None);
    assert_eq!(did_you_mean("main", 0), None);
}

#[test]
fn enrich_at_start_mentions_keywords_and_keeps_diagram() {
    let msg = enrich_message("SELEC * FROM docs", 0, "<pest diagram>");
    assert!(msg.contains("must start with a keyword"));
    assert!(msg.contains("Did you mean `SELECT`?"));
    assert!(msg.contains("<pest diagram>"));
}

#[test]
fn enrich_midquery_points_at_word() {
    let msg = enrich_message("SELECT * docs", 9, "<diagram>");
    assert!(msg.contains("Unexpected syntax near 'docs'"));
    assert!(msg.contains("<diagram>"));
}
