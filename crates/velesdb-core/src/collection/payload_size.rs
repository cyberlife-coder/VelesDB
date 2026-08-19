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
#[path = "payload_size_tests.rs"]
mod tests;
