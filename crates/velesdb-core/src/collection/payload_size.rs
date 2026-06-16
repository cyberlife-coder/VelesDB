//! Bounded serialized-payload size measurement (parity item E).
//!
//! [`BoundedCounter`] is an [`std::io::Write`] sink that counts bytes without
//! storing them and short-circuits once the running total passes a cap. Feeding
//! it to `serde_json::to_writer` measures a payload's serialized size at the
//! cost of at most `cap + 1` bytes of serialization — no throwaway `Vec`
//! allocation, and no work proportional to a payload far larger than the cap.

use std::io::{self, Write};

/// A `Write` sink that counts bytes and aborts once the cap is exceeded.
pub(crate) struct BoundedCounter {
    written: usize,
    cap: usize,
    exceeded: bool,
}

impl BoundedCounter {
    /// Creates a counter that trips once more than `cap` bytes are written.
    pub(crate) fn new(cap: usize) -> Self {
        Self {
            written: 0,
            cap,
            exceeded: false,
        }
    }

    /// Returns whether the serialized output passed the cap.
    pub(crate) fn exceeded(&self) -> bool {
        self.exceeded
    }
}

impl Write for BoundedCounter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.written = self.written.saturating_add(buf.len());
        if self.written > self.cap {
            self.exceeded = true;
            // Stop serialization early: returning an error makes
            // `serde_json::to_writer` abort once the cap is provably blown,
            // so the cost is bounded to ~`cap + 1` bytes regardless of how
            // large the payload actually is.
            return Err(io::Error::from(io::ErrorKind::WriteZero));
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
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
}
