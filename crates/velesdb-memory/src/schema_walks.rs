//! Tree walks the wire schemas run before inlining: the rustdoc-link rewrite
//! of every `description` (#2261), which both `harden`s apply, and the id
//! widening, which only `WireOutputSchema::harden` applies. Its entry points
//! are `pub(super)`, private to `schema`; the split keeps `schema.rs` within
//! its file budget.

use serde_json::{Map, Value};
use std::borrow::Cow;

/// Rewrites rustdoc link syntax in every `description` of a published schema
/// into the text rustdoc shows for it (#2261).
///
/// schemars copies doc comments verbatim, so an intra-doc link reached the
/// wire as Markdown no client can resolve: ``[`X`](crate::path)`` renders as a
/// broken link, ``[`X`]`` as bracketed text. The doc comments stay
/// rustdoc-correct; only what the schema publishes changes. A `description`
/// inside instance data ([`INSTANCE_KEYWORDS`]) is a value, not a doc
/// comment, and stays.
pub(super) fn unlink_rustdoc_descriptions(map: &mut Map<String, Value>) {
    for (key, value) in map.iter_mut() {
        match (key.as_str(), value) {
            ("description", Value::String(text)) => {
                if let Some(unlinked) = unlink_rustdoc(text) {
                    *text = unlinked;
                }
            }
            (keyword, _) if INSTANCE_KEYWORDS.contains(&keyword) => {}
            // Keys here are names, not keywords: a property may be called
            // `default` or `description`.
            (keyword, Value::Object(named)) if NAMED_SCHEMA_MAPS.contains(&keyword) => {
                named.values_mut().for_each(unlink_in_value);
            }
            (_, value) => unlink_in_value(value),
        }
    }
}

fn unlink_in_value(value: &mut Value) {
    match value {
        Value::Object(map) => unlink_rustdoc_descriptions(map),
        Value::Array(items) => items.iter_mut().for_each(unlink_in_value),
        _ => {}
    }
}

/// Schema keywords whose values are instance data, not schemas.
pub(super) const INSTANCE_KEYWORDS: [&str; 5] = ["default", "examples", "example", "const", "enum"];

/// Schema keywords whose values map names to schemas.
pub(super) const NAMED_SCHEMA_MAPS: [&str; 6] = [
    "properties",
    "patternProperties",
    "$defs",
    "definitions",
    "dependentSchemas",
    "dependencies",
];

/// The kinds rustdoc accepts before `@` in an intra-doc link (`fn@build`),
/// and drops from the text it shows.
const DISAMBIGUATORS: [&str; 22] = [
    "struct",
    "enum",
    "trait",
    "union",
    "mod",
    "module",
    "const",
    "constant",
    "static",
    "fn",
    "function",
    "method",
    "derive",
    "field",
    "variant",
    "type",
    "value",
    "macro",
    "prim",
    "primitive",
    "tyalias",
    "typealias",
];

/// `text` with each rustdoc link replaced by what rustdoc shows for it, or
/// `None` when it holds none: ``[`X`](path)`` and ``[`X`]`` become `` `X` ``,
/// `[Name](path)` becomes `Name`, and a path-like shortcut (`[a::B]`, `[f()]`,
/// `[m!]`, `[fn@f]`) shows its path without the disambiguator. A bare
/// `[name]` stays whether or not rustdoc resolves it (`map[key]`, `[sic]`),
/// and so do `[0, 1]`, a bracketed code span that is not one word or that
/// rustdoc does not resolve (``[`a.b`]``), a web link and reference-style
/// links (`[text][label]`).
///
/// It also leaves as written every link in a text it cannot read exactly
/// ([`scan_is_exact`]) or that holds an inline link it does not render (a web
/// link, an image), whose target and title it cannot read as prose; a
/// `[label]` a colon follows; and a text a second pass would change further,
/// such as nested brackets (``[[`X`]]``, whose outer pair Markdown shows as
/// written). The guard then
/// fails on the rustdoc link syntax left, except a bare `[Name]`, which it
/// cannot tell from prose.
///
/// One pass is final: an input schema can be hardened twice (at its tool
/// attribute, then in `reharden_tool_input`) and must publish what an output
/// schema, hardened once, publishes for the same doc comment.
pub(super) fn unlink_rustdoc(text: &str) -> Option<String> {
    if !scan_is_exact(text) {
        return None;
    }
    let Pass::Changed(once) = unlink_once(text) else {
        return None;
    };
    matches!(unlink_once(&once), Pass::Unchanged).then_some(once)
}

