//! Range bounds and candidate limits for secondary-index prefilters.

use super::SecondaryIndexMap;
use crate::filter::{Condition, POINT_ID_FIELD};
use crate::index::{secondary::IdSet, JsonValue, SecondaryIndex};
use roaring::RoaringBitmap;
use serde_json::Value;
use std::ops::Bound::{self, Excluded, Included, Unbounded};

/// Number of IDs a necessary equality or ID list can contribute. Only AND
/// branches restrict every matching row. Counting an index bucket does not
/// materialize its IDs.
pub(super) fn required_candidate_count(
    indexes: &SecondaryIndexMap,
    cond: &Condition,
) -> Option<u64> {
    match cond {
        Condition::And { conditions } => conditions
            .iter()
            .filter_map(|child| required_candidate_count(indexes, child))
            .min(),
        Condition::Eq { field, value } if field != POINT_ID_FIELD => {
            let key = JsonValue::from_json(value)?;
            let guard = indexes.read();
            let SecondaryIndex::BTree(tree) = guard.get(field)?;
            let count = tree.read().get(&key).map_or(0, IdSet::len);
            drop(guard);
            u64::try_from(count).ok()
        }
        _ => u64::try_from(cond.point_ids()?.len()).ok(),
    }
}

/// The f64 index cannot distinguish neighbouring integers at or past 2^53.
/// The logical point ID is outside the payload index altogether.
fn range_key(field: &str, value: &Value) -> Option<JsonValue> {
    if field == POINT_ID_FIELD || !crate::filter::exact_as_f64(value) {
        return None;
    }
    JsonValue::from_json(value)
}

/// The two inclusive bounds produced by lowering BETWEEN, in either order.
pub(super) fn inclusive_index_bounds(
    conditions: &[Condition],
) -> Option<(&str, JsonValue, JsonValue)> {
    use Condition::{Gte, Lte};
    let ([Gte { field, value: low }, Lte {
        field: other,
        value: high,
    }]
    | [Lte {
        field: other,
        value: high,
    }, Gte { field, value: low }]) = conditions
    else {
        return None;
    };
    if field != other {
        return None;
    }
    Some((field, range_key(field, low)?, range_key(field, high)?))
}

pub(super) fn one_sided_bitmap(
    indexes: &SecondaryIndexMap,
    field: &str,
    value: &Value,
    cond: &Condition,
    max_ids: Option<u64>,
) -> Option<RoaringBitmap> {
    let key = range_key(field, value)?;
    let (low, high) = match cond {
        Condition::Gt { .. } => (Excluded(&key), Unbounded),
        Condition::Gte { .. } => (Included(&key), Unbounded),
        Condition::Lt { .. } => (Unbounded, Excluded(&key)),
        Condition::Lte { .. } => (Unbounded, Included(&key)),
        _ => return None,
    };
    indexed_range(indexes, field, low, high, max_ids)
}

pub(super) fn indexed_range(
    indexes: &SecondaryIndexMap,
    field: &str,
    low: Bound<&JsonValue>,
    high: Bound<&JsonValue>,
    max_ids: Option<u64>,
) -> Option<RoaringBitmap> {
    let guard = indexes.read();
    let index = guard.get(field)?;
    if matches!((low, high), (Included(a), Included(b)) if a > b) {
        return Some(RoaringBitmap::new());
    }
    let bitmap = match max_ids {
        Some(limit) => index.range_bitmap_with_limit(low, high, Some(limit)),
        None => index.range_bitmap(low, high),
    };
    drop(guard);
    bitmap
}
