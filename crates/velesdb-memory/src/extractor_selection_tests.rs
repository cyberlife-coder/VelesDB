//! Tests for [`super::select_extractor`] — the seam that resolves an extraction
//! backend name to what the caller must do about it.
//!
//! Deliberately NOT gated on `feature = "extract"`, and that absence is the
//! point: `outline` and `none` must resolve in every build, including the
//! published one that has no HTTP backend compiled in. #1734 is exactly what
//! happens when the only code that can *choose* a dependency-free backend sits
//! behind an unrelated HTTP feature — two published tools went dead by default.
//! A test living inside `extract.rs`'s own `cfg(feature = "extract")` module
//! would have been compiled away alongside the defect.
//!
//! Its counterpart is `embedder_selection_tests.rs`; the two roles are kept
//! deliberately symmetric, down to the shape of the refusals.

use super::*;

#[test]
fn outline_is_ready_with_no_configuration_and_no_network() {
    let selection = select_extractor("outline").expect("`outline` is an accepted backend");
    assert!(
        matches!(selection, ExtractorSelection::Ready(_)),
        "the offline deterministic reader needs no URL and no model, so it comes \
         back ready to use — in every build"
    );
}

#[test]
fn ollama_defers_to_the_caller_for_url_and_model() {
    let selection = select_extractor("ollama").expect("`ollama` is an accepted backend");
    assert!(
        matches!(selection, ExtractorSelection::NeedsRemoteConfig("ollama")),
        "only the caller knows the URL and model, so the library names the \
         backend and stops there"
    );
}

#[test]
fn openai_defers_to_the_caller_for_url_and_model() {
    let selection = select_extractor("openai").expect("`openai` is an accepted backend");
    assert!(
        matches!(selection, ExtractorSelection::NeedsRemoteConfig("openai")),
        "the name must reach the caller intact: it is what the daemon dispatches \
         on, and a backend that came back as `ollama` would be silently served by \
         the wrong protocol"
    );
}

#[test]
fn none_and_the_empty_value_both_mean_no_extraction() {
    for backend in ["none", ""] {
        let selection =
            select_extractor(backend).unwrap_or_else(|err| panic!("`{backend}`: {err}"));
        assert!(
            matches!(selection, ExtractorSelection::Disabled),
            "`{backend}` must disable extraction rather than pick a backend"
        );
    }
    // Where this role legitimately diverges from the embedding one: "no
    // extraction" is a real choice — the graph simply does not build — while a
    // memory store cannot exist without an embedder, so `select_embedder`
    // refuses `""` instead.
}

#[test]
fn an_unknown_backend_names_the_accepted_forms() {
    // A vendor name on purpose. `openai` is now a real backend, and the point
    // of the whole protocol split is that reaching a new server is a different
    // URL rather than a new name here — so the refusal must steer a user who
    // typed their vendor towards the protocol they actually speak.
    let err = select_extractor("lmstudio").expect_err("`lmstudio` is not a backend name");
    assert!(
        err.contains("lmstudio"),
        "the refusal must quote what was asked for, got: {err}"
    );
    assert!(
        err.contains("outline") && err.contains("ollama") && err.contains("openai"),
        "the refusal must name the accepted forms, got: {err}"
    );
}
