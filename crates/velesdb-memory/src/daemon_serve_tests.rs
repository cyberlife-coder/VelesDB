use super::is_loopback_host;

#[test]
fn loopback_v4_and_v6_are_recognized() {
    assert!(is_loopback_host("127.0.0.1:18090"));
    assert!(is_loopback_host("127.0.0.5:18090"));
    assert!(is_loopback_host("[::1]:18090"));
}

#[test]
fn non_loopback_hosts_are_rejected() {
    assert!(!is_loopback_host("0.0.0.0:18090"));
    assert!(!is_loopback_host("192.168.1.10:18090"));
    assert!(!is_loopback_host("[::]:18090"));
    assert!(!is_loopback_host("mcp.example.com:18090"));
}
