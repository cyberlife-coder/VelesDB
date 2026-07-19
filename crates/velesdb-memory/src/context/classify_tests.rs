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
        media: None,
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
    let (collapsed, normalized) = collapse_repeated_lines("b\na\nb\nb\nc", false);
    assert_eq!(collapsed, "b (x3)\na\nc");
    assert!(
        !normalized,
        "normalization is off, nothing should be flagged"
    );
}

#[test]
fn test_collapse_repeated_lines_without_repeats_is_identity() {
    let (collapsed, normalized) = collapse_repeated_lines("a\nb\nc", false);
    assert_eq!(collapsed, "a\nb\nc");
    assert!(!normalized);
}

#[test]
fn test_collapse_repeated_lines_timestamped_duplicates_do_not_collapse_by_default() {
    // RED (pre-fix) baseline: a real log where every line differs only by an
    // ISO timestamp does not collapse today — this is the documented
    // limitation the skill's "Timestamped logs" bullet calls out.
    let log = "2026-07-18T10:23:45.001Z INFO canary check passed for shard-1\n\
               2026-07-18T10:23:45.501Z INFO canary check passed for shard-1\n\
               2026-07-18T10:23:46.002Z INFO canary check passed for shard-1";
    let (collapsed, normalized) = collapse_repeated_lines(log, false);
    assert_eq!(
        collapsed, log,
        "without the option, timestamp-only variants must stay distinct (golden, unchanged)"
    );
    assert!(!normalized);
}

#[test]
fn test_collapse_repeated_lines_timestamped_duplicates_collapse_when_normalized() {
    // GREEN (post-fix): the same log, with normalize_log_timestamps on,
    // collapses to one annotated line — the volatile ISO prefix is masked
    // before grouping, so the three lines are recognized as the same log
    // event.
    let log = "2026-07-18T10:23:45.001Z INFO canary check passed for shard-1\n\
               2026-07-18T10:23:45.501Z INFO canary check passed for shard-1\n\
               2026-07-18T10:23:46.002Z INFO canary check passed for shard-1";
    let (collapsed, normalized) = collapse_repeated_lines(log, true);
    assert_eq!(
        collapsed,
        "2026-07-18T10:23:45.001Z INFO canary check passed for shard-1 (x3)"
    );
    assert!(
        normalized,
        "masking must be reported as having changed the grouping"
    );
}

#[test]
fn test_collapse_repeated_lines_normalize_on_but_no_timestamps_reports_unmodified() {
    // The option being on must not itself flag "modified" when nothing in
    // the content actually had a volatile prefix to mask.
    let (collapsed, normalized) = collapse_repeated_lines("a\nb\nc", true);
    assert_eq!(collapsed, "a\nb\nc");
    assert!(!normalized);
}

#[test]
fn test_classify_plain_prose_gets_default_rule() {
    let matched = classify_default(&fragment("Some plain prose."));
    assert_eq!(matched.id, "preserve.default");
    assert_eq!(matched.action, ContextAction::Preserve);
    assert!(!matched.critical);
}

fn media_fragment(caption: &str) -> ContextFragment {
    ContextFragment {
        media: Some(crate::context::model::MediaRef {
            mime: "image/png".to_owned(),
            bytes_b64: "iVBORw0KGgo=".to_owned(),
        }),
        ..fragment(caption)
    }
}

// --- screenshot_supersession (US-009, PR2) --------------------------------

fn screenshot(caption: &str, target: &str) -> ContextFragment {
    let mut meta = Map::new();
    meta.insert("target".to_owned(), Value::String(target.to_owned()));
    ContextFragment {
        kind: Some("screenshot".to_owned()),
        metadata: Some(meta),
        ..media_fragment(caption)
    }
}

#[test]
fn test_screenshot_supersession_three_same_target_supersedes_all_but_the_last() {
    let fragments = vec![
        screenshot("v1", "login-page"),
        screenshot("v2", "login-page"),
        screenshot("v3", "login-page"),
    ];
    assert_eq!(screenshot_supersession(&fragments), vec![true, true, false]);
}

