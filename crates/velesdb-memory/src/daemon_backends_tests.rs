//! Tests for the extraction role's `chat_template_kwargs` setting: what
//! `VELESDB_MEMORY_EXTRACTOR_CHAT_TEMPLATE_KWARGS` is allowed to hold, and
//! that a bad value stops the daemon at startup instead of being dropped.

use super::{build_openai_extractor, parse_chat_template_kwargs, CHAT_TEMPLATE_KWARGS_VAR};
use velesdb_memory::SettingError;

#[test]
fn an_unset_variable_means_no_kwargs() {
    assert_eq!(parse_chat_template_kwargs(None), Ok(None));
}

#[test]
fn an_empty_or_blank_value_means_no_kwargs() {
    assert_eq!(parse_chat_template_kwargs(Some("")), Ok(None));
    assert_eq!(parse_chat_template_kwargs(Some("  \t\n")), Ok(None));
}

#[test]
fn a_json_object_is_kept_as_written() {
    let parsed = parse_chat_template_kwargs(Some(r#"  {"enable_thinking": false, "n": [1]}  "#))
        .expect("an object is valid");
    assert_eq!(
        parsed,
        Some(serde_json::json!({ "enable_thinking": false, "n": [1] }))
    );
}

#[test]
fn an_empty_object_is_a_valid_value() {
    // `{}` is an explicit "send an empty chat_template_kwargs", distinct from
    // the variable being unset.
    assert_eq!(
        parse_chat_template_kwargs(Some("{}")),
        Ok(Some(serde_json::json!({})))
    );
}

#[test]
fn invalid_json_is_refused_and_names_the_variable() {
    let err = parse_chat_template_kwargs(Some(r#"{"enable_thinking": "#))
        .expect_err("truncated JSON must not be silently dropped");
    assert!(
        matches!(err, SettingError::InvalidChatTemplateKwargs { ref variable, .. } if variable == CHAT_TEMPLATE_KWARGS_VAR),
        "{err:?}"
    );
    let err = err.to_string();
    assert!(
        err.starts_with(&format!(
            "{CHAT_TEMPLATE_KWARGS_VAR} must be a JSON object: "
        )),
        "got: {err}"
    );
    assert!(
        err.contains("line 1 column"),
        "the parser's position is what makes the typo findable, got: {err}"
    );
}

#[test]
fn a_json_value_that_is_not_an_object_is_refused() {
    for raw in ["[1, 2]", "\"text\"", "42", "true", "null"] {
        let err = parse_chat_template_kwargs(Some(raw))
            .expect_err("only an object is a set of template arguments")
            .to_string();
        assert!(
            err.starts_with(&format!(
                "{CHAT_TEMPLATE_KWARGS_VAR} must be a JSON object: "
            )),
            "{raw} -> {err}"
        );
    }
}

#[test]
fn a_refusal_never_echoes_the_raw_value() {
    // The value is operator-supplied and may hold a secret the chat template
    // reads; a startup error ends up in a service manager's journal.
    for raw in [
        "sk-live-hunter2",
        "\"sk-live-hunter2\"",
        "{\"k\": sk-live-hunter2}",
    ] {
        let err = parse_chat_template_kwargs(Some(raw))
            .expect_err("not an object")
            .to_string();
        assert!(!err.contains("hunter2"), "{raw} leaked into: {err}");
    }
}

/// The tests below set process environment variables, which every test thread
/// shares: the same hazard and the same remedy as `remote_endpoint_tests.rs`.
static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Point the `OpenAI` extractor's role variables at a placeholder server and set
/// the kwargs variable to `kwargs` (`None` unsets it). Callers hold `ENV`.
fn configure_openai_extractor(kwargs: Option<&str>) {
    std::env::set_var("VELESDB_MEMORY_EXTRACTOR_URL", "http://127.0.0.1:1");
    std::env::set_var("VELESDB_MEMORY_EXTRACTOR_MODEL", "any-model");
    std::env::remove_var("VELESDB_MEMORY_EXTRACTOR_API_TOKEN");
    match kwargs {
        Some(value) => std::env::set_var(CHAT_TEMPLATE_KWARGS_VAR, value),
        None => std::env::remove_var(CHAT_TEMPLATE_KWARGS_VAR),
    }
}

fn clear_extractor_variables() {
    for key in [
        "VELESDB_MEMORY_EXTRACTOR_URL",
        "VELESDB_MEMORY_EXTRACTOR_MODEL",
        CHAT_TEMPLATE_KWARGS_VAR,
    ] {
        std::env::remove_var(key);
    }
}

#[test]
fn the_daemon_builder_fails_at_startup_on_a_malformed_variable() {
    let _env = ENV
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    configure_openai_extractor(Some("{not json"));
    let outcome = build_openai_extractor();
    clear_extractor_variables();

    let err = outcome
        .err()
        .expect("a malformed variable must stop startup, like DEFAULT_TTL does");
    assert!(
        err.to_string().contains(CHAT_TEMPLATE_KWARGS_VAR),
        "the operator must be told which variable to fix, got: {err}"
    );
}

#[test]
fn the_daemon_builder_fails_at_startup_on_a_non_object_variable() {
    let _env = ENV
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    configure_openai_extractor(Some("[true]"));
    let outcome = build_openai_extractor();
    clear_extractor_variables();

    assert!(
        outcome.is_err(),
        "an array is not a set of template arguments"
    );
}

#[test]
fn the_daemon_builder_accepts_a_valid_object_and_an_unset_variable() {
    let _env = ENV
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    configure_openai_extractor(Some(r#"{"enable_thinking": false}"#));
    let with_kwargs = build_openai_extractor();
    configure_openai_extractor(None);
    let without = build_openai_extractor();
    clear_extractor_variables();

    assert!(with_kwargs.is_ok());
    assert!(without.is_ok());
}
