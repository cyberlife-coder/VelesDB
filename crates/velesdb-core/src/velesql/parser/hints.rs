//! Friendlier syntax-error messages for the `VelesQL` parser.
//!
//! pest's raw error already renders a caret diagram, but it lists grammar rule
//! names (`expected select_stmt, match_query, …`) that mean nothing to a user.
//! These helpers prepend a plain-language summary and a "did you mean" keyword
//! suggestion, while keeping pest's diagram underneath.

/// Keywords used for typo ("did you mean") suggestions at the failure point.
const KEYWORDS: &[&str] = &[
    "SELECT",
    "FROM",
    "WHERE",
    "LIMIT",
    "OFFSET",
    "ORDER",
    "GROUP",
    "HAVING",
    "INSERT",
    "UPSERT",
    "UPDATE",
    "DELETE",
    "INTO",
    "VALUES",
    "CREATE",
    "DROP",
    "COLLECTION",
    "INDEX",
    "MATCH",
    "RETURN",
    "EXPLAIN",
    "ANALYZE",
    "TRUNCATE",
    "FLUSH",
    "DESCRIBE",
    "DISTINCT",
    "JOIN",
    "UNION",
    "INTERSECT",
    "EXCEPT",
    "BETWEEN",
    "LIKE",
    "ILIKE",
    "NEAR",
];

/// Returns the identifier-like word starting at `position`, if any.
fn word_at(input: &str, position: usize) -> Option<&str> {
    let rest = input.get(position..)?;
    let end = rest
        .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .unwrap_or(rest.len());
    let word = &rest[..end];
    (!word.is_empty()).then_some(word)
}

/// Levenshtein distance between two byte slices (used only for short keywords).
fn edit_distance(a: &[u8], b: &[u8]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, &ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Suggests the closest keyword to the word at `position` (edit distance ≤ 2),
/// unless that word already *is* a keyword (then it is misused, not misspelled).
fn did_you_mean(input: &str, position: usize) -> Option<&'static str> {
    let word = word_at(input, position)?;
    if word.len() < 3 {
        return None;
    }
    let upper = word.to_ascii_uppercase();
    if KEYWORDS.iter().any(|kw| kw.eq_ignore_ascii_case(word)) {
        return None;
    }
    KEYWORDS
        .iter()
        .map(|kw| (*kw, edit_distance(upper.as_bytes(), kw.as_bytes())))
        .filter(|(_, distance)| *distance <= 2)
        .min_by_key(|(_, distance)| *distance)
        .map(|(kw, _)| kw)
}

/// Builds an enriched message: plain-language lead + optional suggestion, then
/// pest's original diagram on the following lines.
pub(super) fn enrich_message(input: &str, position: usize, pest_message: &str) -> String {
    let lead = if position == 0 {
        "VelesQL statements must start with a keyword such as SELECT, MATCH, \
         INSERT, UPSERT, UPDATE, DELETE, CREATE, DROP, or EXPLAIN"
            .to_string()
    } else if let Some(word) = word_at(input, position) {
        format!("Unexpected syntax near '{word}'")
    } else {
        "Unexpected syntax".to_string()
    };
    let suggestion = did_you_mean(input, position)
        .map(|keyword| format!(". Did you mean `{keyword}`?"))
        .unwrap_or_default();
    format!("{lead}{suggestion}.\n{pest_message}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggests_keyword_for_simple_typo() {
        assert_eq!(did_you_mean("SELEC * FROM docs", 0), Some("SELECT"));
    }

    #[test]
    fn suggests_keyword_for_transposition() {
        // "FORM" -> "FROM" is edit distance 2.
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
}
