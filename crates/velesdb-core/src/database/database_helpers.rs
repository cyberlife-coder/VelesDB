#[cfg(feature = "persistence")]
use super::{Database, Error, Result};

impl Database {
    /// The metadata filter of a DML WHERE that [`crate::velesql::check_dml_where`]
    /// accepted and whose parameters are bound (`velesql::bind_condition`):
    /// the filter conversion would turn a placeholder into NULL, matching no
    /// row.
    ///
    /// # Errors
    ///
    /// [`Error::Internal`] when the checked WHERE yields no filter.
    pub(super) fn build_dml_filter(
        statement: &str,
        bound: &crate::velesql::Condition,
    ) -> Result<crate::Filter> {
        let condition =
            crate::collection::Collection::extract_metadata_filter(bound).ok_or_else(|| {
                Error::Internal(format!("{statement} WHERE produced no metadata filter"))
            })?;
        Ok(crate::Filter::new(crate::Condition::from(condition)))
    }

    /// The joined collection's rows the pushed-down `filters` keep
    /// (`Collection::rows_where`, the rows a SELECT of that collection reads,
    /// a graph's payload-only nodes included). A text `MATCH` among them is
    /// that collection's BM25 hits (`Collection::bind_row_text_matches`), and
    /// each condition's field names are stripped of table prefixes (e.g.,
    /// `inventory.price` → `price`) before evaluation against the row
    /// payloads.
    pub(super) fn filtered_join_points(
        collection: &crate::collection::Collection,
        filters: &[crate::velesql::Condition],
    ) -> Vec<crate::Point> {
        let combined =
            collection.bind_row_text_matches(&Self::combine_filter_conditions(filters), None);
        let stripped = Self::strip_table_prefix_from_condition(combined);
        let filter = crate::Filter::new(crate::Condition::from(stripped));
        collection.rows_where(Some(&filter))
    }

    /// Combines multiple `velesql::Condition`s into a single AND tree.
    fn combine_filter_conditions(
        filters: &[crate::velesql::Condition],
    ) -> crate::velesql::Condition {
        let mut iter = filters.iter().cloned();
        let Some(first) = iter.next() else {
            unreachable!("caller must pass at least one filter");
        };
        iter.fold(first, |acc, c| {
            crate::velesql::Condition::And(Box::new(acc), Box::new(c))
        })
    }

    /// Strips table prefixes from all column references in a `velesql::Condition`.
    ///
    /// Converts qualified names like `inventory.price` to `price` so that
    /// `Filter::matches_point` can evaluate them against unqualified payload keys.
    pub(super) fn strip_table_prefix_from_condition(
        condition: crate::velesql::Condition,
    ) -> crate::velesql::Condition {
        use crate::velesql::Condition as C;
        match condition {
            C::And(l, r) => C::And(
                Box::new(Self::strip_table_prefix_from_condition(*l)),
                Box::new(Self::strip_table_prefix_from_condition(*r)),
            ),
            C::Or(l, r) => C::Or(
                Box::new(Self::strip_table_prefix_from_condition(*l)),
                Box::new(Self::strip_table_prefix_from_condition(*r)),
            ),
            C::Not(inner) => C::Not(Box::new(Self::strip_table_prefix_from_condition(*inner))),
            C::Group(inner) => C::Group(Box::new(Self::strip_table_prefix_from_condition(*inner))),
            leaf => Self::strip_table_prefix_on_leaf(leaf),
        }
    }

    /// Applies [`strip_prefix`] to the `column` field of leaf (non-composite)
    /// conditions. Engine-handled variants (`VectorSearch`, `GraphMatch`, …)
    /// pass through unchanged.
    ///
    /// [`strip_prefix`]: Self::strip_prefix
    fn strip_table_prefix_on_leaf(
        condition: crate::velesql::Condition,
    ) -> crate::velesql::Condition {
        use crate::velesql::Condition as C;
        match condition {
            C::Comparison(mut cmp) => {
                cmp.column = Self::strip_prefix(&cmp.column);
                C::Comparison(cmp)
            }
            C::In(mut inc) => {
                inc.column = Self::strip_prefix(&inc.column);
                C::In(inc)
            }
            C::Between(mut btw) => {
                btw.column = Self::strip_prefix(&btw.column);
                C::Between(btw)
            }
            C::Like(mut lk) => {
                lk.column = Self::strip_prefix(&lk.column);
                C::Like(lk)
            }
            C::IsNull(mut isn) => {
                isn.column = Self::strip_prefix(&isn.column);
                C::IsNull(isn)
            }
            C::Match(mut m) => {
                m.column = Self::strip_prefix(&m.column);
                C::Match(m)
            }
            C::Contains(mut cc) => {
                cc.column = Self::strip_prefix(&cc.column);
                C::Contains(cc)
            }
            C::ContainsText(mut ct) => {
                ct.column = Self::strip_prefix(&ct.column);
                C::ContainsText(ct)
            }
            C::GeoDistance(mut gd) => {
                gd.column = Self::strip_prefix(&gd.column);
                C::GeoDistance(gd)
            }
            C::GeoBbox(mut gb) => {
                gb.column = Self::strip_prefix(&gb.column);
                C::GeoBbox(gb)
            }
            // Engine-handled conditions pass through unchanged.
            other => other,
        }
    }

    /// Strips the `table.` prefix from a column name, if present.
    fn strip_prefix(column: &str) -> String {
        column
            .split_once('.')
            .map_or_else(|| column.to_string(), |(_, col)| col.to_string())
    }

    /// Every row of `collection` for JOIN-side materialization: the rows a
    /// SELECT of it reads (`Collection::rows_where`), lazily-expired TTL
    /// points excluded.
    pub(super) fn fetch_join_points(
        collection: &crate::collection::Collection,
    ) -> Vec<crate::Point> {
        collection.rows_where(None)
    }
}
