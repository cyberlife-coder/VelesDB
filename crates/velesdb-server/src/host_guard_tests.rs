//! Unit tests for `host_guard.rs`.

use super::{canonical_entry, is_loopback, parse_host};

#[test]
fn test_only_the_exact_probe_paths_are_probes() {
    for probe in ["/health", "/ready", "/v1/health", "/v1/ready"] {
        assert!(super::is_probe(probe), "{probe}");
    }
    for other in [
        "/healthz",
        "/health/x",
        "/v1/healthz",
        "/readyz",
        "/v2/health",
    ] {
        assert!(!super::is_probe(other), "{other}");
    }
}

#[test]
fn test_parse_host_drops_the_port_and_canonicalises() {
    assert_eq!(parse_host("LocalHost:8080").as_deref(), Some("localhost"));
    assert_eq!(parse_host("[::1]:8080").as_deref(), Some("[::1]"));
    assert_eq!(parse_host("[FE80:0::1]").as_deref(), Some("[fe80::1]"));
    assert_eq!(
        parse_host("db.example.com").as_deref(),
        Some("db.example.com")
    );
    for bad in [
        "[::1]junk",
        "localhost:evil",
        "localhost:",
        "::1",
        "a b",
        "evil.com.",
    ] {
        assert_eq!(parse_host(bad), None, "{bad}");
    }
}

#[test]
fn test_loopback_covers_the_whole_ipv4_block_and_ipv6() {
    for name in ["localhost", "127.0.0.1", "127.255.0.9", "[::1]"] {
        assert!(is_loopback(name), "{name}");
    }
    for name in ["128.0.0.1", "[::2]", "localhost.evil.com", "0.0.0.0"] {
        assert!(!is_loopback(name), "{name}");
    }
}

#[test]
fn test_configured_hosts_are_spelt_as_a_host_header_would() {
    assert_eq!(canonical_entry("::").as_deref(), Some("[::]"));
    assert_eq!(canonical_entry("FE80:0:0::1").as_deref(), Some("[fe80::1]"));
    assert_eq!(canonical_entry("[::1]").as_deref(), Some("[::1]"));
    assert_eq!(
        canonical_entry("My-Box.local").as_deref(),
        Some("my-box.local")
    );
    for bad in [
        "db:8080",
        "[::1]:80",
        "*.example.com",
        "example.com.",
        "https://x",
    ] {
        assert_eq!(canonical_entry(bad), None, "{bad}");
    }
}
