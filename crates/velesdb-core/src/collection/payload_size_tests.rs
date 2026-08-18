use super::BoundedCounter;
use std::io::Write;

#[test]
fn counts_under_cap_without_tripping() {
    let mut counter = BoundedCounter::new(16);
    assert_eq!(counter.write(b"hello").expect("write"), 5);
    assert!(!counter.exceeded());
}

#[test]
fn trips_once_cap_exceeded() {
    let mut counter = BoundedCounter::new(4);
    assert!(counter.write(b"hello").is_err());
    assert!(counter.exceeded());
}

#[test]
fn measures_serialized_json_size() {
    let value = serde_json::json!({ "a": 1, "b": "xyz" });
    let exact = serde_json::to_vec(&value).expect("serialize").len();

    let mut under = BoundedCounter::new(exact);
    assert!(serde_json::to_writer(&mut under, &value).is_ok());
    assert!(!under.exceeded());

    let mut over = BoundedCounter::new(exact - 1);
    assert!(serde_json::to_writer(&mut over, &value).is_err());
    assert!(over.exceeded());
}

/// Writing exactly `cap` bytes is allowed; the cap trips strictly above it.
/// Exercises the boundary of the `written > cap` test, then `flush`.
#[test]
fn boundary_at_cap_and_flush_are_noops() {
    let mut counter = BoundedCounter::new(5);
    assert_eq!(counter.write(b"hello").expect("at cap"), 5);
    assert!(!counter.exceeded(), "exactly cap bytes does not trip");
    // One more byte tips it over.
    assert!(counter.write(b"!").is_err());
    assert!(counter.exceeded());
    // flush is a no-op that always succeeds.
    assert!(counter.flush().is_ok());
}
