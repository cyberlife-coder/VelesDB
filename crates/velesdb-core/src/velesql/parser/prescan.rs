//! Cheap, allocation-free pre-scan of the raw query string (GitHub #896).
//!
//! `pest` materialises the FULL recursive parse tree before any Rust callback
//! runs, so the AST-level depth guards (`MAX_CONDITION_DEPTH`, `max_ast_depth`)
//! and the length check in [`super::super::validation`] fire only *after* the
//! tree is built. A deeply nested `(((…)))` (WHERE `primary_expr`, ORDER BY
//! `arithmetic_atom`, or nested `(SELECT …)` `subquery_expr`) — or a
//! bracket-free `NOT NOT NOT …` chain (`not_expr = { ^"NOT" ~ primary_expr }`)
//! — therefore overflows the native stack inside `VelesQLParser::parse` itself,
//! crashing the process (SIGABRT) before any guard can run.
//!
//! This module runs a single O(n) pass over the raw bytes BEFORE pest is ever
//! invoked, rejecting queries whose effective parse-recursion depth exceeds a
//! small bound. The depth metric is `bracket_depth + pending_prefix_run`:
//!
//! * `bracket_depth` — open `()`/`[]` not yet closed (drives `primary_expr`,
//!   `arithmetic_atom`, `subquery_expr`, vector/array recursion).
//! * `pending_prefix_run` — consecutive leading `NOT` tokens not yet followed
//!   by an operand. `not_expr = { ^"NOT" ~ primary_expr }` recurses WITHOUT
//!   any bracket, so a bracket-only scan misses `"NOT ".repeat(N)`; each `NOT`
//!   nonetheless stacks one `primary_expr → not_expr → primary_expr` frame in
//!   pest. The run resets to 0 as soon as any non-`NOT` operand token is read.
//!   `NOT` is the ONLY bracket-free recursive prefix in `grammar.pest`: the
//!   leading `-` in `integer`/`float` is part of the atomic literal token and
//!   the arithmetic `+`/`-` (`add_op`/`sub_op`) are binary infix, so neither
//!   drives prefix recursion.
//!
//! Parentheses/brackets inside string literals (`'…'`), backtick identifiers
//! (`` `…` ``), double-quoted identifiers (`"…"`) and `--` line comments are
//! ignored so the guard never produces a false positive on quoted or commented
//! payload content. Comments are skipped in the SAME linear pass, matching the
//! grammar exactly (`COMMENT = _{ "--" ~ (!"\n" ~ ANY)* }`; the grammar has no
//! block-comment form), so an apostrophe inside a comment can no longer poison
//! the quote state and hide later real brackets.

use crate::velesql::error::{ParseError, ParseErrorKind};

/// Maximum effective parse-recursion depth accepted in a raw query.
///
/// `pest`'s recursive descent overflows the native stack somewhere in the low
/// thousands of nesting levels (the reproduced #896 vectors used ~3000–5000
/// levels of either brackets or bracket-free `NOT`). We pick 64 — the same
/// order of magnitude as the existing `max_ast_depth` budget (64) and far below
/// any value that can exhaust the stack — so the bound leaves a
/// multiple-orders-of-magnitude safety margin while remaining well above any
/// realistic hand-written query (legitimate queries nest a handful of levels of
/// parentheses, vector arrays, subqueries or `NOT`, never dozens).
const MAX_NESTING_DEPTH: usize = 64;

/// Lexer state while scanning: whether we are inside a quoted span where
/// brackets must be ignored.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ScanState {
    /// Outside any quoted span; brackets count.
    Normal,
    /// Inside a single-quoted string literal (`'…'`, `''` escapes).
    SingleQuote,
    /// Inside a double-quoted identifier (`"…"`, `""` escapes).
    DoubleQuote,
    /// Inside a backtick identifier (`` `…` ``, no escapes).
    Backtick,
}

/// Mutable scan cursor: bracket depth plus the pending leading-`NOT` run, with
/// the running maximum of their sum (the effective recursion depth).
struct Depth {
    /// Open `()`/`[]` not yet closed.
    brackets: usize,
    /// Consecutive leading `NOT` tokens not yet followed by an operand.
    prefix_run: usize,
    /// Highest `brackets + prefix_run` observed so far.
    max: usize,
}

impl Depth {
    fn new() -> Self {
        Self {
            brackets: 0,
            prefix_run: 0,
            max: 0,
        }
    }

    /// Records the current effective depth into the running maximum.
    fn observe(&mut self) {
        let effective = self.brackets + self.prefix_run;
        if effective > self.max {
            self.max = effective;
        }
    }
}

/// Validates raw query length and recursion depth before parsing.
///
/// Runs first on the hot path: a single linear pass, no allocations.
///
/// # Errors
///
/// Returns [`ParseErrorKind::ComplexityLimit`] if the query is longer than
/// `max_query_length` or its effective recursion depth (open `()`/`[]` plus the
/// pending leading-`NOT` run) exceeds [`MAX_NESTING_DEPTH`].
pub(super) fn prescan(input: &str, max_query_length: usize) -> Result<(), ParseError> {
    if input.len() > max_query_length {
        return Err(length_error(input, max_query_length));
    }
    check_nesting_depth(input)
}

