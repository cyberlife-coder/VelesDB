//! Splits an NDJSON body, received in arbitrary chunks, into lines while
//! holding a bounded amount of it.
//!
//! The stream route reads its body incrementally and is exempt from the body
//! limit for that reason, which is only sound if an unfinished line is
//! bounded too: a body that never sends a newline is otherwise one line the
//! size of the body. [`LineSplitter`] keeps at most `max_line_bytes` of an
//! unfinished line; a longer one is dropped whole and reported, and the
//! stream carries on at the next newline.

/// Upper bound, in bytes, on one NDJSON line of the stream route.
///
/// The router sets it to `[server] max_body_size`: a line is one point, and a
/// point larger than the batch routes accept as a whole body is refused there
/// too.
#[derive(Clone, Copy, Debug)]
pub struct NdjsonLineLimit(pub usize);

/// Incremental line splitter with a per-line byte ceiling.
pub(super) struct LineSplitter {
    /// Bytes of the current, unfinished line; never longer than `max_line_bytes`.
    pending: Vec<u8>,
    max_line_bytes: usize,
    /// The current line already exceeded the ceiling: skip to its newline.
    discarding: bool,
}

impl LineSplitter {
    pub(super) fn new(max_line_bytes: usize) -> Self {
        Self {
            pending: Vec::new(),
            max_line_bytes,
            discarding: false,
        }
    }

    /// Feeds one body chunk, calling `on_line` with each line it completes
    /// (newline excluded). Returns how many lines were dropped for exceeding
    /// the ceiling; each is counted once, when it first exceeds it.
    ///
    /// Only `chunk` is scanned for newlines, so a line arriving in many
    /// chunks costs time linear in its length.
    pub(super) fn push(&mut self, mut chunk: &[u8], mut on_line: impl FnMut(&[u8])) -> usize {
        let mut dropped = 0;
        while let Some(newline) = chunk.iter().position(|byte| *byte == b'\n') {
            if self.take(&chunk[..newline]) {
                on_line(&self.pending);
            } else {
                dropped += usize::from(!self.discarding);
            }
            self.pending.clear();
            self.discarding = false;
            chunk = &chunk[newline + 1..];
        }
        if !self.discarding && !self.take(chunk) {
            dropped += 1;
            self.discarding = true;
            self.pending = Vec::new();
        }
        dropped
    }

    /// The bytes after the last newline: a final line sent without one.
    pub(super) fn finish(self) -> Vec<u8> {
        self.pending
    }

    /// Appends `bytes` to the current line; `false`, appending nothing, when
    /// the line is being discarded or would exceed the ceiling.
    fn take(&mut self, bytes: &[u8]) -> bool {
        let fits = !self.discarding
            && self.pending.len().saturating_add(bytes.len()) <= self.max_line_bytes;
        if fits {
            self.pending.extend_from_slice(bytes);
        }
        fits
    }
}

#[cfg(test)]
#[path = "ndjson_lines_tests.rs"]
mod tests;
