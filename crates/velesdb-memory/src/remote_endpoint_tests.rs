//! Tests for [`role_auth`] and [`embedder_env_endpoint`] — the resolution the
//! daemon and the Python/Node bindings now share (#1886), where before only
//! the daemon read any of these variables at all.

use super::*;

/// Every variable [`embedder_env_endpoint`] reads, cleared so one test's
/// setup cannot leak into the next (tests run `--test-threads=1`, so the
/// risk is ordering, not races).
fn clear_embedder_vars() {
    for key in [
        "VELESDB_MEMORY_EMBEDDER_URL",
        "VELESDB_MEMORY_OLLAMA_URL",
        "VELESDB_MEMORY_EMBEDDER_MODEL",
        "VELESDB_MEMORY_OLLAMA_MODEL",
        "VELESDB_MEMORY_EMBEDDER_API_TOKEN",
    ] {
        std::env::remove_var(key);
    }
}

#[test]
fn role_auth_reads_no_credential_when_unset() {
    let key = "VELESDB_MEMORY_TEST_TOKEN_UNSET";
    std::env::remove_var(key);
    let auth = role_auth(key).expect("an unset token is not an error");
    assert!(matches!(auth, Auth::None));
}

#[test]
fn role_auth_refuses_a_blank_value() {
    let key = "VELESDB_MEMORY_TEST_TOKEN_BLANK";
    std::env::set_var(key, "   ");
    let err = role_auth(key).expect_err("a blank token must be refused, not sent as-is");
    assert!(
        err.contains(key),
        "the refusal must name the variable, got: {err}"
    );
    std::env::remove_var(key);
}

#[test]
fn role_auth_carries_a_set_token_as_bearer() {
    let key = "VELESDB_MEMORY_TEST_TOKEN_SET";
    std::env::set_var(key, "sk-secret");
    let auth = role_auth(key).expect("a real token is accepted");
    match auth {
        Auth::Bearer(token) => assert_eq!(token, "sk-secret"),
        other => panic!("expected a bearer token, got {other:?}"),
    }
    std::env::remove_var(key);
}

#[test]
fn embedder_env_endpoint_prefers_the_role_named_variables() {
    clear_embedder_vars();
    std::env::set_var("VELESDB_MEMORY_EMBEDDER_URL", "http://role");
    std::env::set_var("VELESDB_MEMORY_OLLAMA_URL", "http://legacy");
    std::env::set_var("VELESDB_MEMORY_EMBEDDER_MODEL", "role-model");

    let (endpoint, notice) = embedder_env_endpoint().expect("no token set is not an error");

    assert_eq!(endpoint.url.as_deref(), Some("http://role"));
    assert_eq!(endpoint.model.as_deref(), Some("role-model"));
    assert!(
        notice.is_some(),
        "the URL disagreeing between the two names must be reported"
    );
    clear_embedder_vars();
}

#[test]
fn embedder_env_endpoint_falls_back_to_the_legacy_ollama_alias() {
    clear_embedder_vars();
    std::env::set_var("VELESDB_MEMORY_OLLAMA_URL", "http://legacy-only");
    std::env::set_var("VELESDB_MEMORY_OLLAMA_MODEL", "legacy-model");

    let (endpoint, notice) = embedder_env_endpoint().expect("no token set is not an error");

    assert_eq!(endpoint.url.as_deref(), Some("http://legacy-only"));
    assert_eq!(endpoint.model.as_deref(), Some("legacy-model"));
    assert!(notice.is_none(), "using only the alias is not a conflict");
    clear_embedder_vars();
}

#[test]
fn embedder_env_endpoint_propagates_a_blank_token() {
    clear_embedder_vars();
    std::env::set_var("VELESDB_MEMORY_EMBEDDER_API_TOKEN", " ");

    let err = embedder_env_endpoint().expect_err("a blank token must fail resolution");
    assert!(
        err.contains("VELESDB_MEMORY_EMBEDDER_API_TOKEN"),
        "got: {err}"
    );
    clear_embedder_vars();
}

#[test]
fn require_reports_which_variable_is_missing() {
    let endpoint = RemoteEndpoint {
        url: None,
        model: Some("m".to_owned()),
        auth: Auth::None,
    };
    let err = endpoint
        .require("VELESDB_MEMORY_EMBEDDER")
        .expect_err("a missing url must fail");
    assert!(err.contains("VELESDB_MEMORY_EMBEDDER_URL"), "got: {err}");
}
