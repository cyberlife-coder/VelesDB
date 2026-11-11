//! Conversion from `VelesQL` conditions to filter conditions.

use super::Condition;
use serde_json::Value;

/// Converts a single `VelesQL` value into its `serde_json` equivalent.
///
/// Temporal values are converted to epoch seconds; subqueries and parameters
/// produce `Value::Null` (runtime evaluation happens in the query executor).
fn velesql_value_to_json(v: crate::velesql::Value) -> Value {
    match v {
        crate::velesql::Value::Integer(i) => Value::Number(i.into()),
        crate::velesql::Value::UnsignedInteger(u) => Value::Number(u.into()),
        crate::velesql::Value::Float(f) => Value::from(f),
        crate::velesql::Value::String(s) => Value::String(s),
        crate::velesql::Value::Boolean(b) => Value::Bool(b),
        crate::velesql::Value::Null
        | crate::velesql::Value::Parameter(_)
        | crate::velesql::Value::Subquery(_) => Value::Null,
        crate::velesql::Value::Temporal(t) => Value::Number(t.to_epoch_seconds().into()),
    }
}

/// Converts a `VelesQL` value to JSON, accepting only numeric types.
///
/// Non-numeric variants (String, Boolean, Null, etc.) produce `Value::Null`.
fn velesql_numeric_to_json(v: &crate::velesql::Value) -> Value {
    match *v {
        crate::velesql::Value::Integer(i) => Value::Number(i.into()),
        crate::velesql::Value::UnsignedInteger(u) => Value::Number(u.into()),
        crate::velesql::Value::Float(f) => Value::from(f),
        _ => Value::Null,
    }
}

/// The filter field a `VelesQL` column reads:
/// [`POINT_ID_FIELD`](super::POINT_ID_FIELD) for `id`, the point id in every
/// clause of a query, else the payload path.
fn field(column: String) -> String {
    if column == "id" {
        super::POINT_ID_FIELD.to_string()
    } else {
        column
    }
}

/// Sentinel for conditions handled externally by the query engine (vector
/// search, graph match, etc.). Uses an empty AND as the identity element.
fn engine_handled_identity() -> Condition {
    Condition::And { conditions: vec![] }
}

/// Names a `VelesQL` construct an observer *scope* filter cannot express as a
/// row-level metadata predicate — the first one found, depth-first, or `None`
/// when the whole tree lowers faithfully through [`From`].
///
/// The `From` lowering must stay total: the query engine pre-extracts
/// engine-handled conditions before lowering a WHERE clause, so
/// `VectorSearch`/`Similarity`/`GraphMatch` map to an always-true sentinel
/// and `MATCH` to a substring `Contains`. An observer scope cannot afford
/// that: a conjunct lowered to "match all" silently narrows nothing, and a
/// `MATCH` lowered to substring containment narrows by the *wrong* predicate.
/// A `$param` or subquery compared value is likewise not a policy value — it
/// would bind to the *caller's* params or lower to `NULL`, so a scope must be
/// self-contained. Callers enforcing a scope consult this first and fail
/// closed (`VELES-009`).
///
/// `CONTAINS_TEXT` is deliberately absent: it already *is* a strict substring
/// metadata predicate ([`crate::velesql::ContainsTextCondition`]), so its
/// `Contains` lowering is faithful, not a degradation.
// Ungated: `Filter::try_from_where` below consults it in every build; the
// gated re-export in `mod` is what `database::query_engine` sees.
pub(crate) fn inexpressible_scope_construct(
    condition: &crate::velesql::Condition,
) -> Option<&'static str> {
    use crate::velesql::{Condition as C, Value as V};
    const ENGINE: &str = "a vector-similarity or graph-match directive";
    const VALUE: &str = "a parameter placeholder or subquery value";
    let unbound = |v: &V| matches!(v, V::Parameter(_) | V::Subquery(_));
    match condition {
        C::VectorSearch(_)
        | C::VectorFusedSearch(_)
        | C::SparseVectorSearch(_)
        | C::Similarity(_)
        | C::GraphMatch(_) => Some(ENGINE),
        C::Match(_) => Some("a full-text MATCH"),
        C::Comparison(c) => unbound(&c.value).then_some(VALUE),
        C::In(c) => c.values.iter().any(&unbound).then_some(VALUE),
        C::Between(c) => (unbound(&c.low) || unbound(&c.high)).then_some(VALUE),
        C::Contains(c) => c.values.iter().any(&unbound).then_some(VALUE),
        C::And(l, r) | C::Or(l, r) => {
            inexpressible_scope_construct(l).or_else(|| inexpressible_scope_construct(r))
        }
        C::Not(inner) | C::Group(inner) => inexpressible_scope_construct(inner),
        C::IsNull(_) | C::Like(_) | C::ContainsText(_) | C::GeoDistance(_) | C::GeoBbox(_) => None,
    }
}

