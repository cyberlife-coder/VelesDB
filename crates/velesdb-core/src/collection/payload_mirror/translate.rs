//! Translation of canonical filter conditions into `ColumnStore` bitmaps.
//!
//! # Soundness contract
//!
//! The caller post-filters candidates with `Filter::matches`, so **false
//! positives are harmless but false negatives are not**. Every bitmap
//! returned here must therefore contain *at least* every live row the JSON
//! filter would match. Leaves achieve this by being *exact* replicas of the
//! `filter::matching` semantics (same `f64` epsilon equality, same
//! cross-type-never-matches rules), enforced through strict eligibility:
//!
//! - A leaf is only translated when the literal's JSON type matches the
//!   mirrored column's type. Cross-type comparisons are `false` in JSON
//!   semantics for rows of the column's type, but rows holding a value of
//!   *another* type are stored as nulls in the mirror and could still match
//!   in JSON — those leaves return `None` (fall back to the JSON scan).
//! - A field with no column and never seen as a scalar provably matches
//!   nothing (`get_field` misses or hits a non-scalar) — exact empty bitmap.
//! - Fields capped out of the mirror (`uncolumnized`) return `None`.
//!
//! Inexact (superset) results only arise from `And` branches whose siblings
//! were untranslatable; `exact` tracks this so `Not`/`Neq` complements are
//! only ever taken over exact operands (a complemented superset would lose
//! matches — a false negative).

use super::MirrorState;
use crate::column_store::TypedColumn;
use crate::filter::Condition;
use roaring::RoaringBitmap;

/// A translated condition: matching live rows plus exactness.
pub(super) struct Eval {
    pub(super) bits: RoaringBitmap,
    /// `true` when `bits` is exactly the JSON-filter match set; required for
    /// complement operations (`Not`, `Neq`).
    pub(super) exact: bool,
}

impl Eval {
    fn exact(bits: RoaringBitmap) -> Self {
        Self { bits, exact: true }
    }

    fn empty() -> Self {
        Self::exact(RoaringBitmap::new())
    }
}

/// Mirror column classification for a condition field.
enum FieldCol {
    Float,
    Str,
    Bool,
    /// No column and never seen as a top-level scalar: the JSON filter can
    /// only see a missing field or a non-scalar value there.
    Absent,
}

/// Translates a condition tree into a bitmap of candidate live rows.
///
/// Returns `None` when the condition (or a structurally required branch)
/// cannot be answered from columnar data.
pub(super) fn condition_bitmap(state: &MirrorState, cond: &Condition) -> Option<Eval> {
    match cond {
        Condition::Eq { field, value } => leaf_eq(state, field, value),
        Condition::Neq { field, value } => {
            let eq = leaf_eq(state, field, value)?;
            complement(state, &eq)
        }
        Condition::Gt { field, value } => leaf_ord(state, field, value, std::cmp::Ordering::is_gt),
        Condition::Gte { field, value } => leaf_ord(state, field, value, std::cmp::Ordering::is_ge),
        Condition::Lt { field, value } => leaf_ord(state, field, value, std::cmp::Ordering::is_lt),
        Condition::Lte { field, value } => leaf_ord(state, field, value, std::cmp::Ordering::is_le),
        Condition::In { field, values } => leaf_in(state, field, values),
        Condition::And { conditions } => and_bitmap(state, conditions),
        Condition::Or { conditions } => or_bitmap(state, conditions),
        Condition::Not { condition } => {
            let inner = condition_bitmap(state, condition)?;
            complement(state, &inner)
        }
        _ => None,
    }
}

/// Complements an exact eval over the live-row set; refuses inexact input.
fn complement(state: &MirrorState, eval: &Eval) -> Option<Eval> {
    eval.exact.then(|| Eval::exact(&state.live - &eval.bits))
}

/// AND: intersect translatable branches; untranslatable branches widen the
/// result to a superset (`exact = false`), which the post-filter narrows.
fn and_bitmap(state: &MirrorState, conditions: &[Condition]) -> Option<Eval> {
    if conditions.is_empty() {
        // `And { [] }` is the engine-handled identity: matches everything.
        return Some(Eval::exact(state.live.clone()));
    }
    let mut acc: Option<RoaringBitmap> = None;
    let mut exact = true;
    for cond in conditions {
        match condition_bitmap(state, cond) {
            Some(eval) => {
                exact &= eval.exact;
                acc = Some(match acc {
                    Some(bits) => bits & eval.bits,
                    None => eval.bits,
                });
            }
            None => exact = false,
        }
    }
    acc.map(|bits| Eval { bits, exact })
}

/// OR: every branch must translate, otherwise matches could be lost.
fn or_bitmap(state: &MirrorState, conditions: &[Condition]) -> Option<Eval> {
    let mut acc = RoaringBitmap::new();
    let mut exact = true;
    for cond in conditions {
        let eval = condition_bitmap(state, cond)?;
        exact &= eval.exact;
        acc |= eval.bits;
    }
    Some(Eval { bits: acc, exact })
}

