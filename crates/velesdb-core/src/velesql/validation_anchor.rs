//! V011 anchor rule for `MATCH` predicates in SELECT WHERE clauses.
//!
//! Separated from `validation.rs` to keep each file under the 500 NLOC limit.
//!
//! The first node of a MATCH pattern (the *anchor*) binds the pattern to the
//! FROM rows. Binding is **explicit** when the anchor alias is declared in
//! FROM/JOIN, and **implicit** when no pattern alias matches a declared alias
//! — the leftmost node then binds to the FROM rows. Implicit binding is
//! guarded by:
//! - **G1**: a declared alias in a non-anchor position means the pattern
//!   direction is inverted — the anchor must be that alias;
//! - **G2**: the implicit anchor alias must not appear in another graph
//!   predicate of the same WHERE (ambiguous — chain into a single pattern);
//! - **G3**: the anchor node must not carry a `@collection` override (its
//!   rows would come from another collection, not the FROM rows).
//!
//! The runtime mirror (`resolve_anchor_alias` in `execution_paths.rs`)
//! enforces G1/G3 only; G2 needs the whole WHERE tree and is validation-only.

use std::collections::{HashMap, HashSet};

use super::ast::{Condition, GraphMatchPredicate};
use super::graph_pattern::{Direction, GraphPattern};
use super::validation_types::{ValidationError, ValidationErrorKind};

/// Validates every `GraphMatch` predicate anchor in a SELECT WHERE condition
/// tree, after a G2 pre-pass collecting aliases shared across predicates.
pub(super) fn walk_graph_match_anchors(
    condition: &Condition,
    from_aliases: &[String],
) -> Result<(), ValidationError> {
    let shared_aliases = collect_shared_pattern_aliases(condition);
    walk_anchors(condition, from_aliases, &shared_aliases)
}

/// G2 pre-pass: node aliases referenced by two or more distinct graph
/// predicates of the WHERE tree.
fn collect_shared_pattern_aliases(condition: &Condition) -> HashSet<String> {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    accumulate_pattern_aliases(condition, &mut counts);
    counts
        .into_iter()
        .filter_map(|(alias, predicates)| (predicates >= 2).then_some(alias.to_string()))
        .collect()
}

/// Counts, per node alias, how many graph predicates reference it.
/// Keys borrow from the AST nodes so no String allocations are needed during
/// the counting phase; owned strings are produced only for aliases that make
/// the final shared set.
fn accumulate_pattern_aliases<'c>(condition: &'c Condition, counts: &mut HashMap<&'c str, usize>) {
    match condition {
        Condition::GraphMatch(predicate) => {
            let aliases: HashSet<&str> = predicate
                .pattern
                .nodes
                .iter()
                .filter_map(|node| node.alias.as_deref())
                .collect();
            for alias in aliases {
                *counts.entry(alias).or_insert(0) += 1;
            }
        }
        Condition::And(l, r) | Condition::Or(l, r) => {
            accumulate_pattern_aliases(l, counts);
            accumulate_pattern_aliases(r, counts);
        }
        Condition::Not(inner) | Condition::Group(inner) => {
            accumulate_pattern_aliases(inner, counts);
        }
        _ => {}
    }
}

/// Recursive walk checking each `GraphMatch` predicate anchor.
fn walk_anchors(
    condition: &Condition,
    from_aliases: &[String],
    shared_aliases: &HashSet<String>,
) -> Result<(), ValidationError> {
    match condition {
        Condition::GraphMatch(predicate) => {
            check_graph_match_anchor(predicate, from_aliases, shared_aliases)
        }
        Condition::And(l, r) | Condition::Or(l, r) => {
            walk_anchors(l, from_aliases, shared_aliases)?;
            walk_anchors(r, from_aliases, shared_aliases)
        }
        Condition::Not(inner) | Condition::Group(inner) => {
            walk_anchors(inner, from_aliases, shared_aliases)
        }
        _ => Ok(()),
    }
}

/// Checks a single MATCH predicate's anchor (first node) alias against the
/// explicit-or-implicit binding rule.
fn check_graph_match_anchor(
    predicate: &GraphMatchPredicate,
    from_aliases: &[String],
    shared_aliases: &HashSet<String>,
) -> Result<(), ValidationError> {
    let nodes = &predicate.pattern.nodes;
    let Some(anchor) = nodes.first().and_then(|node| node.alias.as_deref()) else {
        return Err(v011(
            "MATCH (...)",
            "MATCH in SELECT WHERE requires an alias on the first node, \
             e.g. MATCH (d:Doc)-[:REL]->(x)",
        ));
    };
    // Explicit anchor — or a bare FROM, where any anchor is accepted.
    if from_aliases.is_empty() || from_aliases.iter().any(|a| a == anchor) {
        return Ok(());
    }
    check_implicit_anchor(predicate, anchor, from_aliases, shared_aliases)
}

