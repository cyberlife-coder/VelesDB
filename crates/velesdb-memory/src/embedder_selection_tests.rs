//! Tests for [`super::select_embedder`] — the seam that resolves an embedding
//! backend name to what the caller must do about it.
//!
//! Deliberately NOT gated on `feature = "embedder-http"` (unlike
//! `embedder_tests.rs`,
//! which tests the Ollama client itself): the `hash` arm must resolve in every
//! build, including a feature-free one that has no HTTP backend compiled in. A
//! test that only ran under `--features embedder-http` would leave the shipped
//! binary's own path unexercised.

use super::*;

#[test]
fn an_unset_variable_takes_the_offline_default() {
    let selection = select_embedder(None).expect("an unset variable is not a caller error");
    match selection {
        EmbedderSelection::Ready(name, embedder) => {
            assert_eq!(name, "hash", "the default backend is the offline one");
            assert_eq!(
                embedder.dimension(),
                crate::DEFAULT_DIMENSION,
                "the default embedder is built at the crate's default dimension"
            );
        }
        other @ EmbedderSelection::NeedsRemoteConfig(_) => {
            panic!("an unset variable must resolve to a ready embedder, got {other:?}")
        }
    }
}

#[test]
fn hash_is_ready_with_no_configuration_and_no_network() {
    let selection = select_embedder(Some("hash")).expect("`hash` is an accepted backend");
    assert!(
        matches!(selection, EmbedderSelection::Ready("hash", _)),
        "`hash` needs no URL and no model, so it comes back ready to use"
    );
}

#[test]
fn ollama_defers_to_the_caller_for_url_and_model() {
    let selection = select_embedder(Some("ollama")).expect("`ollama` is an accepted backend");
    assert!(
        matches!(selection, EmbedderSelection::NeedsRemoteConfig("ollama")),
        "only the caller knows the URL and model, so the library names the \
         backend and stops there"
    );
}

#[test]
fn openai_defers_to_the_caller_for_url_and_model() {
    let selection = select_embedder(Some("openai")).expect("`openai` is an accepted backend");
    assert!(
        matches!(selection, EmbedderSelection::NeedsRemoteConfig("openai")),
        "the name must reach the caller intact: it is what the daemon dispatches \
         on, and a backend that came back as `ollama` would be silently served by \
         the wrong protocol"
    );
}

#[test]
fn an_empty_value_is_refused_like_any_other_unknown_name() {
    // Distinct from `None` ON PURPOSE, and preserved from the behaviour that
    // predates this seam: an *unset* variable means "no preference" and takes
    // the offline default, while `VELESDB_MEMORY_EMBEDDER=` is a caller who
    // set something and set it wrong. Collapsing the two (an `unwrap_or_default`
    // at the call site) would silently turn that mistake into a default.
    //
    // This is also where the embedder legitimately diverges from
    // [`crate::select_extractor`], which reads `""` as `none`: "no extraction"
    // is a real choice, "no embedder" is not.
    let err = select_embedder(Some("")).expect_err("an empty backend name is not a selection");
    assert!(
        err.contains("hash") && err.contains("ollama") && err.contains("openai"),
        "the refusal must name the accepted forms, got: {err}"
    );
}

#[test]
fn an_unknown_backend_names_the_accepted_forms() {
    // A vendor name on purpose. `openai` is now a real backend, and the point
    // of the whole protocol split is that reaching a new server is a different
    // URL rather than a new name here — so the refusal must steer a user who
    // typed their vendor towards the protocol they actually speak.
    let err = select_embedder(Some("lmstudio")).expect_err("`lmstudio` is not a backend name");
    assert!(
        err.contains("lmstudio"),
        "the refusal must quote what was asked for, got: {err}"
    );
    assert!(
        err.contains("hash") && err.contains("ollama") && err.contains("openai"),
        "the refusal must name the accepted forms, got: {err}"
    );
}
