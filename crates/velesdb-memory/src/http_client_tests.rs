//! Tests for the transport layer's own behaviour.
//!
//! What goes on the wire is proved in `tests/openai_auth_bdd.rs`, against a
//! real socket. What is proved here is what can be checked without one: URL
//! assembly, and that a credential never leaks through `Debug`.

use super::*;

fn client(base_url: &str, auth: Auth) -> HttpJsonClient {
    HttpJsonClient::new(base_url, auth, ureq::AgentBuilder::new().build())
}

#[test]
fn a_non_standard_port_survives_into_the_url() {
    // oMLX serves on 8020/8028/8030. The base URL is kept verbatim precisely
    // so a port needs no special handling — it is already part of the string.
    let client = client("http://localhost:8020", Auth::None);
    assert_eq!(
        client.url_for("/v1/embeddings"),
        "http://localhost:8020/v1/embeddings"
    );
}

#[test]
fn a_trailing_slash_does_not_double_up() {
    let client = client("http://localhost:8020/", Auth::None);
    assert_eq!(
        client.url_for("/v1/embeddings"),
        "http://localhost:8020/v1/embeddings",
        "a base URL copied from a browser usually ends in a slash"
    );
}

#[test]
fn a_base_path_is_preserved() {
    // A provider behind a reverse proxy is reached at a prefix, not at the
    // origin. Concatenation rather than URL joining is what keeps this working.
    let client = client("https://gateway.example/inference", Auth::None);
    assert_eq!(
        client.url_for("/v1/embeddings"),
        "https://gateway.example/inference/v1/embeddings"
    );
}

#[test]
fn debug_redacts_a_bearer_token() {
    let printed = format!("{:?}", Auth::Bearer("sk-must-not-appear".to_owned()));
    assert!(
        !printed.contains("sk-must-not-appear"),
        "a token reaches logs through Debug far more often than through a \
         deliberate print, got: {printed}"
    );
}

#[test]
fn debug_redacts_a_custom_header_value_but_keeps_its_name() {
    let printed = format!(
        "{:?}",
        Auth::Header {
            name: "api-key".to_owned(),
            value: "azure-must-not-appear".to_owned(),
        }
    );
    assert!(
        !printed.contains("azure-must-not-appear"),
        "the value is the secret, got: {printed}"
    );
    assert!(
        printed.contains("api-key"),
        "the NAME is not a secret, and printing it is what makes a provider \
         configured with the wrong scheme diagnosable at all: {printed}"
    );
}
