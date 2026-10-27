use super::*;

/// `Auth` redacts its credential in `Debug`; the template arguments are
/// operator-supplied the same way and may carry one too (a chat template can
/// read any key it is given), so they are held to the same rule.
#[test]
fn debug_output_never_shows_the_chat_template_kwargs() {
    let extractor = OpenAiExtractor::new(
        "http://localhost:8019",
        "any-model",
        crate::http_client::Auth::None,
    )
    .with_chat_template_kwargs(serde_json::json!({ "api_key": "sk-live-hunter2" }));
    let shown = format!("{extractor:?}");
    assert!(!shown.contains("hunter2"), "leaked into: {shown}");
    assert!(
        shown.contains("chat_template_kwargs"),
        "the field stays visible, only its value is redacted: {shown}"
    );
}
