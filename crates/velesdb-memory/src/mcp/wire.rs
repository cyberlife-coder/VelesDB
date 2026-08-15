//! MCP-local path for the crate-wide defensive wire deserializer.

pub(super) use crate::wire::lenient;

#[cfg(test)]
#[path = "wire_tests.rs"]
mod tests;
