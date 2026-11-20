use super::*;

#[test]
fn test_build_rate_limit_config_succeeds() {
    let config = build_rate_limit_config(100);
    assert!(
        config.is_ok(),
        "governor config should build with burst=100"
    );
}

#[test]
fn test_build_rate_limit_config_burst_one() {
    let config = build_rate_limit_config(1);
    assert!(config.is_ok(), "governor config should build with burst=1");
}

#[test]
fn test_build_rate_limit_config_refuses_zero() {
    assert!(build_rate_limit_config(0).is_err());
}

#[test]
fn test_refill_interval_is_one_request_share_of_a_second() {
    assert_eq!(refill_interval(1), Some(Duration::from_secs(1)));
    assert_eq!(refill_interval(100), Some(Duration::from_millis(10)));
    assert_eq!(refill_interval(0), None);
    // Rounds to zero nanoseconds; the builder needs a non-zero interval.
    assert_eq!(refill_interval(u32::MAX), Some(Duration::from_nanos(1)));
    assert!(build_rate_limit_config(u32::MAX).is_ok());
}

/// An emptied bucket gives requests back at `burst` per second.
///
/// `per_second(burst)` set the interval to `burst` seconds per request: at the
/// default of 100, a client that had spent its 100 requests was allowed one
/// more every 100 seconds. The wait below is a lower bound only — a slow
/// machine waits longer and still finds the request given back — so load
/// cannot fail it.
#[test]
fn test_emptied_bucket_refills_at_the_configured_rate() {
    const BURST: u32 = 20; // one request back every 50 ms
    let config = build_rate_limit_config(BURST).expect("config should build");
    let limiter = config.limiter();
    let key = std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST);

    let admitted = (0..BURST * 2)
        .filter(|_| limiter.check_key(&key).is_ok())
        .count();
    assert!(
        admitted >= BURST as usize && admitted < (BURST * 2) as usize,
        "the bucket holds {BURST} and must run dry, admitted {admitted}"
    );

    std::thread::sleep(Duration::from_millis(200));
    assert!(
        limiter.check_key(&key).is_ok(),
        "a request must be given back within 4 refill intervals"
    );
}
