//! What a filter says about point ids, and about numbers an `f64` index
//! cannot tell apart.
//!
//! A `VelesQL` `id` column converts to [`POINT_ID_FIELD`]. `id = 5` and
//! `id IN (1, 2)` name their points exactly, so a scan can visit those ids
//! instead of every point, and a bitmap prefilter can hold exactly them.

use serde_json::Value;

use super::{Condition, POINT_ID_FIELD};

/// 2^53: an integer at or past it may share its `f64` with a neighbour.
const F64_EXACT_INTEGER_LIMIT: u64 = 1 << 53;

/// Whether a payload index or mirror keyed on `f64` answers for `value`
/// exactly: anything but an integer of magnitude 2^53 or more, which shares
/// its `f64` with a neighbour while the filter compares two integers
/// exactly. Such a literal must fall back to the filter itself.
pub(crate) fn exact_as_f64(value: &Value) -> bool {
    match value {
        Value::Number(n) => match (n.as_i64(), n.as_u64()) {
            (Some(i), _) => i.unsigned_abs() < F64_EXACT_INTEGER_LIMIT,
            (None, Some(u)) => u < F64_EXACT_INTEGER_LIMIT,
            (None, None) => true,
        },
        _ => true,
    }
}

impl Condition {
    /// Whether any leaf of this condition reads `field`.
    pub(crate) fn names_field(&self, field: &str) -> bool {
        match self {
            Self::And { conditions } | Self::Or { conditions } => {
                conditions.iter().any(|c| c.names_field(field))
            }
            Self::Not { condition } => condition.names_field(field),
            leaf => leaf.field() == Some(field),
        }
    }

    /// The points this condition can match, when it names them: `id = n`
    /// and `id IN (...)` with non-negative integers, sorted and deduplicated,
    /// or the smallest such set among the members of an `AND`. `None` when
    /// any point may match.
    pub(crate) fn point_ids(&self) -> Option<Vec<u64>> {
        let ids = match self {
            Self::Eq { field, value } if field == POINT_ID_FIELD => vec![value.as_u64()?],
            Self::In { field, values } if field == POINT_ID_FIELD => values
                .iter()
                .map(Value::as_u64)
                .collect::<Option<Vec<u64>>>()?,
            Self::And { conditions } => {
                return conditions
                    .iter()
                    .filter_map(Self::point_ids)
                    .min_by_key(Vec::len);
            }
            _ => return None,
        };
        let mut ids = ids;
        ids.sort_unstable();
        ids.dedup();
        Some(ids)
    }

    /// The field a leaf reads; `None` for `AND`, `OR` and `NOT`.
    fn field(&self) -> Option<&str> {
        match self {
            Self::Eq { field, .. }
            | Self::Neq { field, .. }
            | Self::Gt { field, .. }
            | Self::Gte { field, .. }
            | Self::Lt { field, .. }
            | Self::Lte { field, .. }
            | Self::In { field, .. }
            | Self::Contains { field, .. }
            | Self::IsNull { field }
            | Self::IsNotNull { field }
            | Self::Like { field, .. }
            | Self::ILike { field, .. }
            | Self::ArrayContains { field, .. }
            | Self::ArrayContainsAny { field, .. }
            | Self::ArrayContainsAll { field, .. }
            | Self::GeoDistance { field, .. }
            | Self::GeoBbox { field, .. } => Some(field),
            Self::And { .. } | Self::Or { .. } | Self::Not { .. } => None,
        }
    }
}
