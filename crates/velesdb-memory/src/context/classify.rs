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

/// One classification rule. `applies` takes the policy too — only
/// `is_repetitive_log` reads it (`normalize_log_timestamps` changes what
/// counts as "repeated"), everything else ignores the parameter.
struct Rule {
    id: &'static str,
    action: ContextAction,
    critical: bool,
    reason: &'static str,
    applies: fn(&ContextFragment, &CompilePolicy) -> bool,
}

/// The ordered rule table — first match wins. `preserve.default` is the
/// unconditional last entry, so classification always yields a rule.
const RULES: &[Rule] = &[
    Rule {
        id: "media.atomic",
        action: ContextAction::Preserve,
        critical: true,
        reason: "media fragment packs as one atomic, unsplittable piece",
        applies: is_media,
    },
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
        applies: |_, _| true,
    },
];

/// Id of the terminal catch-all rule — exempt from `disabled_rules`, so
/// classification always terminates on a real table row (disabling it would
/// otherwise be a silent no-op knob).
const TERMINAL_RULE_ID: &str = "preserve.default";

/// Classify `fragment` under `policy`: the first enabled rule that matches.
pub(crate) fn classify(fragment: &ContextFragment, policy: &CompilePolicy) -> RuleMatch {
    RULES
        .iter()
        .filter(|rule| {
            rule.id == TERMINAL_RULE_ID || !policy.disabled_rules.iter().any(|d| d == rule.id)
        })
        .find(|rule| (rule.applies)(fragment, policy))
        .map_or_else(|| to_match(&RULES[RULES.len() - 1]), to_match)
}

/// Project a table row into its public match shape.
fn to_match(rule: &Rule) -> RuleMatch {
    RuleMatch {
        id: rule.id,
        action: rule.action,
        critical: rule.critical,
        reason: rule.reason,
    }
}

/// The fragment carries an inline media payload (US-009, PR1). Checked
/// first, unconditionally: a media fragment's classification must never
/// depend on its (often empty) caption text — a caption that happens to
/// contain a code fence, a URL, or a `verbatim`/`cache` metadata flag must
/// not steer a screenshot away from atomic packing. This also guarantees no
/// text-scanning rule below ever needs to consider `bytes_b64` — by the time
/// any of them run, a media fragment has already matched here.
fn is_media(fragment: &ContextFragment, _policy: &CompilePolicy) -> bool {
    fragment.media.is_some()
}

/// `metadata.verbatim == true`.
fn is_marked_verbatim(fragment: &ContextFragment, _policy: &CompilePolicy) -> bool {
    bool_meta(fragment, "verbatim")
}

/// `metadata.cache == true`.
fn is_marked_cache(fragment: &ContextFragment, _policy: &CompilePolicy) -> bool {
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
fn is_code(fragment: &ContextFragment, _policy: &CompilePolicy) -> bool {
    fragment.kind.as_deref() == Some("code") || fragment.content.contains("```")
}

/// Contains a negative-constraint marker (English or French). Lowercases
/// line by line so a megabyte fragment never allocates a second megabyte
/// (markers contain no newline, so no match can span two lines).
fn has_negative_constraint(fragment: &ContextFragment, _policy: &CompilePolicy) -> bool {
    const MARKERS: &[&str] = &[
        "never ",
        "must not",
        "do not",
        "don't",
        "ne pas",
        "ne jamais",
        "jamais ",
    ];
    fragment.content.lines().any(|line| {
        let lowered = word_bounded(&line.to_lowercase());
        MARKERS.iter().any(|marker| lowered.contains(marker))
    })
}

/// Normalize a lowercased line so a trailing-space marker (`"never "`,
/// `"jamais "`) still matches when the word is followed by punctuation
/// ("Never,") or ends the line outright, instead of only whitespace: ASCII
/// punctuation (apostrophe excluded, so `"don't"` stays intact) becomes a
/// space, and a trailing space is appended.
fn word_bounded(line: &str) -> String {
    let mut normalized: String = line
        .chars()
        .map(|c| {
            if c.is_ascii_punctuation() && c != '\'' {
                ' '
            } else {
                c
            }
        })
        .collect();
    normalized.push(' ');
    normalized
}

/// A `kind = "log"` fragment where at least one line repeats — under
/// [`CompilePolicy::normalize_log_timestamps`], "repeats" is judged on the
/// same masked grouping key [`collapse_repeated_lines`] will use, so a log
/// whose lines differ only by a timestamp is recognized as repetitive here
/// too (otherwise the rule would never fire for exactly the fragments the
/// option exists for).
fn is_repetitive_log(fragment: &ContextFragment, policy: &CompilePolicy) -> bool {
    if fragment.kind.as_deref() != Some("log") {
        return false;
    }
    let mut seen = std::collections::BTreeSet::new();
    fragment.content.lines().any(|line| {
        !line.trim().is_empty() && !seen.insert(dedup_key(line, policy.normalize_log_timestamps))
    })
}

/// At least three whitespace-separated tokens carry an ASCII digit — the
/// fragment is dense with exact values (ids, dates, quantities).
fn is_value_dense(fragment: &ContextFragment, _policy: &CompilePolicy) -> bool {
    fragment
        .content
        .split_whitespace()
        .filter(|token| token.bytes().any(|byte| byte.is_ascii_digit()))
        .count()
        >= 3
}

/// Contains an http(s) URL.
fn has_url(fragment: &ContextFragment, _policy: &CompilePolicy) -> bool {
    fragment.content.contains("http://") || fragment.content.contains("https://")
}

/// The `abstract.log_dedup` transformation: keep each distinct line's first
/// occurrence (in order) and annotate repeated ones with their total count —
/// a *structured* reduction, never a generative summary, so it is exactly
/// reproducible and reversible through the fragment's source handle.
///
/// When `normalize_timestamps` is set
/// ([`CompilePolicy::normalize_log_timestamps`]), lines are grouped by a
/// masked key (see [`super::log_normalize::mask_volatile_prefix`]) instead
/// of their raw text, so lines that differ only by a timestamp or a
/// bracketed hex/pid counter collapse together; the *emitted* line is still
/// the first occurrence's exact bytes — masking only changes grouping, never
/// what gets printed. Returns whether masking actually changed the grouping
/// (used to ventilate the decision `reason`); with the option off, or with
/// it on but nothing to mask, this is always `false` and the output is
/// byte-identical to the un-normalized path.
pub(crate) fn collapse_repeated_lines(content: &str, normalize_timestamps: bool) -> (String, bool) {
    let counts = line_groups(content, normalize_timestamps);
    let mut emitted: std::collections::BTreeSet<std::borrow::Cow<'_, str>> =
        std::collections::BTreeSet::new();
    let mut lines: Vec<String> = Vec::new();
    for line in content.lines() {
        let key = dedup_key(line, normalize_timestamps);
        // On first sight of a key, emit its line annotated with the group's
        // total count. Every key was just inserted into `counts`, so the
        // lookup is always `Some` — `&0` is unreachable, and
        // `annotated(_, 0)` would be wrong.
        if emitted.insert(key.clone()) {
            lines.push(annotated(line, counts[&key]));
        }
    }
    // Masking "modified" the fragment only when it actually merged lines
    // that would otherwise have stayed distinct — not merely when some line
    // happened to have a maskable prefix.
    let modified = normalize_timestamps && counts.len() < line_groups(content, false).len();
    (lines.join("\n"), modified)
}

/// This line's grouping key: the raw line when normalization is off, or its
/// masked form when normalization is on and a volatile prefix matched
/// ([`super::log_normalize::mask_volatile_prefix`]); unmasked lines fall
/// back to their raw text either way.
fn dedup_key(line: &str, normalize_timestamps: bool) -> std::borrow::Cow<'_, str> {
    if normalize_timestamps {
        if let Some(masked) = super::log_normalize::mask_volatile_prefix(line) {
            return std::borrow::Cow::Owned(masked);
        }
    }
    std::borrow::Cow::Borrowed(line)
}

