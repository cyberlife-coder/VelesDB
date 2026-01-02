//! Fuzz target for VelesQL parser.
//!
//! This target tests the VelesQL parser with arbitrary input strings to find:
//! - Panics on malformed input
//! - Memory safety issues
//! - Infinite loops or excessive memory allocation
//!
//! # Running
//!
//! ```bash
//! cd fuzz
//! cargo +nightly fuzz run fuzz_velesql_parser
//! ```

#![no_main]

use libfuzzer_sys::fuzz_target;
use velesdb_core::velesql::Parser;

fuzz_target!(|data: &[u8]| {
    // Convert bytes to string (UTF-8 validation)
    if let Ok(input) = std::str::from_utf8(data) {
        // Parser should never panic on any input
        let _ = Parser::parse(input);
    }
});
