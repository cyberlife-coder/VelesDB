//! The ONE table every [`MemoryStore`](crate::storage::MemoryStore) backend
//! must satisfy for [`ColumnFilter`], and the fixture it runs against.
//!
//! Why it is shared rather than written twice: the native backend translates
//! `ColumnFilter` into `VelesQL` while the WASM backend tests the payload
//! directly, so the two evaluate the same public filter through completely
//! different machinery. Two parallel test files would drift — and did: `ne`
//! on an ABSENT field matched natively and never matched on WASM, for the
//! whole life of the API, because nothing ever compared them (#1759).
//!
//! Both backends therefore import [`fixture`] and [`cases`] from here. A case
//! added once is enforced on both, or neither.
//!
//! # The contract for [`ColumnOp::Ne`]
//!
//! `field != target` matches only when the field is **present**, its value is
//! **not null**, and that value **differs** from the target.
//!
//! | field state | `field != target` |
//! |---|---|
//! | absent | does not match |
//! | present, `null` | does not match |
//! | present, equal to target | does not match |
//! | present, different from target | matches |
//!
//! This mirrors SQL, where a comparison against `NULL` is never true.
//! [`Condition::IsNull`](velesdb_core::filter::Condition) and its `IsNotNull`
//! twin remain the operators dedicated to null-ness — `Ne` is not one of them.

use crate::model::{column_value_matches, ColumnFilter, ColumnOp};
use crate::service::Metadata;
use serde_json::Value;

/// Fixture id whose `status` field is absent entirely.
pub const ABSENT: u64 = 1;
/// Fixture id whose `status` is present and explicitly `null`.
pub const NULL_VALUED: u64 = 2;
/// Fixture id whose `status` equals the target every case compares against.
pub const EQUAL: u64 = 3;
/// Fixture id whose `status` differs from that target.
pub const DIFFERENT: u64 = 4;
/// Differing `status`, plus a `year` below the ordering cases' pivot.
pub const DIFFERENT_EARLY: u64 = 5;
/// Differing `status`, plus a `year` above that pivot.
pub const DIFFERENT_LATE: u64 = 6;
/// `year` exactly on the pivot, with NO `status` — pins that a second filter
/// still excludes a fact the first one would have kept.
pub const YEAR_ONLY: u64 = 7;
/// Internal scaffolding, which no case may ever return.
pub const SCAFFOLDING: u64 = 90;

/// The value every `status` case compares against.
pub const TARGET: &str = "archived";
/// The value the ordering cases pivot on.
pub const PIVOT: i64 = 2010;

/// One fact the conformance fixture stores.
pub struct FixtureFact {
    /// Stable id, so a case names the facts it expects by constant.
    pub id: u64,
    /// Stored content — identical across facts, since no case reads it.
    pub content: &'static str,
    /// The payload whose fields the filters address.
    pub metadata: Metadata,
}

/// One conformance case: filters applied to [`fixture`], and the ids that must
/// come back, ascending.
pub struct Case {
    /// Name carried into the assertion message, so a failure says WHICH rule broke.
    pub name: &'static str,
    /// Filters to pass to `query_columnar`, AND-combined.
    pub filters: Vec<ColumnFilter>,
    /// Ids the backend must return, ascending.
    pub expected: Vec<u64>,
}

fn meta(pairs: &[(&str, Value)]) -> Metadata {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_string(), value.clone()))
        .collect()
}

fn filter(field: &str, op: ColumnOp, value: Value) -> ColumnFilter {
    ColumnFilter {
        field: field.to_string(),
        op,
        value,
    }
}

/// Every fact both backends store before running [`cases`].
///
/// [`SCAFFOLDING`] is deliberately in here: a backend that stopped excluding
/// internal scaffolding would return it, and every case would fail — which is
/// how the exclusion is kept alive by this suite rather than by a separate
/// test that could be deleted without anyone noticing.
#[must_use]
pub fn fixture() -> Vec<FixtureFact> {
    vec![
        FixtureFact {
            id: ABSENT,
            content: "status absent",
            metadata: Metadata::new(),
        },
        FixtureFact {
            id: NULL_VALUED,
            content: "status null",
            metadata: meta(&[("status", Value::Null)]),
        },
        FixtureFact {
            id: EQUAL,
            content: "status equals the target",
            metadata: meta(&[("status", Value::from(TARGET))]),
        },
        FixtureFact {
            id: DIFFERENT,
            content: "status differs",
            metadata: meta(&[("status", Value::from("active"))]),
        },
        FixtureFact {
            id: DIFFERENT_EARLY,
            content: "status differs, early year",
            metadata: meta(&[
                ("status", Value::from("active")),
                ("year", Value::from(2003)),
            ]),
        },
        FixtureFact {
            id: DIFFERENT_LATE,
            content: "status differs, late year",
            metadata: meta(&[
                ("status", Value::from("active")),
                ("year", Value::from(2020)),
            ]),
        },
        FixtureFact {
            id: YEAR_ONLY,
            content: "year only, no status",
            metadata: meta(&[("year", Value::from(PIVOT))]),
        },
        FixtureFact {
            id: SCAFFOLDING,
            content: "internal scaffolding",
            metadata: meta(&[("_veles_hub", Value::Bool(true))]),
        },
    ]
}

