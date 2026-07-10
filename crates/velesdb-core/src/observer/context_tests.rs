//! Unit tests for the control-plane decision types.
//!
//! Covers construction of every [`AccessDecision`] and [`QueryOperationKind`]
//! variant, exhaustive-style matching, [`AccessScope`] defaulting, and building
//! a [`QueryAccessContext`].
//!
//! # A note on `#[non_exhaustive]` and exhaustive matches
//!
//! Every public type here is `#[non_exhaustive]`. Within the *defining* crate
//! (`velesdb-core`), `#[non_exhaustive]` imposes no restriction: an in-crate
//! `match` may still be written exhaustively without a wildcard arm, and doing
//! so is desirable — it makes the compiler force this test to be updated
//! whenever a new variant is added, documenting the full variant set.
//!
//! From an *external* crate (e.g. `velesdb-premium`), the same `match` would
//! fail to compile without a trailing wildcard (`_ => ...`) arm, because the
//! compiler cannot prove the match covers every future variant. That
//! external-crate wildcard requirement is exactly the forward-compatibility
//! guarantee `#[non_exhaustive]` provides (Requirement 3.3). We deliberately do
//! not (and cannot) create an external crate here, so this in-crate exhaustive
//! match stands in as the documented contract.

use super::{AccessDecision, AccessScope, QueryAccessContext, QueryOperationKind};
use crate::velesql::{CompareOp, Comparison, Condition, Value};

/// Classifies an `AccessDecision` via an exhaustive (wildcard-free) match.
///
/// The absence of a `_ =>` arm is intentional: adding a new `AccessDecision`
/// variant will break this function's compilation, forcing the variant set to
/// stay documented here.
fn classify(decision: &AccessDecision) -> &'static str {
    match decision {
        AccessDecision::Allow => "allow",
        AccessDecision::Deny(_) => "deny",
        AccessDecision::AllowWithScope(_) => "allow_with_scope",
    }
}

#[test]
fn access_decision_allow_can_be_constructed_and_matched() {
    let decision = AccessDecision::Allow;
    assert_eq!(classify(&decision), "allow");
}

#[test]
fn access_decision_deny_carries_the_supplied_error() {
    let decision = AccessDecision::Deny(crate::Error::Query("denied".to_string()));
    assert_eq!(classify(&decision), "deny");

    match decision {
        AccessDecision::Deny(err) => {
            assert!(err.to_string().contains("denied"));
        }
        other => panic!("expected Deny, got {other:?}"),
    }
}

#[test]
fn access_decision_allow_with_scope_carries_the_scope() {
    let scope = AccessScope {
        tenant: Some("tenant-a".to_string()),
        filter: Some(Condition::Comparison(Comparison {
            column: "org".to_string(),
            operator: CompareOp::Eq,
            value: Value::String("acme".to_string()),
        })),
    };
    let decision = AccessDecision::AllowWithScope(scope);
    assert_eq!(classify(&decision), "allow_with_scope");

    match decision {
        AccessDecision::AllowWithScope(scope) => {
            assert_eq!(scope.tenant.as_deref(), Some("tenant-a"));
            assert!(matches!(scope.filter, Some(Condition::Comparison(_))));
        }
        other => panic!("expected AllowWithScope, got {other:?}"),
    }
}

#[test]
fn query_operation_kind_all_variants_construct_and_compare() {
    // Exhaustive, wildcard-free listing of every variant. Adding a variant
    // breaks this and forces the set to stay documented.
    let all = [
        QueryOperationKind::VectorSearch,
        QueryOperationKind::TextSearch,
        QueryOperationKind::HybridSearch,
        QueryOperationKind::GraphTraversal,
        QueryOperationKind::Select,
    ];

    // Copy + Eq behavior: each variant equals itself and differs from others.
    for (i, a) in all.iter().enumerate() {
        for (j, b) in all.iter().enumerate() {
            assert_eq!(*a == *b, i == j, "variant equality mismatch at {i},{j}");
        }
    }
}

#[test]
fn query_operation_kind_is_copy() {
    let kind = QueryOperationKind::HybridSearch;
    let copied = kind; // Copy, not move
    assert_eq!(kind, copied);
}

#[test]
fn access_scope_default_is_fully_unset() {
    let scope = AccessScope::default();
    assert!(scope.tenant.is_none());
    assert!(scope.filter.is_none());
}

#[test]
fn query_access_context_borrows_its_fields() {
    let collection = "documents".to_string();
    let principal = "user-42".to_string();
    let tenant = "tenant-a".to_string();

    let ctx = QueryAccessContext {
        collection: &collection,
        operation: QueryOperationKind::Select,
        principal: Some(&principal),
        tenant_hint: Some(&tenant),
    };

    assert_eq!(ctx.collection, "documents");
    assert_eq!(ctx.operation, QueryOperationKind::Select);
    assert_eq!(ctx.principal, Some("user-42"));
    assert_eq!(ctx.tenant_hint, Some("tenant-a"));
}

#[test]
fn query_access_context_optional_hints_can_be_absent() {
    let collection = "vectors".to_string();

    let ctx = QueryAccessContext {
        collection: &collection,
        operation: QueryOperationKind::VectorSearch,
        principal: None,
        tenant_hint: None,
    };

    assert_eq!(ctx.collection, "vectors");
    assert_eq!(ctx.operation, QueryOperationKind::VectorSearch);
    assert!(ctx.principal.is_none());
    assert!(ctx.tenant_hint.is_none());
}