impl super::Filter {
    /// Lowers a `VelesQL` `WHERE` condition into a row-level filter for a
    /// caller that evaluates it with
    /// [`matches_point`](super::Filter::matches_point) alone — a scroll or an
    /// export, with no query engine in front to pre-extract vector, graph or
    /// full-text conditions.
    ///
    /// The infallible [`From`] lowering is unfit for that: it maps those
    /// conditions to an always-true sentinel, so a clause like
    /// `WHERE similarity(v) > 0.8` would silently match every row. This
    /// refuses exactly what an observer scope refuses, through the same check.
    ///
    /// # Errors
    ///
    /// Names the first construct (depth-first) that a row-level predicate
    /// cannot express: a vector-similarity or graph-match directive, a
    /// full-text `MATCH`, or a `$parameter` / subquery value.
    pub fn try_from_where(condition: crate::velesql::Condition) -> Result<Self, String> {
        match inexpressible_scope_construct(&condition) {
            Some(construct) => Err(format!(
                "WHERE clause contains {construct}, which cannot be evaluated \
                 as a row-level filter"
            )),
            None => Ok(Self::new(condition.into())),
        }
    }
}

/// Converts a comparison condition using the shared value converter.
fn convert_comparison(
    column: String,
    operator: crate::velesql::CompareOp,
    value: crate::velesql::Value,
) -> Condition {
    let value = velesql_value_to_json(value);
    let column = field(column);
    match operator {
        crate::velesql::CompareOp::Eq => Condition::eq(column, value),
        crate::velesql::CompareOp::NotEq => Condition::neq(column, value),
        crate::velesql::CompareOp::Gt => Condition::Gt {
            field: column,
            value,
        },
        crate::velesql::CompareOp::Gte => Condition::Gte {
            field: column,
            value,
        },
        crate::velesql::CompareOp::Lt => Condition::Lt {
            field: column,
            value,
        },
        crate::velesql::CompareOp::Lte => Condition::Lte {
            field: column,
            value,
        },
    }
}

/// Converts an IN condition, optionally negated via NOT.
///
/// [`InValues`](super::InValues) sorts by `json_value_cmp` and drops exact
/// duplicates at construction, so large IN lists can use O(log n) binary
/// search in `matching.rs::in_list_matches` (issue #512).
fn convert_in(inc: crate::velesql::InCondition) -> Condition {
    let values: Vec<Value> = inc.values.into_iter().map(velesql_value_to_json).collect();
    let in_cond = Condition::In {
        field: field(inc.column),
        values: values.into(),
    };
    if inc.negated {
        Condition::Not {
            condition: Box::new(in_cond),
        }
    } else {
        in_cond
    }
}

/// Converts a BETWEEN condition into `Gte AND Lte`.
fn convert_between(btw: crate::velesql::BetweenCondition) -> Condition {
    Condition::And {
        conditions: vec![
            Condition::Gte {
                field: field(btw.column.clone()),
                value: velesql_numeric_to_json(&btw.low),
            },
            Condition::Lte {
                field: field(btw.column),
                value: velesql_numeric_to_json(&btw.high),
            },
        ],
    }
}

