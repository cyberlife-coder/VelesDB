use super::{feed_joined_groups, Grouper};
use crate::error::{Error, GuardRailKind, TimeoutBudget};
use crate::guardrails::{QueryContext, QueryLimits};
use crate::point::{Point, SearchResult};
use crate::velesql::Parser;
use std::cell::Cell;

#[test]
fn joined_grouping_stops_between_batches_before_the_next_group_limit() {
    let query =
        Parser::parse("SELECT bucket, COUNT(*) FROM items GROUP BY bucket WITH (max_groups=255)")
            .expect("grouped query");
    let rows: Vec<_> = (0..512)
        .map(|id| {
            SearchResult::new(
                Point::new(id, vec![], Some(serde_json::json!({"bucket":id}))),
                0.0,
            )
        })
        .collect();
    let ordered: Vec<_> = rows.iter().collect();
    let grouper = || Grouper::new(&query.select, None, true).expect("grouper");
    let mut control = grouper();
    assert!(matches!(
        feed_joined_groups(&ordered, &mut control, &|| Ok(())),
        Err(Error::Config(_))
    ));

    let expired = QueryContext::new(QueryLimits::default().with_timeout_ms(10_000))
        .with_clock_skew_ms(20_000);
    let first_batch = Cell::new(true);
    let mut timed = grouper();
    let result = feed_joined_groups(&ordered, &mut timed, &|| {
        if first_batch.replace(false) {
            Ok(())
        } else {
            expired.check_timeout().map_err(Error::from)
        }
    });
    assert!(matches!(
        result,
        Err(Error::GuardRail {
            kind: GuardRailKind::Timeout(TimeoutBudget::Configured),
            ..
        })
    ));
    assert!((1..255).contains(&timed.finish().len()));
}
