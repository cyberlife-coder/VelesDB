//! Fixed-regex-equivalent masking of volatile log-line prefixes, used by
//! [`super::classify::collapse_repeated_lines`] when
//! [`super::model::CompilePolicy::normalize_log_timestamps`] is on.
//!
//! Two patterns are recognized, each optional and applied in order — never a
//! caller-supplied pattern, so masking stays deterministic and the same
//! request always yields the same collapse:
//!
//! 1. a leading timestamp: ISO-8601 (`2026-07-18T10:23:45(.123)?(Z|+02:00)?`
//!    or the space/comma log4j variant `2026-07-18 10:23:45,123`), or
//!    syslog (`Jul 18 10:23:45`);
//! 2. one or more immediately-following bracketed hex/decimal counters
//!    (`[a1b2c3]`, `[1234]`) — a bracket whose content is not purely
//!    hex/decimal (`[ERROR]`, `[shard-3]`) is left alone, so level tags and
//!    ids survive untouched.
//!
//! Known adversarial bias (documented, not fixed): a bracketed token made
//! purely of hex *letters* (`[deadbeef]`) masks like a pid even when it is
//! actually a meaningful id — the same class of trade-off
//! [`super::estimator::HeuristicEstimator`] already documents for its own
//! hex-letter corpus.

/// Mask `line`'s volatile prefix. Returns `None` when neither pattern
/// matched, so the caller can tell "not modified" apart from "matched and
/// the masked form happens to equal the input" (impossible here, but keeps
/// the contract explicit for callers that key on the `Option`).
pub(crate) fn mask_volatile_prefix(line: &str) -> Option<String> {
    let mut rest = line;
    let mut changed = false;
    if let Some(after) = strip_iso_timestamp(rest).or_else(|| strip_syslog_timestamp(rest)) {
        rest = after;
        changed = true;
    }
    while let Some(after) = strip_bracketed_counter(rest) {
        rest = after;
        changed = true;
    }
    changed.then(|| format!("<TS>{rest}"))
}

/// `s`'s first `n` bytes are ASCII digits: the remainder after them.
fn strip_digits(s: &str, n: usize) -> Option<&str> {
    let head = s.get(..n)?;
    head.bytes()
        .all(|byte| byte.is_ascii_digit())
        .then(|| &s[n..])
}

/// A syslog day field: a two-digit day (`"18"`), or a space-padded
/// single-digit day (`" 8"`, i.e. one more space then one digit) — syslog
/// pads single-digit days with a space, not a zero.
fn strip_syslog_day(s: &str) -> Option<&str> {
    strip_digits(s, 2).or_else(|| strip_digits(s.strip_prefix(' ')?, 1))
}

/// `YYYY-MM-DD(T| )HH:MM:SS` plus an optional fractional-second suffix and
/// an optional UTC/offset suffix.
fn strip_iso_timestamp(s: &str) -> Option<&str> {
    let s = strip_digits(s, 4)?;
    let s = s.strip_prefix('-')?;
    let s = strip_digits(s, 2)?;
    let s = s.strip_prefix('-')?;
    let s = strip_digits(s, 2)?;
    let s = s.strip_prefix('T').or_else(|| s.strip_prefix(' '))?;
    let s = strip_digits(s, 2)?;
    let s = s.strip_prefix(':')?;
    let s = strip_digits(s, 2)?;
    let s = s.strip_prefix(':')?;
    let s = strip_digits(s, 2)?;
    Some(strip_offset(strip_fraction(s)))
}

/// An optional `.123` / `,123` fractional-second suffix.
fn strip_fraction(s: &str) -> &str {
    let Some(rest) = s.strip_prefix('.').or_else(|| s.strip_prefix(',')) else {
        return s;
    };
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        s
    } else {
        &rest[digits..]
    }
}

/// An optional `Z` or `+HH:MM` / `-HH:MM` offset suffix.
fn strip_offset(s: &str) -> &str {
    if let Some(rest) = s.strip_prefix('Z') {
        return rest;
    }
    let Some(rest) = s.strip_prefix('+').or_else(|| s.strip_prefix('-')) else {
        return s;
    };
    strip_digits(rest, 2)
        .and_then(|r| r.strip_prefix(':'))
        .and_then(|r| strip_digits(r, 2))
        .unwrap_or(s)
}

/// `Mon DD HH:MM:SS` (three-letter month, space-padded single-digit day).
fn strip_syslog_timestamp(s: &str) -> Option<&str> {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let month = MONTHS.iter().find(|m| s.starts_with(*m))?;
    let s = &s[month.len()..];
    let s = s.strip_prefix(' ')?;
    let s = strip_syslog_day(s)?;
    let s = s.strip_prefix(' ')?;
    let s = strip_digits(s, 2)?;
    let s = s.strip_prefix(':')?;
    let s = strip_digits(s, 2)?;
    let s = s.strip_prefix(':')?;
    strip_digits(s, 2)
}

/// A leading `[hex-or-decimal]` token (a pid or hex counter), skipping one
/// leading space first. `None` when the bracket's content is empty or not
/// purely hex/decimal, so level tags (`[ERROR]`) and named ids
/// (`[shard-3]`) never match.
fn strip_bracketed_counter(s: &str) -> Option<&str> {
    let s = s.strip_prefix(' ').unwrap_or(s);
    let rest = s.strip_prefix('[')?;
    let end = rest.find(']')?;
    let token = &rest[..end];
    (!token.is_empty() && token.bytes().all(|b| b.is_ascii_hexdigit())).then(|| &rest[end + 1..])
}

#[cfg(test)]
#[path = "log_normalize_tests.rs"]
mod tests;