/// G1/G2/G3 guards for an anchor alias that is not declared in FROM/JOIN
/// (implicit binding candidate).
fn check_implicit_anchor(
    predicate: &GraphMatchPredicate,
    anchor: &str,
    from_aliases: &[String],
    shared_aliases: &HashSet<String>,
) -> Result<(), ValidationError> {
    check_g1_inverted_anchor(predicate, anchor, from_aliases)?;
    let nodes = &predicate.pattern.nodes;
    // G3: a @collection anchor resolves outside the FROM collection and
    // cannot bind implicitly to its rows.
    if nodes.first().is_some_and(|node| node.collection.is_some()) {
        return Err(v011(
            format!("MATCH ({anchor}@...)"),
            format!(
                "MATCH anchor alias '{anchor}' carries a @collection override and \
                 cannot bind implicitly to the FROM rows. Anchor the pattern on a \
                 declared alias, e.g. {rewritten}",
                rewritten =
                    render_pattern_with_anchor(&predicate.pattern, &from_aliases[0], anchor)
            ),
        ));
    }
    // G2: the implicit anchor must be unambiguous across graph predicates.
    if shared_aliases.contains(anchor) {
        return Err(v011(
            format!("MATCH ({anchor})"),
            format!(
                "implicit MATCH anchor '{anchor}' also appears in another MATCH \
                 predicate of this WHERE clause; chain into a single pattern \
                 instead, e.g. MATCH (m)-[:R]->(f)-[:S]->(g)"
            ),
        ));
    }
    // Implicit anchor: the leftmost node binds to the FROM rows.
    Ok(())
}

/// G1: a declared alias elsewhere in the pattern means the anchor must be
/// that alias (the pattern direction is likely inverted).
fn check_g1_inverted_anchor(
    predicate: &GraphMatchPredicate,
    anchor: &str,
    from_aliases: &[String],
) -> Result<(), ValidationError> {
    let declared = predicate
        .pattern
        .nodes
        .iter()
        .skip(1)
        .filter_map(|node| node.alias.as_deref())
        .find(|alias| from_aliases.iter().any(|f| f == alias));
    let Some(declared) = declared else {
        return Ok(());
    };
    Err(v011(
        format!("MATCH ({anchor})"),
        format!(
            "MATCH anchor alias '{anchor}' is not declared in FROM/JOIN while \
             '{declared}' is. Anchor the pattern on '{declared}', \
             e.g. {rewritten}",
            rewritten = render_pattern_with_anchor(&predicate.pattern, declared, anchor)
        ),
    ))
}

/// Builds a V011 validation error.
fn v011(fragment: impl Into<String>, suggestion: impl Into<String>) -> ValidationError {
    ValidationError::new(
        ValidationErrorKind::GraphMatchAnchorMismatch,
        None,
        fragment,
        suggestion,
    )
}

/// Renders the user's actual pattern re-anchored on `anchor`, displacing any
/// later occurrence of that alias to `displaced` — e.g. for
/// `MATCH (ctx)-[:RELATES_TO]->(memory)` with FROM alias `memory`, yields
/// `MATCH (memory)-[:RELATES_TO]->(ctx)`.
fn render_pattern_with_anchor(pattern: &GraphPattern, anchor: &str, displaced: &str) -> String {
    use std::fmt::Write;

    let mut out = format!("MATCH ({anchor})");
    for (rel, node) in pattern
        .relationships
        .iter()
        .zip(pattern.nodes.iter().skip(1))
    {
        let types = if rel.types.is_empty() {
            String::new()
        } else {
            format!(":{}", rel.types.join("|"))
        };
        let range = rel
            .range
            .map(|(lo, hi)| format!("*{lo}..{hi}"))
            .unwrap_or_default();
        let body = format!("[{types}{range}]");
        let arrow = match rel.direction {
            Direction::Outgoing => format!("-{body}->"),
            Direction::Incoming => format!("<-{body}-"),
            Direction::Both => format!("-{body}-"),
        };
        let alias = match node.alias.as_deref() {
            Some(a) if a == anchor => displaced,
            Some(a) => a,
            None => "",
        };
        // Infallible: `write!` into a `String` never errors.
        let _ = write!(out, "{arrow}({alias})");
    }
    out
}
