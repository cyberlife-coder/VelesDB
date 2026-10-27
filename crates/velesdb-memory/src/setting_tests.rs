//! The variant of each refused startup setting, which is what lets a binding
//! tell its caller which setting to correct without reading prose.

use crate::http_client::Auth;
use crate::remote_endpoint::RemoteEndpoint;
use crate::{select_embedder, select_extractor, SettingError};

#[test]
fn an_unknown_embedding_backend_is_named_in_the_error() {
    let error = select_embedder(Some("lmstudio"))
        .map(drop)
        .expect_err("refused");

    assert_eq!(
        error,
        SettingError::UnknownEmbedder {
            name: "lmstudio".to_owned()
        }
    );
}

#[test]
fn an_unknown_extraction_backend_is_named_in_the_error() {
    let error = select_extractor("lmstudio").map(drop).expect_err("refused");

    assert_eq!(
        error,
        SettingError::UnknownExtractor {
            name: "lmstudio".to_owned()
        }
    );
}

#[test]
fn an_openai_role_without_a_url_says_which_role_lacks_it() {
    let endpoint = RemoteEndpoint {
        url: None,
        model: Some("m".to_owned()),
        auth: Auth::None,
    };

    let error = endpoint
        .require("VELESDB_MEMORY_EXTRACTOR")
        .expect_err("no url");

    assert_eq!(
        error,
        SettingError::MissingUrl {
            role: "VELESDB_MEMORY_EXTRACTOR".to_owned()
        }
    );
    assert!(error.to_string().contains("VELESDB_MEMORY_EXTRACTOR_URL"));
}

#[test]
fn an_openai_role_without_a_model_says_which_role_lacks_it() {
    let endpoint = RemoteEndpoint {
        url: Some("http://localhost:8020".to_owned()),
        model: None,
        auth: Auth::None,
    };

    let error = endpoint
        .require("VELESDB_MEMORY_EMBEDDER")
        .expect_err("no model");

    assert_eq!(
        error,
        SettingError::MissingModel {
            role: "VELESDB_MEMORY_EMBEDDER".to_owned()
        }
    );
}
