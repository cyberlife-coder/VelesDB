//! Unit tests for the classification rule table.

use serde_json::{Map, Value};

use super::*;
use crate::context::model::{CompilePolicy, ContextAction, ContextFragment};

fn fragment(content: &str) -> ContextFragment {
    ContextFragment {
        id: None,
        content: content.to_owned(),
        kind: None,
        priority: None,
        metadata: None,
    }
}

fn classify_default(fragment: &ContextFragment) -> RuleMatch {
    classify(fragment, &CompilePolicy::default())
}

#[test]
fn test_classify_marked_verbatim_wins_over_everything() {
    let mut meta = Map::new();
    meta.insert("verbatim".to_owned(), Value::Bool(true));
    let frag = ContextFragment {
        metadata: Some(meta),
        ..fragment("```code``` never do this http://x 1 2 3")
    };
    let matched = classify_default(&frag);
    assert_eq!(matched.id, "preserve.marked_verbatim");
    assert!(matched.critical);
}

#[test]
fn test_classify_cache_flag_yields_cache_action() {
    let mut meta = Map::new();
    meta.insert("cache".to_owned(), Value::Bool(true));
    let frag = ContextFragment {
        metadata: Some(meta),
        ..fragment("You are the deploy assistant.")
    };
    let matched = classify_default(&frag);
    assert_eq!(matched.id, "cache.stable_prefix");
    assert_eq!(matched.action, ContextAction::Cache);
}

#[test]
fn test_classify_code_fence_and_code_kind() {
    assert_eq!(
        classify_default(&fragment("```rust\nfn f() {}\n```")).id,
        "preserve.code_fence"
    );
    let typed = ContextFragment {
        kind: Some("code".to_owned()),
        ..fragment("fn f() {}")
    };
    assert_eq!(classify_default(&typed).id, "preserve.code_fence");
}

#[test]
fn test_classify_negative_constraint_english_and_french() {
    assert_eq!(
        classify_default(&fragment("Never restart the primary.")).id,
        "preserve.negative_constraint"
    );
    assert_eq!(
        classify_default(&fragment("Ne pas relancer le primaire.")).id,
        "preserve.negative_constraint"
    );
}

#[test]
fn test_classify_negative_constraint_detected_even_before_punctuation() {
    // "never "/"jamais " require a trailing space in the marker table, so a
    // marker word immediately followed by punctuation instead of whitespace
    // ("Never," "jamais.") — or ending the line outright — must still be
    // recognized; a purely-`contains("never ")`-style check misses both.
    assert_eq!(
        classify_default(&fragment("Never, under any circumstances, delete vol-042.")).id,
        "preserve.negative_constraint",
        "a marker followed by a comma must still be detected"
    );
    assert_eq!(
        classify_default(&fragment(
            "Ne jamais: relancer le primaire sans validation."
        ))
        .id,
        "preserve.negative_constraint",
        "a marker followed by a colon must still be detected"
    );
    assert_eq!(
        classify_default(&fragment("The primary must never")).id,
        "preserve.negative_constraint",
        "a marker ending the line, with no trailing character at all, must still be detected"
    );
}

#[test]
fn test_classify_repetitive_log_abstracts() {
    let log = ContextFragment {
        kind: Some("log".to_owned()),
        ..fragment("ERROR x\nERROR x\nINFO done")
    };
    let matched = classify_default(&log);
    assert_eq!(matched.id, "abstract.log_dedup");
    assert_eq!(matched.action, ContextAction::Abstract);
    assert!(!matched.critical);
}

#[test]
fn test_classify_log_without_repeats_falls_through() {
    let log = ContextFragment {
        kind: Some("log".to_owned()),
        ..fragment("ERROR x\nINFO done")
    };
    assert_eq!(classify_default(&log).id, "preserve.default");
}

#[test]
fn test_classify_value_dense_needs_three_numeric_tokens() {
    assert_eq!(
        classify_default(&fragment("Order 8f3a-11 shipped 2026-07-14 for 42.50 EUR")).id,
        "preserve.exact_values"
    );
    assert_eq!(
        classify_default(&fragment("shard-3 recovered")).id,
        "preserve.default"
    );
}

#[test]
fn test_classify_url_is_preserved() {
    let matched = classify_default(&fragment("see https://wiki.example.com/x"));
    assert_eq!(matched.id, "preserve.url");
    assert_eq!(matched.action, ContextAction::Preserve);
}

#[test]
fn test_classify_disabled_rule_falls_through_to_next_match() {
    let policy = CompilePolicy {
        disabled_rules: vec!["abstract.log_dedup".to_owned()],
        ..CompilePolicy::default()
    };
    let log = ContextFragment {
        kind: Some("log".to_owned()),
        ..fragment("ERROR x\nERROR x\nINFO done")
    };
    assert_eq!(classify(&log, &policy).id, "preserve.default");
}

#[test]
fn test_classify_terminal_default_rule_cannot_be_disabled() {
    let policy = CompilePolicy {
        disabled_rules: vec!["preserve.default".to_owned()],
        ..CompilePolicy::default()
    };
    let matched = classify(&fragment("plain prose"), &policy);
    assert_eq!(
        matched.id, "preserve.default",
        "the terminal rule must be exempt from disabling"
    );
}

#[test]
fn test_collapse_repeated_lines_annotates_counts_in_first_seen_order() {
    let collapsed = collapse_repeated_lines("b\na\nb\nb\nc");
    assert_eq!(collapsed, "b (x3)\na\nc");
}

#[test]
fn test_collapse_repeated_lines_without_repeats_is_identity() {
    assert_eq!(collapse_repeated_lines("a\nb\nc"), "a\nb\nc");
}

#[test]
fn test_classify_plain_prose_gets_default_rule() {
    let matched = classify_default(&fragment("Some plain prose."));
    assert_eq!(matched.id, "preserve.default");
    assert_eq!(matched.action, ContextAction::Preserve);
    assert!(!matched.critical);
}
