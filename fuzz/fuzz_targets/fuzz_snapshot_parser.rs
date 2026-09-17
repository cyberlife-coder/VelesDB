//! Fuzz target for the payload snapshot parser.
//!
//! `LogPayloadStorage` parses `payloads.snapshot` when it opens, and the file
//! is whatever is on disk: truncated, corrupted, or crafted to claim a huge
//! `entry_count`. The parser must reject every malformed input with an error;
//! a panic, a sanitizer report or an allocation sized from an unchecked length
//! is a finding.
//!
//! The target calls velesdb-core's own parser, through the byte-slice entry the
//! crate compiles under `--cfg fuzzing`. It must never fuzz a copy: the copy it
//! used to carry never matched the real parser, crashed on its own arithmetic,
//! and left the real parser unfuzzed (#2298).

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Rejection is the expected outcome for almost every input.
    let _ = velesdb_core::storage::parse_payload_snapshot(data);
});