/// What one pass of the rewrite makes of a text.
enum Pass {
    /// It holds no link to rewrite.
    Unchanged,
    /// The text with its links rewritten.
    Changed(String),
    /// It holds an inline link the rewrite does not render: the text stays as
    /// written.
    Leave,
}

/// Whether the scan reads `text` as Markdown does. It models inline code
/// spans that close on their own line and the links around them, in any
/// paragraph, heading, quote or list item, and leaves a text holding
/// anything else:
///
/// - a backslash, which can escape a bracket or a backtick;
/// - a tab or four spaces, which can open an indented code block, in or out
///   of a quote or a list item;
/// - a code fence (```` ``` ```` or `~~~`), wherever it stands;
/// - a `<` outside a code span, which can open HTML or an autolink;
/// - a table, in or out of a quote, which splits its cells before it reads
///   code spans;
/// - a code span that crosses a line, whose extent depends on the blocks
///   around it.
fn scan_is_exact(text: &str) -> bool {
    !text.contains(['\\', '\t'])
        && !text.contains("    ")
        && !text.contains("```")
        && !text.contains("~~~")
        && !outside_code_spans(text).any(|part| part.contains('<'))
        && !text.split(LINE_ENDINGS).any(is_a_table_delimiter_row)
        && !has_a_code_span_across_lines(text)
}

/// Whether `line` can be the delimiter row that makes the lines above it a
/// table (`|---|:--:|`): nothing but pipes, colons, dashes, spaces and the
/// `>` of a quote, with a pipe among them.
fn is_a_table_delimiter_row(line: &str) -> bool {
    line.contains('|') && line.chars().all(|c| "|:- >".contains(c))
}

/// Whether a code span of `text` crosses a line.
fn has_a_code_span_across_lines(text: &str) -> bool {
    let mut rest = text;
    while let Some(at) = rest.find('`') {
        let span = &rest[at..at + code_span_len(&rest[at..])];
        if spans_a_line(span) {
            return true;
        }
        rest = &rest[at + span.len()..];
    }
    false
}

/// One left-to-right pass of [`unlink_rustdoc`].
fn unlink_once(text: &str) -> Pass {
    if !text.contains('[') {
        return Pass::Unchanged;
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    let mut changed = false;
    while let Some(pos) = rest.find(['[', '`']) {
        out.push_str(&rest[..pos]);
        let marker = &rest[pos..];
        if marker.starts_with('`') {
            // A code span is copied as it stands: brackets inside it are
            // code (`decisions[fragment_index]`), not links.
            let span = code_span_len(marker);
            out.push_str(&marker[..span]);
            rest = &marker[span..];
            continue;
        }
        let after = &marker[1..];
        let Some((shown, remaining)) = link_at(&out, after) else {
            if opens_an_inline_link(after) {
                return Pass::Leave;
            }
            out.push('[');
            rest = after;
            continue;
        };
        push_apart(&mut out, &shown, remaining);
        rest = remaining;
        changed = true;
    }
    out.push_str(rest);
    if changed {
        Pass::Changed(out)
    } else {
        Pass::Unchanged
    }
}

/// Whether the `[` that `after` follows opens an inline link: its label can
/// be a link's text ([`can_be_link_text`]) and a `(` follows it.
fn opens_an_inline_link(after: &str) -> bool {
    label_end(after).is_some_and(|close| {
        can_be_link_text(&after[..close]) && after[close + 1..].starts_with('(')
    })
}

/// What rustdoc shows for the link the `[` between `before` and `after` opens,
/// and the text after it, when the rewrite reads one there. A `[` right after
/// `]` is a reference label (`[a][b]`), and one right after `!` opens an
/// image (`![a](b)`). A `[label]` a colon follows is a reference definition
/// wherever a block starts, in a quote or a list item too, and a link
/// elsewhere: rather than tell the two apart, the rewrite leaves all three.
fn link_at<'a>(before: &str, after: &'a str) -> Option<(Cow<'a, str>, &'a str)> {
    let defines = label_end(after).is_some_and(|close| after[close + 1..].starts_with(':'));
    if defines || before.ends_with([']', '!']) {
        return None;
    }
    rustdoc_link(after)
}