#[test]
fn test_screenshot_supersession_different_targets_are_never_superseded() {
    let fragments = vec![
        screenshot("a", "login-page"),
        screenshot("b", "checkout-page"),
    ];
    assert_eq!(screenshot_supersession(&fragments), vec![false, false]);
}

#[test]
fn test_screenshot_supersession_without_target_is_never_superseded() {
    let no_target = media_fragment_kind("screenshot");
    let fragments = vec![no_target.clone(), no_target];
    assert_eq!(screenshot_supersession(&fragments), vec![false, false]);
}

#[test]
fn test_screenshot_supersession_ignores_media_fragments_without_screenshot_kind() {
    // Same media bytes, same metadata shape, but not `kind: "screenshot"` —
    // never a supersession candidate.
    let mut meta = Map::new();
    meta.insert("target".to_owned(), Value::String("login-page".to_owned()));
    let not_a_screenshot = ContextFragment {
        metadata: Some(meta),
        ..media_fragment("a")
    };
    let fragments = vec![not_a_screenshot.clone(), not_a_screenshot];
    assert_eq!(screenshot_supersession(&fragments), vec![false, false]);
}

#[test]
fn test_screenshot_supersession_ignores_non_media_fragments_even_with_matching_metadata() {
    let mut meta = Map::new();
    meta.insert("target".to_owned(), Value::String("login-page".to_owned()));
    let text_only = ContextFragment {
        kind: Some("screenshot".to_owned()),
        metadata: Some(meta),
        ..fragment("no media here")
    };
    let fragments = vec![text_only.clone(), text_only];
    assert_eq!(screenshot_supersession(&fragments), vec![false, false]);
}

#[test]
fn test_screenshot_supersession_a_single_screenshot_is_never_superseded() {
    let fragments = vec![screenshot("only one", "login-page")];
    assert_eq!(screenshot_supersession(&fragments), vec![false]);
}

fn media_fragment_kind(kind: &str) -> ContextFragment {
    ContextFragment {
        kind: Some(kind.to_owned()),
        ..media_fragment("no target")
    }
}

#[test]
fn test_classify_media_fragment_gets_its_own_atomic_rule() {
    let matched = classify_default(&media_fragment(""));
    assert_eq!(matched.id, "media.atomic");
    assert_eq!(matched.action, ContextAction::Preserve);
    assert!(matched.critical);
}

#[test]
fn test_classify_media_fragment_wins_over_every_text_rule() {
    // A caption that would otherwise match code/url/negative-constraint/
    // value-dense rules must still classify as media.atomic — the media
    // rule never lets a fragment's own text content steer its
    // classification away from atomic packing.
    let matched = classify_default(&media_fragment(
        "```rust\nfn f() {}\n``` never do this http://x 1 2 3",
    ));
    assert_eq!(matched.id, "media.atomic");
}

#[test]
fn test_classify_media_fragment_wins_even_over_marked_verbatim_and_cache() {
    let mut meta = Map::new();
    meta.insert("cache".to_owned(), Value::Bool(true));
    let frag = ContextFragment {
        metadata: Some(meta),
        ..media_fragment("caption")
    };
    assert_eq!(classify_default(&frag).id, "media.atomic");
}

#[test]
fn test_classify_media_atomic_rule_can_be_disabled() {
    let policy = CompilePolicy {
        disabled_rules: vec!["media.atomic".to_owned()],
        ..CompilePolicy::default()
    };
    // With the rule disabled, an otherwise-plain-prose caption falls through
    // to the next matching rule exactly like any other disabled rule.
    assert_eq!(
        classify(&media_fragment("plain caption"), &policy).id,
        "preserve.default"
    );
}

#[test]
fn test_classify_non_media_fragment_is_unaffected_by_the_media_rule() {
    assert_ne!(
        classify_default(&fragment("plain prose")).id,
        "media.atomic"
    );
}