/// Every rule both backends must satisfy, run against [`fixture`].
#[must_use]
pub fn cases() -> Vec<Case> {
    vec![
        Case {
            name: "ne excludes absent, null and equal; keeps only present-and-different",
            filters: vec![filter("status", ColumnOp::Ne, Value::from(TARGET))],
            expected: vec![DIFFERENT, DIFFERENT_EARLY, DIFFERENT_LATE],
        },
        Case {
            name: "eq keeps only the exact match",
            filters: vec![filter("status", ColumnOp::Eq, Value::from(TARGET))],
            expected: vec![EQUAL],
        },
        Case {
            name: "eq on an absent field matches nothing",
            filters: vec![filter("missing", ColumnOp::Eq, Value::from(TARGET))],
            expected: vec![],
        },
        Case {
            name: "lt requires the field and compares below the pivot",
            filters: vec![filter("year", ColumnOp::Lt, Value::from(PIVOT))],
            expected: vec![DIFFERENT_EARLY],
        },
        Case {
            name: "le includes the pivot",
            filters: vec![filter("year", ColumnOp::Le, Value::from(PIVOT))],
            expected: vec![DIFFERENT_EARLY, YEAR_ONLY],
        },
        Case {
            name: "gt excludes the pivot",
            filters: vec![filter("year", ColumnOp::Gt, Value::from(PIVOT))],
            expected: vec![DIFFERENT_LATE],
        },
        Case {
            name: "ge includes the pivot",
            filters: vec![filter("year", ColumnOp::Ge, Value::from(PIVOT))],
            expected: vec![DIFFERENT_LATE, YEAR_ONLY],
        },
        Case {
            name: "several filters are AND-combined",
            filters: vec![
                filter("status", ColumnOp::Ne, Value::from(TARGET)),
                filter("year", ColumnOp::Ge, Value::from(PIVOT)),
            ],
            expected: vec![DIFFERENT_LATE],
        },
        Case {
            name: "no filter returns every caller fact and no scaffolding",
            filters: Vec::new(),
            expected: vec![
                ABSENT,
                NULL_VALUED,
                EQUAL,
                DIFFERENT,
                DIFFERENT_EARLY,
                DIFFERENT_LATE,
                YEAR_ONLY,
            ],
        },
    ]
}

/// How a backend evaluates `Ne`, for the positive control below.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NeSemantics {
    /// The contract this module documents.
    Contract,
    /// The native backend's behaviour before #1759: an ABSENT field matched.
    AbsentMatched,
    /// Both backends' behaviour before #1759: an explicit `null` matched.
    NullMatched,
}

/// The contract's own decision, with `semantics` overriding only the two
/// answers this change replaced.
///
/// The per-value rule is [`column_value_matches`] — the same function both
/// backends run. Re-implementing it here would defeat the purpose: a control
/// that drifted from the rule it guards would certify a backend against a
/// contract nobody enforces.
fn matches(metadata: &Metadata, one: &ColumnFilter, semantics: NeSemantics) -> bool {
    let stored = metadata.get(&one.field);
    match (one.op, semantics, stored) {
        (ColumnOp::Ne, NeSemantics::AbsentMatched, None) => true,
        (ColumnOp::Ne, NeSemantics::NullMatched, Some(value)) if value.is_null() => true,
        (_, _, Some(value)) => column_value_matches(value, one.op, &one.value),
        (_, _, None) => false,
    }
}

/// Names of the cases a backend evaluating `Ne` with `semantics` would FAIL.
///
/// This is the suite's positive control. `Contract` must yield an empty list;
/// every other variant must yield a non-empty one, which is what proves the
/// table can still tell the contract apart from the behaviour it replaced. A
/// table that accepted the old behaviour too would be green and worthless.
#[must_use]
pub fn cases_failed_under(semantics: NeSemantics) -> Vec<&'static str> {
    let facts = fixture();
    cases()
        .into_iter()
        .filter(|case| {
            let got: Vec<u64> = facts
                .iter()
                .filter(|fact| fact.id != SCAFFOLDING)
                .filter(|fact| {
                    case.filters
                        .iter()
                        .all(|one| matches(&fact.metadata, one, semantics))
                })
                .map(|fact| fact.id)
                .collect();
            got != case.expected
        })
        .map(|case| case.name)
        .collect()
}
