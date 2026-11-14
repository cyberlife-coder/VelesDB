use super::{
    compile_stdin_json, parse_compile_stdin_args, CompileStdinError, CompileStdinOptions,
    DEFAULT_COMPILE_STDIN_BUDGET,
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
        ..CompileStdinOptions::default()
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
        ..CompileStdinOptions::default()
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
        ..CompileStdinOptions::default()
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
            tool_output: false,
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
fn malformed_flags_are_rejected_with_the_reason() {
    use velesdb_memory::cli_flags::FlagError;

    let cases = [
        (
            vec!["--budget"],
            CompileStdinError::Flag(FlagError::MissingValue {
                flag: "--budget".to_owned(),
            }),
        ),
        (
            vec!["--query"],
            CompileStdinError::Flag(FlagError::MissingValue {
                flag: "--query".to_owned(),
            }),
        ),
        (
            vec!["--budget", "zero"],
            CompileStdinError::InvalidBudget {
                raw: "zero".to_owned(),
            },
        ),
        (vec!["--budget", "0"], CompileStdinError::ZeroBudget),
        (
            vec!["--nope"],
            CompileStdinError::Flag(FlagError::Unknown {
                command: "compile-stdin",
                flag: "--nope".to_owned(),
            }),
        ),
    ];
    for (bad, expected) in cases {
        let bad: Vec<String> = bad.into_iter().map(str::to_owned).collect();
        assert_eq!(
            parse_compile_stdin_args(&bad),
            Err(expected),
            "must reject {bad:?}"
        );
    }
}

#[test]
fn the_refusals_keep_their_wording() {
    assert_eq!(
        CompileStdinError::InvalidBudget {
            raw: "zero".to_owned()
        }
        .to_string(),
        r#"--budget expects a positive integer, got "zero""#
    );
    assert_eq!(
        CompileStdinError::ZeroBudget.to_string(),
        "--budget must be greater than 0"
    );
}

#[test]
fn tool_output_flag_takes_no_value() {
    let parsed = parse_compile_stdin_args(&[
        "--tool-output".to_owned(),
        "--budget".to_owned(),
        "512".to_owned(),
    ])
    .unwrap();
    assert!(parsed.tool_output);
    assert_eq!(parsed.token_budget, 512);
}

#[test]
fn tool_output_ships_value_dense_output_the_fragment_path_refuses() {
    // An `ls -l`-shaped listing: digits on every line. The fragment compiler
    // reports `high` at any cut (or fits nothing); the line pruner elides it
    // at `medium`.
    let listing = (0..600)
        .map(|i| {
            format!(
                "-rw-r--r-- 1 user staff {} Nov 13 12:{:02} file_{i}.rs",
                1000 + i,
                i % 60
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut options = parse_compile_stdin_args(&["--budget".to_owned(), "800".to_owned()]).unwrap();
    // Control: the fragment path cannot ship this — an error (nothing fits)
    // or a `high` verdict, both of which the hook turns into a passthrough.
    if let Ok(json) = compile_stdin_json(&listing, &options) {
        assert_eq!(parse(&json)["risk"], "high");
    }
    options.tool_output = true;
    let pruned = parse(&compile_stdin_json(&listing, &options).unwrap());
    assert_eq!(pruned["risk"], "medium");
    assert!(pruned["tokens_out"].as_u64().unwrap() <= 800);
    assert!(pruned["content"].as_str().unwrap().contains("lines elided"));
}