/// Classifies a condition field against the mirror schema.
///
/// Returns `None` when the field must fall back to the JSON filter
/// (dotted path, capped-out field, or unexpected column kind).
fn classify_field(state: &MirrorState, field: &str) -> Option<FieldCol> {
    if field.contains('.') {
        return None; // nested paths are not mirrored
    }
    match state.store.get_column(field) {
        Some(TypedColumn::Float(_)) => Some(FieldCol::Float),
        Some(TypedColumn::String(_)) => Some(FieldCol::Str),
        Some(TypedColumn::Bool(_)) => Some(FieldCol::Bool),
        Some(_) => None,
        None if state.uncolumnized.contains(field) => None,
        None => Some(FieldCol::Absent),
    }
}

/// Equality leaf. Eligible only for type-matched scalar literals; replicates
/// `values_equal` exactly (epsilon equality for numbers, interned equality
/// for strings).
fn leaf_eq(state: &MirrorState, field: &str, value: &serde_json::Value) -> Option<Eval> {
    let col = classify_field(state, field)?;
    let bits = match (col, value) {
        (FieldCol::Absent, v) if is_scalar(v) => RoaringBitmap::new(),
        (FieldCol::Float, serde_json::Value::Number(n)) => {
            let lit = n.as_f64()?;
            state
                .store
                .filter_float_bitmap(field, move |v| (v - lit).abs() < f64::EPSILON)
        }
        (FieldCol::Str, serde_json::Value::String(s)) => {
            state.store.filter_eq_string_bitmap(field, s)
        }
        (FieldCol::Bool, serde_json::Value::Bool(b)) => {
            state.store.filter_bool_eq_bitmap(field, *b)
        }
        _ => return None,
    };
    Some(Eval::exact(bits))
}

/// Ordering leaf. JSON ordering (`compare_values`) only ever compares
/// Number/Number or String/String; all other combinations match nothing.
fn leaf_ord(
    state: &MirrorState,
    field: &str,
    value: &serde_json::Value,
    holds: impl Fn(std::cmp::Ordering) -> bool,
) -> Option<Eval> {
    let col = classify_field(state, field)?;
    match (col, value) {
        // Non-orderable literal types compare to nothing, on any column.
        (_, v) if !v.is_number() && !v.is_string() => Some(Eval::empty()),
        (FieldCol::Absent, _) => Some(Eval::empty()),
        (FieldCol::Float, serde_json::Value::Number(n)) => {
            let lit = n.as_f64()?;
            let bits = state
                .store
                .filter_float_bitmap(field, move |v| v.partial_cmp(&lit).is_some_and(&holds));
            Some(Eval::exact(bits))
        }
        // String ordering (lexicographic) and cross-type leaves fall back.
        _ => None,
    }
}

/// IN leaf. Eligible only when every list value matches the column type —
/// a single off-type value could match rows the mirror stored as null.
fn leaf_in(state: &MirrorState, field: &str, values: &[serde_json::Value]) -> Option<Eval> {
    let col = classify_field(state, field)?;
    if values.is_empty() {
        return Some(Eval::empty());
    }
    let bits = match col {
        // Missing fields never match IN, whatever the list contains.
        FieldCol::Absent => RoaringBitmap::new(),
        FieldCol::Float => {
            let lits: Vec<f64> = values
                .iter()
                .map(serde_json::Value::as_f64)
                .collect::<Option<Vec<f64>>>()?;
            state.store.filter_float_bitmap(field, move |v| {
                lits.iter().any(|lit| (v - lit).abs() < f64::EPSILON)
            })
        }
        FieldCol::Str => {
            let strs: Vec<&str> = values
                .iter()
                .map(serde_json::Value::as_str)
                .collect::<Option<Vec<&str>>>()?;
            state.store.filter_in_string_bitmap(field, &strs)
        }
        FieldCol::Bool => bool_in_bitmap(state, field, values)?,
    };
    Some(Eval::exact(bits))
}

/// IN over a bool column: union of the (at most two) distinct values.
fn bool_in_bitmap(
    state: &MirrorState,
    field: &str,
    values: &[serde_json::Value],
) -> Option<RoaringBitmap> {
    let bools: Vec<bool> = values
        .iter()
        .map(serde_json::Value::as_bool)
        .collect::<Option<Vec<bool>>>()?;
    let mut bits = RoaringBitmap::new();
    if bools.contains(&true) {
        bits |= state.store.filter_bool_eq_bitmap(field, true);
    }
    if bools.contains(&false) {
        bits |= state.store.filter_bool_eq_bitmap(field, false);
    }
    Some(bits)
}

/// Whether a JSON value is a mirrorable scalar (number, string, bool).
fn is_scalar(value: &serde_json::Value) -> bool {
    value.is_number() || value.is_string() || value.is_boolean()
}
