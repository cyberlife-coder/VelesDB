use super::*;
use crate::context::estimator::HeuristicEstimator;
use crate::context::model::FidelityRisk;

fn prune(text: &str, query: &str, budget: u64) -> PrunedToolOutput {
    prune_tool_output(
        text,
        query,
        budget,
        &ToolOutputPolicy::default(),
        &HeuristicEstimator,
    )
}

/// `count` value-dense filler lines (an `ls -l` / `git log --stat` shape).
fn listing(count: usize) -> String {
    (0..count)
        .map(|i| {
            format!(
                "-rw-r--r-- 1 user staff {} Nov 13 12:{:02} file_{i}.rs",
                1000 + i,
                i % 60
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn output_that_fits_is_returned_whole_at_low_risk() {
    let text = "line one\nline two\nline three";
    let pruned = prune(text, "", 1_000);
    assert_eq!(pruned.content, text);
    assert_eq!(pruned.elided_lines, 0);
    assert_eq!(pruned.risk, FidelityRisk::Low);
}

#[test]
fn value_dense_filler_is_elided_at_medium_risk_not_refused() {
    // The shape `preserve.exact_values` made critical as a whole: every line
    // carries digits. Line by line, none of them is salient.
    let text = listing(400);
    let pruned = prune(&text, "", 600);
    assert!(pruned.elided_lines > 0);
    assert_eq!(pruned.salient_dropped, 0);
    assert_eq!(pruned.risk, FidelityRisk::Medium);
    assert!(
        pruned.tokens_out <= 600,
        "tokens_out {} over budget",
        pruned.tokens_out
    );
    assert!(pruned.content.contains("lines elided"));
}

#[test]
fn an_error_line_buried_in_filler_survives_verbatim() {
    let mut lines: Vec<String> = listing(300).lines().map(str::to_owned).collect();
    lines.insert(
        150,
        "error[E0308]: mismatched types at src/lib.rs:42".to_owned(),
    );
    let pruned = prune(&lines.join("\n"), "", 500);
    assert!(pruned
        .content
        .contains("error[E0308]: mismatched types at src/lib.rs:42"));
    assert_eq!(pruned.risk, FidelityRisk::Medium);
}

#[test]
fn a_line_matching_the_query_is_kept() {
    let mut lines: Vec<String> = listing(300).lines().map(str::to_owned).collect();
    lines.insert(
        120,
        "-rw-r--r-- 1 user staff 77 Nov 13 09:00 needle_config.toml".to_owned(),
    );
    let pruned = prune(&lines.join("\n"), "ls -l needle_config", 500);
    assert!(pruned.content.contains("needle_config.toml"));
}

#[test]
fn negative_constraints_are_salient() {
    let mut lines: Vec<String> = listing(300).lines().map(str::to_owned).collect();
    lines.insert(200, "NOTE: do not run the migration twice".to_owned());
    let pruned = prune(&lines.join("\n"), "", 500);
    assert!(pruned.content.contains("do not run the migration twice"));
}

#[test]
fn head_and_tail_frame_the_elided_middle() {
    let text = listing(400);
    let pruned = prune(&text, "", 600);
    let first = text.lines().next().unwrap_or_default();
    let last = text.lines().last().unwrap_or_default();
    assert!(pruned.content.starts_with(first));
    assert!(pruned.content.ends_with(last));
}

#[test]
fn salient_lines_beyond_the_budget_raise_high_risk() {
    let text = (0..400)
        .map(|i| format!("error: test case {i} failed with exit code 101"))
        .collect::<Vec<_>>()
        .join("\n");
    let pruned = prune(&text, "", 300);
    assert!(pruned.salient_dropped > 0);
    assert_eq!(pruned.risk, FidelityRisk::High);
}

#[test]
fn consecutive_duplicates_collapse_with_their_count() {
    let text = format!("start\n{}end", "warning: unused import `foo`\n".repeat(50));
    let pruned = prune(&text, "", 1_000);
    assert!(pruned
        .content
        .contains("warning: unused import `foo` [×50]"));
    assert_eq!(pruned.content.matches("unused import").count(), 1);
}

#[test]
fn elision_markers_count_exactly_the_lines_they_replace() {
    let text = listing(400);
    let pruned = prune(&text, "", 600);
    let marked: usize = pruned
        .content
        .lines()
        .filter_map(|line| {
            line.strip_prefix("[… ")?
                .split(' ')
                .next()?
                .parse::<usize>()
                .ok()
        })
        .sum();
    assert_eq!(marked, pruned.elided_lines);
    let kept = pruned
        .content
        .lines()
        .filter(|line| !line.starts_with("[… "))
        .count();
    assert_eq!(kept + pruned.elided_lines, 400);
}

#[test]
fn pruning_is_deterministic() {
    let text = listing(500);
    assert_eq!(
        prune(&text, "file_3", 700).content,
        prune(&text, "file_3", 700).content
    );
}

#[test]
fn prose_about_errors_is_not_a_diagnostic() {
    // A changelog: "error", "fail", "never" are words of the prose there.
    let text = (0..400)
        .map(|i| format!("- Fixed an error where entry {i} could fail and never recover (#{i})."))
        .collect::<Vec<_>>()
        .join("\n");
    let pruned = prune(&text, "cat CHANGELOG.md", 600);
    assert_eq!(pruned.salient_dropped, 0);
    assert_eq!(pruned.risk, FidelityRisk::Medium);
}

#[test]
fn the_program_name_and_flags_are_not_query_terms() {
    assert_eq!(
        query_terms("git log --stat -40 src/lib.rs"),
        vec!["log", "src", "lib"]
    );
}

#[test]
fn uppercase_markers_are_whole_tokens() {
    assert!(is_diagnostic("test foo ... FAILED"));
    assert!(is_diagnostic("[ERROR] build broke"));
    assert!(!is_diagnostic("ERRORLESS mode enabled"));
    assert!(!is_diagnostic("the build failed to reproduce once"));
}

#[test]
fn output_estimated_within_budget_is_kept_whole_even_when_line_costs_overshoot() {
    // Per-line costs (+1 per newline) overshoot the whole-text estimate on
    // short lines: 120 one-letter lines must not be pruned at a budget the
    // text as a whole fits.
    let text = (0..120)
        .map(|i| if i % 2 == 0 { "a" } else { "b" })
        .collect::<Vec<_>>()
        .join("\n");
    let budget = HeuristicEstimator.estimate(&text);
    let pruned = prune(&text, "", budget);
    assert_eq!(pruned.elided_lines, 0);
    assert_eq!(pruned.content, text);
    assert_eq!(pruned.risk, FidelityRisk::Low);
}

#[test]
fn a_negative_constraint_outranks_head_and_tail() {
    let long = "x".repeat(400);
    let mut lines: Vec<String> = (0..40).map(|i| format!("{long} {i}")).collect();
    lines.insert(
        20,
        "warning text aside: do not run cargo clean before the bisect".to_owned(),
    );
    let pruned = prune(&lines.join("\n"), "", 400);
    assert!(
        pruned.content.contains("do not run cargo clean"),
        "{}",
        pruned.content
    );
}
