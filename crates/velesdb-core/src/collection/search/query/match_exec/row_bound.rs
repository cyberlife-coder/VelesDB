//! How many rows a MATCH walk may hold, and when it may stop early.
//!
//! A walk feeds rows to its RETURN `ORDER BY` (or to REST `/match`'s score
//! sort) and only then to `LIMIT`. Whatever reorders the rows needs every one
//! of them, so the walk runs to its end; stopping it at a fixed ceiling would
//! rank a truncated set and return the wrong top rows. What bounds that walk
//! is the cardinality guard-rail (EPIC-048 US-003): the walk refuses the
//! first row past it, with the count the WASM walk reports for the same
//! pattern. Only when nothing reorders the rows does the walk stop at
//! `LIMIT` — the rows the post-walk truncation would keep anyway.

use crate::error::{Error, Result};
use crate::guardrails::{GuardRailViolation, QueryContext};
use crate::metrics::{global_guardrails_metrics, LimitType};
use crate::velesql::MatchClause;

/// What decides which walk rows a MATCH keeps under its `LIMIT`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::collection::search::query) enum RowOrder {
    /// The clause alone: its RETURN `ORDER BY`, else walk order.
    Clause,
    /// A score sort the caller runs on the rows after the walk (REST
    /// `/match`'s query vector), ahead of any `ORDER BY` and `LIMIT`.
    Rescored,
}

/// The walk's row bound: refuse past `max_rows`, stop at `stop_at`.
#[derive(Debug, Clone, Copy)]
pub(super) struct RowBound {
    /// Rows the walk may hold: the cardinality guard-rail.
    max_rows: usize,
    /// The early stop, when nothing reorders the rows before `LIMIT`.
    stop_at: Option<usize>,
}

impl RowBound {
    /// The bound for `clause`: `max_rows` is the context's cardinality limit,
    /// else `default_max_rows` (the collection's configured limit, for the
    /// entry points that run without a context).
    pub(super) fn new(
        clause: &MatchClause,
        order: RowOrder,
        ctx: Option<&QueryContext>,
        default_max_rows: impl FnOnce() -> usize,
    ) -> Self {
        let max_rows = ctx.map_or_else(default_max_rows, |c| c.limits.max_cardinality);
        let reordered = order == RowOrder::Rescored || clause.return_clause.order_by.is_some();
        let stop_at = if reordered {
            None
        } else {
            super::super::match_dispatch::match_return_limit(clause)
        };
        Self { max_rows, stop_at }
    }

    /// Whether the walk holding `held` rows has every row it may return.
    pub(super) fn is_full(self, held: usize) -> bool {
        self.stop_at.is_some_and(|stop| held >= stop)
    }

    /// Admits one more row on top of `held`, or refuses it past the bound
    /// with the error `QueryContext::check_cardinality` raises, counted in
    /// `velesdb_limits_exceeded_total` as that check counts it.
    ///
    /// # Errors
    ///
    /// [`Error::GuardRail`] carrying [`GuardRailViolation::CardinalityExceeded`].
    pub(super) fn admit(self, held: usize) -> Result<()> {
        if held < self.max_rows {
            return Ok(());
        }
        global_guardrails_metrics().record_limit_exceeded(LimitType::Cardinality);
        Err(Error::from(GuardRailViolation::CardinalityExceeded {
            max: self.max_rows,
            actual: held.saturating_add(1),
        }))
    }
}