/// Converts a LIKE/ILIKE condition based on case-sensitivity.
fn convert_like(lk: crate::velesql::LikeCondition) -> Condition {
    if lk.case_insensitive {
        Condition::ILike {
            field: field(lk.column),
            pattern: lk.pattern,
        }
    } else {
        Condition::Like {
            field: field(lk.column),
            pattern: lk.pattern,
        }
    }
}

/// Converts CONTAINS (single/any/all) array conditions.
fn convert_contains(cc: crate::velesql::ContainsCondition) -> Condition {
    let json_values: Vec<Value> = cc.values.into_iter().map(velesql_value_to_json).collect();
    match cc.mode {
        crate::velesql::ContainsMode::Single => Condition::ArrayContains {
            field: field(cc.column),
            value: json_values.into_iter().next().unwrap_or(Value::Null),
        },
        crate::velesql::ContainsMode::Any => Condition::ArrayContainsAny {
            field: field(cc.column),
            values: json_values,
        },
        crate::velesql::ContainsMode::All => Condition::ArrayContainsAll {
            field: field(cc.column),
            values: json_values,
        },
    }
}

/// Converts `IS NULL` / `IS NOT NULL`.
fn convert_is_null(isn: crate::velesql::IsNullCondition) -> Condition {
    let field = field(isn.column);
    if isn.is_null {
        Condition::IsNull { field }
    } else {
        Condition::IsNotNull { field }
    }
}

impl From<crate::velesql::Condition> for Condition {
    fn from(cond: crate::velesql::Condition) -> Self {
        match cond {
            crate::velesql::Condition::Comparison(cmp) => {
                convert_comparison(cmp.column, cmp.operator, cmp.value)
            }
            crate::velesql::Condition::In(inc) => convert_in(inc),
            crate::velesql::Condition::IsNull(isn) => convert_is_null(isn),
            crate::velesql::Condition::And(left, right) => Self::And {
                conditions: vec![Self::from(*left), Self::from(*right)],
            },
            crate::velesql::Condition::Or(left, right) => Self::Or {
                conditions: vec![Self::from(*left), Self::from(*right)],
            },
            crate::velesql::Condition::Not(inner) => Self::Not {
                condition: Box::new(Self::from(*inner)),
            },
            crate::velesql::Condition::Group(inner) => Self::from(*inner),
            crate::velesql::Condition::VectorSearch(_)
            | crate::velesql::Condition::VectorFusedSearch(_)
            | crate::velesql::Condition::SparseVectorSearch(_)
            | crate::velesql::Condition::Similarity(_)
            | crate::velesql::Condition::GraphMatch(_) => engine_handled_identity(),
            crate::velesql::Condition::Match(m) => Self::Contains {
                field: field(m.column),
                value: m.query,
            },
            crate::velesql::Condition::ContainsText(ct) => Self::Contains {
                field: field(ct.column),
                value: ct.query,
            },
            crate::velesql::Condition::Between(btw) => convert_between(btw),
            crate::velesql::Condition::Like(lk) => convert_like(lk),
            crate::velesql::Condition::Contains(cc) => convert_contains(cc),
            crate::velesql::Condition::GeoDistance(gd) => Self::GeoDistance {
                field: field(gd.column),
                lat: gd.lat,
                lng: gd.lng,
                operator: gd.operator,
                threshold: gd.threshold,
            },
            crate::velesql::Condition::GeoBbox(gb) => Self::GeoBbox {
                field: field(gb.column),
                lat_min: gb.lat_min,
                lng_min: gb.lng_min,
                lat_max: gb.lat_max,
                lng_max: gb.lng_max,
            },
        }
    }
}

// Tests moved to conversion_tests.rs per project rules