/// Group `content`'s lines by [`dedup_key`], counting occurrences per group.
fn line_groups(
    content: &str,
    normalize_timestamps: bool,
) -> std::collections::BTreeMap<std::borrow::Cow<'_, str>, usize> {
    let mut counts = std::collections::BTreeMap::new();
    for line in content.lines() {
        *counts
            .entry(dedup_key(line, normalize_timestamps))
            .or_insert(0) += 1;
    }
    counts
}

/// A line plus its repetition annotation when it occurred more than once.
fn annotated(line: &str, count: usize) -> String {
    if count > 1 {
        format!("{line} (x{count})")
    } else {
        line.to_owned()
    }
}

/// Stable id of the screenshot-supersession verdict (US-009, PR2). Not a row
/// in the [`RULES`] table above — it needs cross-fragment visibility a
/// per-fragment `applies: fn(&ContextFragment, &CompilePolicy) -> bool`
/// cannot have (whether THIS fragment is the last of its series) — but it is
/// disclosed here, next to [`screenshot_supersession`], because in every
/// sense a caller cares about it IS a classification rule: a stable public
/// id that decides a fragment's fate before budget packing ever runs (see
/// `super::pack_items` and `super::superseded_screenshot_verdict`).
pub(crate) const SCREENSHOT_SUPERSEDED_RULE_ID: &str = "retrieve.screenshot_superseded";

/// The reason recorded for every screenshot [`screenshot_supersession`]
/// supersedes.
pub(crate) const SCREENSHOT_SUPERSEDED_REASON: &str =
    "superseded by a newer screenshot of the same target";

/// Reserved fragment metadata key naming a screenshot's subject — the value
/// two `kind: "screenshot"` fragments must share (JSON equality) to belong
/// to the same succession series.
const SCREENSHOT_TARGET_KEY: &str = "target";

/// This fragment's screenshot-succession identity: `Some(target)` only when
/// it carries media, is `kind == "screenshot"`, AND names a
/// `metadata.target` — a screenshot with no target is never part of a
/// series (no target is no evidence of succession), and a plain media
/// fragment (no `kind` at all, or a different `kind`) is never a candidate
/// either, however similar its metadata looks.
fn screenshot_target(fragment: &ContextFragment) -> Option<&Value> {
    if fragment.media.is_none() || fragment.kind.as_deref() != Some("screenshot") {
        return None;
    }
    fragment.metadata.as_ref()?.get(SCREENSHOT_TARGET_KEY)
}

/// For each fragment, in input order: whether a LATER fragment in the same
/// request shares its screenshot-succession identity ([`screenshot_target`])
/// — i.e. `true` for every screenshot of a target EXCEPT the last. Whole-
/// batch pass, symmetric to [`super::dedup::find_duplicates`]: both need
/// every fragment at once and both key on an identity narrower than full
/// content equality, but here the picture is reversed — an ordered
/// succession survives at its LAST member (the freshest screenshot), not
/// its first duplicate-detection anchor. Deterministic and clock-free: input
/// order is the only signal, exactly like every other rule in this module.
pub(crate) fn screenshot_supersession(fragments: &[ContextFragment]) -> Vec<bool> {
    let targets: Vec<Option<&Value>> = fragments.iter().map(screenshot_target).collect();
    (0..fragments.len())
        .map(|seq| match targets[seq] {
            Some(target) => targets[seq + 1..].contains(&Some(target)),
            None => false,
        })
        .collect()
}

#[cfg(test)]
#[path = "classify_tests.rs"]
mod tests;