/// Appends `shown` to `out`, a space apart from a code span on either side:
/// two touching spans (`a``b`) read as one.
fn push_apart(out: &mut String, shown: &str, remaining: &str) {
    if out.ends_with('`') && shown.starts_with('`') {
        out.push(' ');
    }
    out.push_str(shown);
    if shown.ends_with('`') && remaining.starts_with('`') {
        out.push(' ');
    }
}

/// Length of the code span `text` starts with: its opening run of backticks
/// through the next run of exactly as many, or just that opening run when
/// none closes it: in Markdown, an unclosed run of backticks is literal
/// text. A span that crosses a line is not modelled ([`scan_is_exact`]).
fn code_span_len(text: &str) -> usize {
    let opening = text.bytes().take_while(|&b| b == b'`').count();
    let mut search = opening;
    while let Some(found) = text[search..].find('`') {
        let at = search + found;
        let run = text[at..].bytes().take_while(|&b| b == b'`').count();
        if run == opening {
            return at + run;
        }
        search = at + run;
    }
    opening
}

/// The parts of `text` outside its code spans, in order, which the rewrite
/// reads for link syntax. A run of backticks nothing closes is
/// literal text that holds none, and is left out.
fn outside_code_spans(text: &str) -> impl Iterator<Item = &str> {
    let mut rest = text;
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        let at = rest.find('`').unwrap_or(rest.len());
        let (part, span) = rest.split_at(at);
        rest = &span[code_span_len(span)..];
        Some(part)
    })
}

/// Where the label `after` starts ends: its first `]` outside a code span, as
/// Markdown reads it, so the `]` in ``[`a]b`](c)`` is code.
fn label_end(after: &str) -> Option<usize> {
    let mut at = 0;
    while let Some(found) = after[at..].find(['`', ']']) {
        let i = at + found;
        if after[i..].starts_with(']') {
            return Some(i);
        }
        at = i + code_span_len(&after[i..]);
    }
    None
}

/// When `after` (the text following a `[`) starts a rustdoc link, what
/// rustdoc shows for it and the text after the link. A reference-style link
/// (`[text][label]`) is left as written.
fn rustdoc_link(after: &str) -> Option<(Cow<'_, str>, &str)> {
    let close = label_end(after)?;
    let (label, tail) = (&after[..close], &after[close + 1..]);
    if !can_be_link_text(label) {
        return None;
    }
    if let Some(inline) = tail.strip_prefix('(') {
        return inline_link(label, inline);
    }
    if tail.starts_with('[') {
        return None;
    }
    if is_code_span(label) {
        return code_link(label, tail);
    }
    is_path_like(label).then(|| {
        (
            Cow::Borrowed(without_disambiguator(label).unwrap_or(label)),
            tail,
        )
    })
}

/// Whether `label` can be a link's text. A `[` outside the label's code spans
/// is one left open earlier, not this link's: the scan moves on to the one
/// just before the `]`. Inside a code span a `[` is code (``[a `[` b](c)``).
/// A label that spans a line is left as written (see [`spans_a_line`]).
fn can_be_link_text(label: &str) -> bool {
    !outside_code_spans(label).any(|part| part.contains('[')) && !spans_a_line(label)
}

/// An inline link, `inline` being the text after its `(`: shown as its label
/// when the target is a Rust path. The target may be padded with spaces, as
/// Markdown allows; one wrapped in `<…>` never gets here, since a `<` outside
/// code leaves the whole text as written ([`scan_is_exact`]). A target that
/// spans a line is left as written (see [`spans_a_line`]).
fn inline_link<'a>(label: &'a str, inline: &'a str) -> Option<(Cow<'a, str>, &'a str)> {
    let end = closing_paren(inline)?;
    let raw = &inline[..end];
    if spans_a_line(raw) {
        return None;
    }
    is_rust_path(raw.trim()).then(|| (Cow::Borrowed(label), &inline[end + 1..]))
}

