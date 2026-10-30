//! V015: MATCH `RETURN ... ORDER BY` forms the MATCH executor cannot sort by.
//!
//! The executor (`Collection::order_match_results`, in
//! `collection/search/query/match_exec/order_by.rs`) sorts by `similarity()`,
//! `similarity(field, $v)`, arithmetic over a property (`year - 2000`),
//! `depth`, and an `alias.property` path. It refuses everything else with
//! `VELES-018` — but only once the pattern has been traversed, and before this
//! rule a query carrying such a form passed validation to get there. Refusing
//! the same forms here answers the caller before any work is done.
//!
//! The two refusal sets must stay equal: this rule refusing less lets the
//! query reach the executor's error again, refusing more turns away a query
//! the engine can run. `collection/match_order_by_validation_parity_tests.rs`
//! runs every form through both and fails when they disagree.

use super::ast::OrderByExpr;
use super::graph_pattern::MatchClause;
use super::validation_types::{ValidationError, ValidationErrorKind};

/// What a refused query can order by instead, named in every V015 error.
const SUPPORTED_FORMS: &str = "order by similarity(), similarity(field, $v), depth, \
     alias.property (e.g. d.year), or arithmetic over a property (e.g. year - 2000)";

/// Refuses a MATCH `RETURN ... ORDER BY` item the MATCH executor cannot sort by.
///
/// # Errors
///
/// Returns [`ValidationErrorKind::UnsupportedMatchOrderBy`] (`V015`) for an
/// aggregate, or for a bare name other than `depth`: a pattern alias
/// (`ORDER BY d`), a `RETURN` alias (`ORDER BY y`), or an unqualified property
/// (`ORDER BY year`).
pub(super) fn validate_match_order_by(clause: &MatchClause) -> Result<(), ValidationError> {
    clause
        .return_clause
        .order_by
        .iter()
        .flatten()
        .try_for_each(|item| validate_item(&item.expr))
}

fn validate_item(expr: &OrderByExpr) -> Result<(), ValidationError> {
    match expr {
        OrderByExpr::Aggregate(agg) => Err(unsupported(
            format!(
                "ORDER BY {}(...)",
                format!("{:?}", agg.function_type).to_uppercase()
            ),
            format!(
                "MATCH has no GROUP BY, so an aggregate has no group to rank; {SUPPORTED_FORMS}"
            ),
        )),
        OrderByExpr::Field(name) if name != "depth" && !is_property_path(name) => Err(unsupported(
            format!("ORDER BY {name}"),
            format!(
                "'{name}' is not a property path: ORDER BY cannot name a pattern alias, a \
                 RETURN alias, or an unqualified property; {SUPPORTED_FORMS}"
            ),
        )),
        OrderByExpr::Field(_)
        | OrderByExpr::Similarity(_)
        | OrderByExpr::SimilarityBare
        | OrderByExpr::Arithmetic(_) => Ok(()),
    }
}

/// `alias.property`, split at the first dot with both halves non-empty: the
/// path the executor's `parse_property_path` resolves against a binding.
fn is_property_path(name: &str) -> bool {
    !name.contains('(')
        && name
            .split_once('.')
            .is_some_and(|(alias, property)| !alias.is_empty() && !property.is_empty())
}

fn unsupported(fragment: String, suggestion: String) -> ValidationError {
    ValidationError::new(
        ValidationErrorKind::UnsupportedMatchOrderBy,
        None,
        fragment,
        suggestion,
    )
}
