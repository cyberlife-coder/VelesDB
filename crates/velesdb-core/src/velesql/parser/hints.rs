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

/// True when `a[i]`/`b[j]` form an adjacent transposition (e.g. `FORM`↔`FROM`).
fn is_transposition(a: &[u8], b: &[u8], i: usize, j: usize) -> bool {
    i > 0 && j > 0 && a[i] == b[j - 1] && a[i - 1] == b[j]
}

/// Damerau-Levenshtein (optimal string alignment) distance between two byte
/// slices. Counts an adjacent transposition as 1 so a single keyword typo —
/// substitution, insertion, deletion, or swap — stays within distance 1.
fn edit_distance(a: &[u8], b: &[u8]) -> usize {
    let n = b.len();
    let mut prev2 = vec![0usize; n + 1];
    let mut prev: Vec<usize> = (0..=n).collect();
    let mut cur = vec![0usize; n + 1];
    for (i, &ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            let mut best = (prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1);
            if is_transposition(a, b, i, j) {
                best = best.min(prev2[j - 1] + 1);
            }
            cur[j + 1] = best;
        }
        std::mem::swap(&mut prev2, &mut prev);
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[n]
}

/// Suggests the closest keyword to the word at `position` (edit distance ≤ 1),
/// unless that word already *is* a keyword (then it is misused, not misspelled).
///
/// The tight threshold avoids suggesting keywords for ordinary identifiers that
/// merely resemble one (e.g. a column named `user` is *not* a typo of `UPSERT`).
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
        .filter(|(_, distance)| *distance <= 1)
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
}