/// Whether `text` holds a line ending. Markdown lets a link span lines, but
/// what a line ending allows there depends on the next line: a blank line, a
/// heading or a list item ends the paragraph. No published doc comment writes
/// one, so the rewrite leaves such a link as written, and the guard fails on
/// it as on any link syntax the rewrite leaves.
fn spans_a_line(text: &str) -> bool {
    text.contains(LINE_ENDINGS)
}

/// What ends a line in Markdown: a line feed or a carriage return.
const LINE_ENDINGS: [char; 2] = ['\n', '\r'];

/// A code link, ``[`code`]``: shown as its code span when the code is one
/// word rustdoc resolves ([`reads_as_a_path`]), without its disambiguator.
fn code_link<'a>(label: &'a str, tail: &'a str) -> Option<(Cow<'a, str>, &'a str)> {
    // rustdoc trims the link text, then the path after a disambiguator.
    let code = label[1..label.len() - 1].trim();
    let word = without_disambiguator(code).map_or(code, str::trim);
    if !is_one_word(word) || !reads_as_a_path(word) {
        return None;
    }
    let shown = if word.len() + 2 == label.len() {
        Cow::Borrowed(label)
    } else {
        Cow::Owned(format!("`{word}`"))
    };
    Some((shown, tail))
}

/// The path rustdoc 1.90 resolves for the link text `word`: `word` past a call
/// suffix such as `()`, when something is left. A `#` fragment is not modelled:
/// a link with one stays as written, and the guard fails on it.
fn rustdoc_path(word: &str) -> &str {
    CALL_SUFFIXES
        .iter()
        .find_map(|suffix| word.strip_suffix(suffix).filter(|rest| !rest.is_empty()))
        .unwrap_or(word)
}

/// Whether the rewrite reads the link text `word` as a path: its
/// [`rustdoc_path`] holds only letters, digits and ``:_<>, !*&;``, the test
/// rustdoc 1.90 applies. rustdoc leaves a shortcut link that fails it as
/// written, brackets and all (``[`a[`]``, ``[`a.b`]``, ``[`()`]``). A `#`
/// fragment fails it too, though rustdoc resolves the part before it: the
/// rewrite leaves ``[`a#b`]`` as written, and the guard fails on it.
fn reads_as_a_path(word: &str) -> bool {
    rustdoc_path(word)
        .chars()
        .all(|c| c.is_alphanumeric() || ":_<>, !*&;".contains(c))
}

/// A single inline code span — the label of a rustdoc code link.
fn is_code_span(label: &str) -> bool {
    label.len() > 2
        && label.starts_with('`')
        && label.ends_with('`')
        && !label[1..label.len() - 1].contains('`')
}

/// A shortcut label rustdoc reads as a path rather than prose: one with a
/// `::`, a [`CALL_SUFFIXES`] suffix, or a disambiguator. A bare `[Name]` may be
/// either (`map[key]`, `[sic]`) and stays as written.
fn is_path_like(label: &str) -> bool {
    is_rust_path(label)
        && (label.contains("::")
            || CALL_SUFFIXES.iter().any(|suffix| label.ends_with(suffix))
            || without_disambiguator(label).is_some())
}

/// What rustdoc accepts after a function or macro name: `f()`, `m!`, `m!()`,
/// `m!{}`. A form not listed, such as `m![]`, stays as written, and the guard
/// fails on it.
const CALL_SUFFIXES: [&str; 4] = ["!()", "!{}", "()", "!"];

/// Whether a code span reads as a link to the rewrite: non-empty and, with
/// balanced generic arguments dropped, one word. `Vec<T>` and `HashMap<K, V>`
/// are; `0, 1`, `a | b` and the unbalanced `a<b c` and `Vec<T>>` are not. A
/// heuristic: rustdoc also needs the name to resolve, which a schema cannot
/// check.
fn is_one_word(code: &str) -> bool {
    let mut depth = 0usize;
    let mut seen = false;
    for c in code.chars() {
        match c {
            '<' => depth += 1,
            '>' if depth == 0 => return false,
            '>' => depth -= 1,
            c if depth == 0 && c.is_whitespace() => return false,
            _ if depth == 0 => seen = true,
            _ => {}
        }
    }
    seen && depth == 0
}

