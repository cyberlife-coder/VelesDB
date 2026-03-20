//! Tests for the WAL group commit batcher.

#[cfg(test)]
mod tests {
    use crate::config::WalBatchConfig;
    use crate::storage::wal_batcher::WalBatcher;

    fn enabled_config(max_batch: usize) -> WalBatchConfig {
        WalBatchConfig {
            enabled: true,
            commit_delay_us: 100,
            max_batch_size: max_batch,
        }
    }

    // ========================================================================
    // Construction tests
    // ========================================================================

    #[test]
    fn new_batcher_starts_empty() {
        let batcher = WalBatcher::new(WalBatchConfig::default());
        assert_eq!(batcher.pending_count(), 0);
        assert_eq!(batcher.pending_bytes(), 0);
    }

    #[test]
    fn is_enabled_reflects_config() {
        let disabled = WalBatcher::new(WalBatchConfig::default());
        assert!(!disabled.is_enabled());

        let enabled = WalBatcher::new(enabled_config(128));
        assert!(enabled.is_enabled());
    }

    // ========================================================================
    // Disabled-mode tests (pass-through)
    // ========================================================================

    #[test]
    fn disabled_mode_writes_immediately() {
        let batcher = WalBatcher::new(WalBatchConfig::default());
        let mut buf = Vec::new();

        batcher.submit(b"hello", &mut buf).unwrap();

        assert_eq!(buf, b"hello");
        assert_eq!(batcher.pending_count(), 0);
    }

    #[test]
    fn disabled_mode_multiple_writes() {
        let batcher = WalBatcher::new(WalBatchConfig::default());
        let mut buf = Vec::new();

        batcher.submit(b"aaa", &mut buf).unwrap();
        batcher.submit(b"bbb", &mut buf).unwrap();

        assert_eq!(buf, b"aaabbb");
        assert_eq!(batcher.pending_count(), 0);
    }

    // ========================================================================
    // Enabled-mode buffering tests
    // ========================================================================

    #[test]
    fn enabled_mode_buffers_entries() {
        let batcher = WalBatcher::new(enabled_config(4));
        let mut buf = Vec::new();

        batcher.submit(b"entry1", &mut buf).unwrap();

        // Data should be buffered, not yet written to the writer
        assert!(buf.is_empty());
        assert_eq!(batcher.pending_count(), 1);
        assert_eq!(batcher.pending_bytes(), 6);
    }

    #[test]
    fn enabled_mode_flushes_at_max_batch_size() {
        let batcher = WalBatcher::new(enabled_config(3));
        let mut buf = Vec::new();

        batcher.submit(b"a", &mut buf).unwrap();
        batcher.submit(b"b", &mut buf).unwrap();
        // Third entry triggers flush
        batcher.submit(b"c", &mut buf).unwrap();

        assert_eq!(buf, b"abc");
        assert_eq!(batcher.pending_count(), 0);
        assert_eq!(batcher.pending_bytes(), 0);
    }

    #[test]
    fn enabled_mode_explicit_flush() {
        let batcher = WalBatcher::new(enabled_config(128));
        let mut buf = Vec::new();

        batcher.submit(b"first", &mut buf).unwrap();
        batcher.submit(b"second", &mut buf).unwrap();
        assert!(buf.is_empty());

        batcher.flush(&mut buf).unwrap();
        assert_eq!(buf, b"firstsecond");
        assert_eq!(batcher.pending_count(), 0);
    }

    #[test]
    fn flush_on_empty_buffer_is_noop() {
        let batcher = WalBatcher::new(enabled_config(128));
        let mut buf = Vec::new();

        // Flush with nothing pending should succeed silently
        batcher.flush(&mut buf).unwrap();
        assert!(buf.is_empty());
    }

    #[test]
    fn batch_resets_after_auto_flush() {
        let batcher = WalBatcher::new(enabled_config(2));
        let mut buf = Vec::new();

        // First batch
        batcher.submit(b"x", &mut buf).unwrap();
        batcher.submit(b"y", &mut buf).unwrap(); // triggers flush
        assert_eq!(buf, b"xy");

        // Second batch starts fresh
        batcher.submit(b"z", &mut buf).unwrap();
        assert_eq!(batcher.pending_count(), 1);
        assert_eq!(batcher.pending_bytes(), 1);

        batcher.flush(&mut buf).unwrap();
        assert_eq!(buf, b"xyz");
    }

    // ========================================================================
    // Edge cases
    // ========================================================================

    #[test]
    fn submit_empty_data() {
        let batcher = WalBatcher::new(enabled_config(128));
        let mut buf = Vec::new();

        batcher.submit(b"", &mut buf).unwrap();

        assert_eq!(batcher.pending_count(), 1);
        assert_eq!(batcher.pending_bytes(), 0);
    }

    #[test]
    fn max_batch_size_one_flushes_every_entry() {
        let batcher = WalBatcher::new(enabled_config(1));
        let mut buf = Vec::new();

        batcher.submit(b"alpha", &mut buf).unwrap();
        assert_eq!(buf, b"alpha");
        assert_eq!(batcher.pending_count(), 0);

        batcher.submit(b"beta", &mut buf).unwrap();
        assert_eq!(buf, b"alphabeta");
        assert_eq!(batcher.pending_count(), 0);
    }

    // ========================================================================
    // I/O error propagation
    // ========================================================================

    /// A writer that always returns an error.
    struct FailWriter;

    impl std::io::Write for FailWriter {
        fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("simulated I/O failure"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Err(std::io::Error::other("simulated flush failure"))
        }
    }

    #[test]
    fn disabled_mode_propagates_write_error() {
        let batcher = WalBatcher::new(WalBatchConfig::default());
        let mut writer = FailWriter;

        let result = batcher.submit(b"data", &mut writer);
        assert!(result.is_err());
    }

    #[test]
    fn enabled_mode_propagates_flush_error() {
        let batcher = WalBatcher::new(enabled_config(2));
        let mut writer = FailWriter;

        // First entry just buffers (no write yet)
        let mut ok_buf = Vec::new();
        batcher.submit(b"a", &mut ok_buf).unwrap();

        // Second entry triggers flush -> writer error
        let result = batcher.submit(b"b", &mut writer);
        assert!(result.is_err());
    }
}
