//! What a text `MATCH` leaf means wherever a WHERE is evaluated.
//!
//! A `MATCH` holds for a row when the row is a BM25 hit for its query: the
//! text index holds a document for the row with at least one of the query's
//! terms, read by the index's tokenizer — the rows `text_search` scores. The
//! column is not consulted: the index covers every string of the payload.
//!
//! One `MATCH` may rank a SELECT ([`TextRanking`]): the one the WHERE
//! AND-requires, beside no other ranking than one dense `NEAR`. Alone, the
//! text search ranks the rows by BM25 and returns only hits; beside a `NEAR`
//! it is a hybrid branch, a boost that selects nothing. Every other `MATCH`
//! is a test of membership in its hit set, which [`bind_text_matches`]
//! writes as an `IN` list of the hits' ids before the WHERE is evaluated —
//! so the scans, the graph post-filter, a JOIN's filters, an aggregate and a
//! MATCH statement's WHERE read the same rows, through the evaluators they
//! already have, and the hit set is computed once per query, never per row.
//!
//! A `MATCH` left in a bound WHERE is its ranking: it reads TRUE in a
//! boolean context, as a `NEAR` does (`Collection::extract_metadata_filter`
//! drops it, the WHERE evaluator answers TRUE). `validate_vector_predicates`
//! refuses the shapes where a `MATCH` would rank beside a ranking no executor
//! fuses with text.

#[cfg(feature = "persistence")]
use super::condition_tree::any_subtree;
#[cfg(feature = "persistence")]
use super::InCondition;
use super::{Condition, MatchCondition};

/// The `MATCH` a WHERE ranks its rows by, if any (module doc).
#[cfg(feature = "persistence")]
#[derive(Debug, Clone, Copy)]
pub(crate) enum TextRanking<'a> {
    /// No `MATCH` ranks the rows: every one is a membership test.
    None,
    /// The text search ranks the rows by BM25 and returns only its hits.
    Text(&'a MatchCondition),
    /// The `NEAR` and this `MATCH` are fused: a boost that selects nothing.
    Hybrid(&'a MatchCondition),
}

#[cfg(feature = "persistence")]
impl<'a> TextRanking<'a> {
    /// The ranking `condition` asks for: its one AND-required `MATCH` (under
    /// `AND` and groups from the root) ranks when the WHERE holds no other
    /// ranking than a dense `NEAR` that the dispatch fuses with it — one
    /// reached through `AND`, `OR` and groups, as the dispatch finds it.
    /// A `similarity()`, `SPARSE_NEAR` or `NEAR_FUSED` leaf, or a `NEAR`
    /// under `NOT`, routes the query to an executor that fuses no text, so
    /// every `MATCH` is then a membership test.
    pub(crate) fn of(condition: &'a Condition) -> Self {
        let mut ranked = Vec::new();
        and_required_text_matches(condition, &mut ranked);
        let [only] = ranked[..] else {
            return Self::None;
        };
        if any_subtree(condition, &|c| {
            matches!(
                c,
                Condition::Similarity(_)
                    | Condition::SparseVectorSearch(_)
                    | Condition::VectorFusedSearch(_)
            ) || matches!(c, Condition::Not(inner) if any_subtree(inner, &is_near))
        }) {
            return Self::None;
        }
        if ranking_near(condition) {
            Self::Hybrid(only)
        } else {
            Self::Text(only)
        }
    }
}

/// The `MATCH` leaves `condition` AND-requires: those reached from its root
/// through `AND` and groups only.
pub(crate) fn and_required_text_matches<'a>(
    condition: &'a Condition,
    out: &mut Vec<&'a MatchCondition>,
) {
    match condition {
        Condition::Match(leaf) => out.push(leaf),
        Condition::And(left, right) => {
            and_required_text_matches(left, out);
            and_required_text_matches(right, out);
        }
        Condition::Group(inner) => and_required_text_matches(inner, out),
        _ => {}
    }
}

/// Whether the dispatch finds a dense `NEAR` to rank by: one reached from the
/// root through `AND`, `OR` and groups (`Collection::extract_vector_search`).
pub(crate) fn ranking_near(condition: &Condition) -> bool {
    match condition {
        Condition::VectorSearch(_) => true,
        Condition::And(left, right) | Condition::Or(left, right) => {
            ranking_near(left) || ranking_near(right)
        }
        Condition::Group(inner) => ranking_near(inner),
        _ => false,
    }
}

#[cfg(feature = "persistence")]
const fn is_near(condition: &Condition) -> bool {
    matches!(condition, Condition::VectorSearch(_))
}

/// `condition` with every `MATCH` but `keep` replaced by its membership
/// test, the `IN` list `membership` builds for it (a caller names the column
/// that reads the row's id where it evaluates the WHERE, and may refuse a
/// `MATCH` it cannot read). `keep`, the WHERE's ranking, is left as written;
/// it is told apart by identity, so pass a leaf of `condition` itself.
///
/// # Errors
///
/// What `membership` returns.
#[cfg(feature = "persistence")]
pub(crate) fn bind_text_matches<E>(
    condition: &Condition,
    keep: Option<&MatchCondition>,
    membership: &mut dyn FnMut(&MatchCondition) -> std::result::Result<InCondition, E>,
) -> std::result::Result<Condition, E> {
    map_text_matches(condition, &mut |leaf| {
        if keep.is_some_and(|kept| std::ptr::eq(kept, leaf)) {
            Ok(Condition::Match(leaf.clone()))
        } else {
            membership(leaf).map(Condition::In)
        }
    })
}

/// `condition` with every `MATCH` leaf replaced by what `map` answers for
/// it, every other leaf as it is.
///
/// # Errors
///
/// What `map` returns.
#[cfg(feature = "persistence")]
pub(crate) fn map_text_matches<E>(
    condition: &Condition,
    map: &mut dyn FnMut(&MatchCondition) -> std::result::Result<Condition, E>,
) -> std::result::Result<Condition, E> {
    match condition {
        Condition::Match(leaf) => map(leaf),
        _ => map_children(condition, &mut |child| map_text_matches(child, map)),
    }
}

/// `condition` with each operand of its `AND`, `OR`, `NOT` or group replaced
/// by what `map` answers for it; a leaf as it is.
#[cfg(feature = "persistence")]
fn map_children<E>(
    condition: &Condition,
    map: &mut dyn FnMut(&Condition) -> std::result::Result<Condition, E>,
) -> std::result::Result<Condition, E> {
    let mut operand = |c: &Condition| map(c).map(Box::new);
    match condition {
        Condition::And(left, right) => {
            operand(left).and_then(|l| operand(right).map(|r| Condition::And(l, r)))
        }
        Condition::Or(left, right) => {
            operand(left).and_then(|l| operand(right).map(|r| Condition::Or(l, r)))
        }
        Condition::Not(inner) => operand(inner).map(Condition::Not),
        Condition::Group(inner) => operand(inner).map(Condition::Group),
        other => Ok(other.clone()),
    }
}

/// Whether `condition` holds a text `MATCH` anywhere.
#[cfg(feature = "persistence")]
pub(crate) fn holds_text_match(condition: &Condition) -> bool {
    any_subtree(condition, &|c| matches!(c, Condition::Match(_)))
}

#[cfg(all(test, feature = "persistence"))]
#[path = "text_match_tests.rs"]
mod tests;