/// Linear scan computing the maximum effective recursion depth outside quotes
/// and comments.
fn check_nesting_depth(input: &str) -> Result<(), ParseError> {
    let bytes = input.as_bytes();
    let mut state = ScanState::Normal;
    let mut depth = Depth::new();
    let mut i = 0;
    while i < bytes.len() {
        i += step(bytes, i, &mut state, &mut depth);
    }
    if depth.max > MAX_NESTING_DEPTH {
        return Err(depth_error(bytes, bytes.len().saturating_sub(1)));
    }
    Ok(())
}

/// Processes one position; returns how many bytes were consumed (>= 1).
fn step(bytes: &[u8], i: usize, state: &mut ScanState, depth: &mut Depth) -> usize {
    match *state {
        ScanState::Normal => step_normal(bytes, i, state, depth),
        ScanState::SingleQuote => step_quoted(bytes, i, bytes[i], b'\'', state),
        ScanState::DoubleQuote => step_quoted(bytes, i, bytes[i], b'"', state),
        ScanState::Backtick => {
            if bytes[i] == b'`' {
                *state = ScanState::Normal;
            }
            1
        }
    }
}

/// Handles a byte while outside any quoted span or comment.
fn step_normal(bytes: &[u8], i: usize, state: &mut ScanState, depth: &mut Depth) -> usize {
    let b = bytes[i];
    if starts_line_comment(bytes, i) {
        return skip_line_comment(bytes, i);
    }
    match b {
        b'(' | b'[' => open_bracket(depth),
        b')' | b']' => depth.brackets = depth.brackets.saturating_sub(1),
        b'\'' => *state = ScanState::SingleQuote,
        b'"' => *state = ScanState::DoubleQuote,
        b'`' => *state = ScanState::Backtick,
        _ if is_word_byte(b) => return word_token(bytes, i, depth),
        _ => {}
    }
    1
}

/// True when position `i` begins a `--` line comment (grammar `COMMENT`).
fn starts_line_comment(bytes: &[u8], i: usize) -> bool {
    bytes[i] == b'-' && bytes.get(i + 1) == Some(&b'-')
}

/// Opens a `(`/`[`: an operand boundary, so the pending prefix run folds into
/// bracket depth (the `NOT`s wrap the bracketed expression) and resets.
/// Records the new effective depth.
fn open_bracket(depth: &mut Depth) {
    depth.brackets += depth.prefix_run + 1;
    depth.prefix_run = 0;
    depth.observe();
}

/// Consumes a maximal run of identifier bytes (one token) and updates the
/// prefix run: a `NOT` keyword extends it (deeper `not_expr` recursion), any
/// other word token is an operand and resets it. Returns the token length.
fn word_token(bytes: &[u8], i: usize, depth: &mut Depth) -> usize {
    let mut end = i;
    while end < bytes.len() && is_word_byte(bytes[end]) {
        end += 1;
    }
    if is_not_keyword(&bytes[i..end]) {
        depth.prefix_run += 1;
        depth.observe();
    } else {
        depth.prefix_run = 0;
    }
    end - i
}

/// Skips a `--` line comment up to (but not including) the next `\n`, so that
/// quotes and brackets inside the comment are never interpreted. Returns the
/// number of bytes consumed.
fn skip_line_comment(bytes: &[u8], i: usize) -> usize {
    let mut end = i + 2;
    while end < bytes.len() && bytes[end] != b'\n' {
        end += 1;
    }
    end - i
}

/// True for bytes that form a `VelesQL` `regular_identifier`
/// (`ASCII_ALPHA | "_" | ASCII_ALPHANUMERIC`).
fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Case-insensitive exact match of the keyword `NOT` on a token slice. Because
/// the caller passes a maximal identifier run, this can never match a `NOT`
/// substring of a longer identifier (e.g. `NOTES`, `not_deleted`).
fn is_not_keyword(token: &[u8]) -> bool {
    token.len() == 3
        && token[0].eq_ignore_ascii_case(&b'N')
        && token[1].eq_ignore_ascii_case(&b'O')
        && token[2].eq_ignore_ascii_case(&b'T')
}

/// Handles a byte inside a `quote`-delimited span supporting `quote``quote`
/// doubling as an escape. Returns bytes consumed (2 when an escape is skipped).
fn step_quoted(bytes: &[u8], i: usize, b: u8, quote: u8, state: &mut ScanState) -> usize {
    if b != quote {
        return 1;
    }
    if bytes.get(i + 1) == Some(&quote) {
        // Doubled quote: escaped delimiter, stay inside the span.
        return 2;
    }
    *state = ScanState::Normal;
    1
}

/// Builds the over-length rejection error.
fn length_error(input: &str, max: usize) -> ParseError {
    ParseError::new(
        ParseErrorKind::ComplexityLimit,
        max,
        input.chars().take(128).collect::<String>(),
        format!("Query length exceeded: max={max}, actual={}", input.len()),
    )
}

/// Builds the excessive-nesting rejection error.
fn depth_error(bytes: &[u8], position: usize) -> ParseError {
    let start = position.saturating_sub(32);
    let end = position.min(bytes.len().saturating_sub(1));
    let fragment = String::from_utf8_lossy(&bytes[start..=end]).into_owned();
    ParseError::new(
        ParseErrorKind::ComplexityLimit,
        position,
        fragment,
        format!("Query nesting too deep: max={MAX_NESTING_DEPTH} levels of recursion"),
    )
}
