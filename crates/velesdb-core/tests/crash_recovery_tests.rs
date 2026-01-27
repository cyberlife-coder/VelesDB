//! Crash recovery integration tests for `VelesDB`.
//!
//! These tests verify that `VelesDB` survives abrupt shutdowns without logical corruption.

mod crash_recovery;

pub use crash_recovery::{CrashTestDriver, DriverConfig, IntegrityReport, IntegrityValidator};
