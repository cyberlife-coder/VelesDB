use super::{
    compile_stdin_json, parse_compile_stdin_args, CompileStdinOptions, DEFAULT_COMPILE_STDIN_BUDGET,
};

/// A tool-output-shaped corpus: repetitive log lines, the exact case a
/// `PostToolUse` hook has to shrink.
fn noisy_tool_output() -> String {
    use std::fmt::Write as _;

    let mut text = String::new();
    for i in 0..120 {
        let _ = writeln!(
            text,
            "[2026-07-25T01:0{}:00Z] INFO  worker: processing batch {} of 120 — retry=0 status=ok",
            i % 10,
            i
        );
    }
    text
}

fn parse(value: &str) -> serde_json::Value {
    serde_json::from_str(value).expect("compile-stdin must emit valid JSON")
}

#[test]
fn tight_budget_actually_shrinks_the_payload() {
    let options = CompileStdinOptions {
        token_budget: 1_500,
        query: "what did the worker do".to_owned(),
    };
    let compiled = parse(&compile_stdin_json(&noisy_tool_output(), &options).unwrap());

    let tokens_in = compiled["tokens_in"].as_u64().unwrap();
    let tokens_out = compiled["tokens_out"].as_u64().unwrap();
    assert!(tokens_in > 0, "tokens_in must be measured, got {tokens_in}");
    assert!(
        tokens_out < tokens_in,
        "a 200-token budget over {tokens_in} tokens of logs must compress: got {tokens_out}"
    );
    assert_eq!(
        compiled["tokens_saved"].as_u64().unwrap(),
        tokens_in - tokens_out
    );
    let content = compiled["content"].as_str().unwrap();
    assert!(
        !content.is_empty(),
        "an empty compilation is worse than no compilation — the caller would replace a \
         real tool result with nothing"
    );
    assert!(
        content.len() < noisy_tool_output().len(),
        "the compiled content must be shorter than the raw tool output"
    );
}

/// A budget too small to fit even one fragment makes the compiler
/// externalize everything and emit an EMPTY context. Returning that as a
/// success is a trap: `compile-stdin`'s caller (a `PostToolUse` hook) would
/// swap a real tool result for an empty string. Fail loudly instead, so
/// the caller falls back to the untouched output.
#[test]
fn budget_too_small_for_any_fragment_is_an_error() {
    let options = CompileStdinOptions {
        token_budget: 50,
        query: String::new(),
    };
    let error = compile_stdin_json(&noisy_tool_output(), &options).unwrap_err();
    let message = error.to_string();
    assert!(
        message.contains("budget"),
        "the error must point at the budget, got {message}"
    );
}

#[test]
fn compilation_is_byte_identical_across_runs() {
    let options = CompileStdinOptions {
        token_budget: 1_500,
        query: "worker batches".to_owned(),
    };
    let first = compile_stdin_json(&noisy_tool_output(), &options).unwrap();
    let second = compile_stdin_json(&noisy_tool_output(), &options).unwrap();
    assert_eq!(first, second, "the compiler must be deterministic");
}

#[test]
fn empty_stdin_is_rejected() {
    let error = compile_stdin_json("   \n\t ", &CompileStdinOptions::default()).unwrap_err();
    assert!(
        error.to_string().contains("empty"),
        "the error must name the cause, got {error}"
    );
}

#[test]
fn flags_default_and_override() {
    assert_eq!(
        parse_compile_stdin_args(&[]).unwrap(),
        CompileStdinOptions {
            token_budget: DEFAULT_COMPILE_STDIN_BUDGET,
            query: String::new(),
        }
    );
    let parsed = parse_compile_stdin_args(&[
        "--budget".to_owned(),
        "512".to_owned(),
        "--query".to_owned(),
        "why did it fail".to_owned(),
    ])
    .unwrap();
    assert_eq!(parsed.token_budget, 512);
    assert_eq!(parsed.query, "why did it fail");
}

#[test]
fn malformed_flags_are_rejected() {
    for bad in [
        vec!["--budget".to_owned()],
        vec!["--budget".to_owned(), "zero".to_owned()],
        vec!["--budget".to_owned(), "0".to_owned()],
        vec!["--nope".to_owned()],
    ] {
        assert!(
            parse_compile_stdin_args(&bad).is_err(),
            "must reject {bad:?}"
        );
    }
}
