use super::{Duration, Instant, QueryContext, QueryLimits};

#[test]
fn a_join_fetch_trains_on_its_own_time_and_keeps_the_statement_deadline() {
    let mut ctx = QueryContext::new(QueryLimits::new().with_timeout_ms(1));
    let statement = Instant::now().checked_sub(Duration::from_secs(1)).unwrap();
    ctx.start_time = statement;
    assert_eq!(ctx.select_started_at(), statement);
    let input = ctx.for_join_input();
    let before_fetch = Instant::now();
    assert!(input.select_started_at() >= before_fetch);
    assert!(input.check_timeout().is_err());
}
