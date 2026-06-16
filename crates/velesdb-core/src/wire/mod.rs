//! Binary wire formats shared across the server, CLI, and SDKs.
//!
//! These are pure byte (de)serialisers with no storage or persistence
//! dependency, so they compile on every target (including `wasm32`).

pub mod vrb1;
