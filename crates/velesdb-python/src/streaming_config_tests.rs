use super::*;

#[test]
fn test_streaming_config_defaults() {
    // The Python constructor defaults must match the core engine defaults.
    let cfg = StreamingIngestConfig::new(10_000, 128, 50);
    assert_eq!(cfg.buffer_size, 10_000);
    assert_eq!(cfg.batch_size, 128);
    assert_eq!(cfg.flush_interval_ms, 50);
}

#[test]
fn test_streaming_config_overrides() {
    let cfg = StreamingIngestConfig::new(4096, 256, 10);
    assert_eq!(cfg.buffer_size, 4096);
    assert_eq!(cfg.batch_size, 256);
    assert_eq!(cfg.flush_interval_ms, 10);
    assert!(cfg.__repr__().contains("buffer_size=4096"));
}
