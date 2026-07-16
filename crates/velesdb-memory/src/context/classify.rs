//! Deterministic fragment classification, driven by a rule *table*.
//!
//! Rules are data, not branching code: an ordered table of `(id, action,
//! matcher)` entries scanned top to bottom — the first enabled rule that
//! matches decides. Rule ids are **stable public contract** (they appear in
//! every [`super::model::ContextDecision`] and in the savings-by-rule
//! insights); add new rules, never rename existing ones.

use serde_json::Value;

use super::model::{CompilePolicy, ContextAction, ContextFragment};

/// The outcome of classifying one fragment.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RuleMatch {
    /// Stable id of the rule that matched.
    pub id: &'static str,
    /// The action the rule prescribes (before budget packing).
    pub action: ContextAction,
    /// Whether the content is critical: failing to pack it raises the
    /// compilation's fidelity risk to high.
    pub critical: bool,
    /// Human-readable reason recorded in the decision.
    pub reason: &'static str,
}

/// One classification rule.
struct Rule {
    id: &'static str,
    action: ContextAction,
    critical: bool,
    reason: &'static str,
    applies: fn(&ContextFragment) -> bool,
}

/// The ordered rule table — first match wins. `preserve.default` is the
/// unconditional last entry, so classification always yields a rule.
const RULES: &[Rule] = &[
    Rule {
        id: "preserve.marked_verbatim",
        action: ContextAction::Preserve,
        critical: true,
        reason: "caller marked this fragment verbatim",
        applies: is_marked_verbatim,
    },
    Rule {
        id: "cache.stable_prefix",
        action: ContextAction::Cache,
        critical: true,
        reason: "caller marked this fragment cacheable; it forms the stable prefix",
        applies: is_marked_cache,
    },
    Rule {
        id: "preserve.code_fence",
        action: ContextAction::Preserve,
        critical: true,
        reason: "code must survive verbatim",
        applies: is_code,
    },
    Rule {
        id: "preserve.negative_constraint",
        action: ContextAction::Preserve,
        critical: true,
        reason: "negative constraints must never be weakened",
        applies: has_negative_constraint,
    },
    Rule {
        id: "abstract.log_dedup",
        action: ContextAction::Abstract,
        critical: false,
        reason: "repeated log lines collapse into one annotated line",
        applies: is_repetitive_log,
    },
    Rule {
        id: "preserve.exact_values",
        action: ContextAction::Preserve,
        critical: true,
        reason: "numbers, dates and identifiers must survive verbatim",
        applies: is_value_dense,
    },
    Rule {
        id: "preserve.url",
        action: ContextAction::Preserve,
        critical: true,
        reason: "URLs must survive verbatim",
        applies: has_url,
    },
    Rule {
        id: "preserve.default",
        action: ContextAction::Preserve,
        critical: false,
        reason: "prose kept subject to budget",
        applies: |_| true,
    },
];

/// Classify `fragment` under `policy`: the first enabled rule that matches.
pub(crate) fn classify(fragment: &ContextFragment, policy: &CompilePolicy) -> RuleMatch {
    RULES
        .iter()
        .filter(|rule| !policy.disabled_rules.iter().any(|d| d == rule.id))
        .find(|rule| (rule.applies)(fragment))
        .map_or(DEFAULT_MATCH, |rule| RuleMatch {
            id: rule.id,
            action: rule.action,
            critical: rule.critical,
            reason: rule.reason,
        })
}

/// The fallback when even `preserve.default` was disabled by policy.
const DEFAULT_MATCH: RuleMatch = RuleMatch {
    id: "preserve.default",
    action: ContextAction::Preserve,
    critical: false,
    reason: "prose kept subject to budget",
};

/// `metadata.verbatim == true`.
fn is_marked_verbatim(fragment: &ContextFragment) -> bool {
    bool_meta(fragment, "verbatim")
}

/// `metadata.cache == true`.
fn is_marked_cache(fragment: &ContextFragment) -> bool {
    bool_meta(fragment, "cache")
}

/// Read a boolean metadata flag.
fn bool_meta(fragment: &ContextFragment, key: &str) -> bool {
    fragment
        .metadata
        .as_ref()
        .and_then(|meta| meta.get(key))
        .is_some_and(|value| matches!(value, Value::Bool(true)))
}

/// A triple-backtick-fenced block, or a caller-declared `kind = "code"`.
fn is_code(fragment: &ContextFragment) -> bool {
    fragment.kind.as_deref() == Some("code") || fragment.content.contains("```")
}

/// Contains a negative-constraint marker (English or French).
fn has_negative_constraint(fragment: &ContextFragment) -> bool {
    const MARKERS: &[&str] = &[
        "never ",
        "must not",
        "do not",
        "don't",
        "ne pas",
        "ne jamais",
        "jamais ",
    ];
    let lowered = fragment.content.to_lowercase();
    MARKERS.iter().any(|marker| lowered.contains(marker))
}

/// A `kind = "log"` fragment where at least one line repeats.
fn is_repetitive_log(fragment: &ContextFragment) -> bool {
    if fragment.kind.as_deref() != Some("log") {
        return false;
    }
    let mut seen = std::collections::BTreeSet::new();
    fragment
        .content
        .lines()
        .any(|line| !line.trim().is_empty() && !seen.insert(line))
}

/// At least three whitespace-separated tokens carry an ASCII digit — the
/// fragment is dense with exact values (ids, dates, quantities).
fn is_value_dense(fragment: &ContextFragment) -> bool {
    fragment
        .content
        .split_whitespace()
        .filter(|token| token.bytes().any(|byte| byte.is_ascii_digit()))
        .count()
        >= 3
}

/// Contains an http(s) URL.
fn has_url(fragment: &ContextFragment) -> bool {
    fragment.content.contains("http://") || fragment.content.contains("https://")
}

/// The `abstract.log_dedup` transformation: keep each distinct line's first
/// occurrence (in order) and annotate repeated ones with their total count —
/// a *structured* reduction, never a generative summary, so it is exactly
/// reproducible and reversible through the fragment's source handle.
pub(crate) fn collapse_repeated_lines(content: &str) -> String {
    let mut counts: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for line in content.lines() {
        *counts.entry(line).or_insert(0) += 1;
    }
    let mut emitted: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    let mut lines: Vec<String> = Vec::new();
    for line in content.lines() {
        if emitted.insert(line) {
            lines.push(annotated(line, counts.get(line).copied().unwrap_or(1)));
        }
    }
    lines.join("\n")
}

/// A line plus its repetition annotation when it occurred more than once.
fn annotated(line: &str, count: usize) -> String {
    if count > 1 {
        format!("{line} (x{count})")
    } else {
        line.to_owned()
    }
}

#[cfg(test)]
#[path = "classify_tests.rs"]
mod tests;
