//! Behaviour: the DEFAULT build carries both semantic backends, so choosing
//! `ollama` or `openai` for embedding/extraction is an env-var switch at
//! runtime — never a rebuild.
//!
//! The install paths that build with default features are exactly the ones
//! whose users cannot rebuild: `cargo install velesdb-memory` and the
//! `.mcpb` registry bundle (Claude Desktop's one-click path). Before this
//! contract, those binaries could not do semantic recall at all while the
//! README's pitch promised it — the least technical install path shipped
//! the least capable binary.
//!
//! This file is deliberately NOT feature-gated: `cargo test` builds it with
//! the crate's default features, so if `ollama` or `extract` ever leave the
//! default set again, the imports below fail to COMPILE. That compile error
//! is the test.

use velesdb_memory::{
    select_embedder, select_extractor, Auth, EmbedderSelection, ExtractorSelection, OllamaEmbedder,
    OllamaExtractor, OpenAiEmbedder, OpenAiExtractor,
};

/// Presence at the type level. The embedder constructors probe their endpoint
/// (a network call), so the contract for them is that the types are reachable
/// from a default build — reaching the call at all is the assertion.
fn exported<T>() {}

#[test]
fn semantic_backends_are_an_env_switch_not_a_rebuild() {
    exported::<OllamaEmbedder>();
    exported::<OpenAiEmbedder>();

    // The extractor constructors are probe-free: construct them for real.
    // Nothing is contacted — that only happens on `extract_graph`.
    let _ollama = OllamaExtractor::new("http://localhost:11434", "any-model");
    let _openai = OpenAiExtractor::new("http://localhost:8019", "any-model", Auth::None);

    // And the selectors route the names to the remote-config path rather
    // than to a "requires rebuilding with --features" refusal.
    assert!(
        matches!(
            select_embedder(Some("ollama")),
            Ok(EmbedderSelection::NeedsRemoteConfig("ollama"))
        ),
        "the default build must route 'ollama' to remote config"
    );
    assert!(
        matches!(
            select_extractor("ollama"),
            Ok(ExtractorSelection::NeedsRemoteConfig("ollama"))
        ),
        "the default build must route the 'ollama' extractor to remote config"
    );
}