/// Where an inline link's destination ends: its first `)` outside balanced
/// parentheses, as Markdown reads it, so `f()` and `m!()` stay inside.
fn closing_paren(destination: &str) -> Option<usize> {
    let mut depth = 0usize;
    for (i, c) in destination.char_indices() {
        match c {
            '(' => depth += 1,
            ')' if depth == 0 => return Some(i),
            ')' => depth -= 1,
            _ => {}
        }
    }
    None
}

/// `target` without its `kind@` prefix, when rustdoc accepts that kind.
fn without_disambiguator(target: &str) -> Option<&str> {
    target
        .split_once('@')
        .filter(|(kind, _)| DISAMBIGUATORS.contains(kind))
        .map(|(_, path)| path)
}

/// A path rustdoc resolves — `crate::a::B`, `super::f`, `Self::g`, `Name`,
/// `fn@name`, `f()`, `m!` — as opposed to a URL or prose.
fn is_rust_path(target: &str) -> bool {
    let path = match without_disambiguator(target) {
        Some(path) => path,
        None if target.contains('@') => return false,
        None => target,
    };
    let path = rustdoc_path(path);
    !path.is_empty()
        && path.split("::").all(|segment| {
            segment.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
                && segment
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
}

/// Recursively widen every property named in `keys` (resolving the `items`
/// of array-typed ones) from `integer` to `["integer", "string"]`, across
/// the whole schema tree — `$defs` included.
///
/// The advertised-schema counterpart of the `context::wire` id contract:
/// under `CompilePolicy::ids_as_strings` a response id field crosses as a
/// decimal string, and `fragments[].id` accepts one on input — and the
/// official MCP SDKs validate `structuredContent` against the advertised
/// `outputSchema` (spec 2025-06-18), so a schema typing those fields
/// `integer` only would make every opted-in response fail validation for
/// exactly the clients the option exists for. Same shape of tree walk as
/// [`strip_int_formats`](super::strip_int_formats), but keyed: only the named properties widen.
///
/// `mcp`-gated: the advertised tool schemas are its only consumer.
pub(super) fn widen_id_properties(map: &mut Map<String, Value>, keys: &[&str]) {
    if let Some(Value::Object(properties)) = map.get_mut("properties") {
        for (name, subschema) in properties.iter_mut() {
            if keys.contains(&name.as_str()) {
                widen_id_schema(subschema);
            }
        }
    }
    for value in map.values_mut() {
        widen_in_value(value, keys);
    }
}

fn widen_in_value(value: &mut Value, keys: &[&str]) {
    match value {
        Value::Object(map) => widen_id_properties(map, keys),
        Value::Array(items) => items.iter_mut().for_each(|item| widen_in_value(item, keys)),
        _ => {}
    }
}

/// Widen one id property's schema: `integer` → `["integer", "string"]`
/// (keeping any `null` of an optional field), recursing into `items` for an
/// array of ids. `minimum: 0` may stay — JSON Schema numeric keywords apply
/// to numbers only, so the string form is unaffected.
fn widen_id_schema(schema: &mut Value) {
    let Value::Object(map) = schema else {
        return;
    };
    match map.get("type").cloned() {
        Some(Value::String(kind)) if kind == "integer" => {
            map.insert(
                "type".to_owned(),
                Value::Array(vec![
                    Value::String("integer".to_owned()),
                    Value::String("string".to_owned()),
                ]),
            );
        }
        Some(Value::String(kind)) if kind == "array" => {
            if let Some(items) = map.get_mut("items") {
                widen_id_schema(items);
            }
        }
        Some(Value::Array(mut kinds)) => {
            let has_integer = kinds.iter().any(|kind| kind == "integer");
            let has_string = kinds.iter().any(|kind| kind == "string");
            if has_integer && !has_string {
                let after = kinds
                    .iter()
                    .position(|kind| kind == "integer")
                    .map_or(kinds.len(), |position| position + 1);
                kinds.insert(after, Value::String("string".to_owned()));
                map.insert("type".to_owned(), Value::Array(kinds));
            }
        }
        _ => {}
    }
}
