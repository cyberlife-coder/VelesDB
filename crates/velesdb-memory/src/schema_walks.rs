//! Tree walks the wire schemas run before inlining: the rustdoc-link rewrite
//! of every `description` (#2261), which both `harden`s apply, and the id
//! widening, which only `WireOutputSchema::harden` applies. Its entry points
//! are `pub(super)`, private to `schema`. `schema.rs` is over its file
//! budget, so these walks live here rather than grow it.

use pulldown_cmark::{
    BrokenLink, BrokenLinkCallback, CowStr, Event, LinkType, Options, Parser, Tag, TagEnd,
};
use serde_json::{Map, Value};
use std::ops::Range;

/// Rewrites the rustdoc links in every `description` of a published schema
/// into their text (#2261), read with pulldown-cmark ([`unlink_rustdoc`]).
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

/// The kinds rustdoc 1.90 accepts before `@` in an intra-doc link
/// (`fn@build`), and drops from the text it shows. It knows no `tyalias@` or
/// `typealias@`: a link with one is no rustdoc link here.
const DISAMBIGUATORS: [&str; 20] = [
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
];

/// The suffixes rustdoc reads as a disambiguator after a path: a function's
/// `()`, a macro's `!`, `!()` or `!{}`.
const CALL_SUFFIXES: [&str; 4] = ["!()", "!{}", "()", "!"];

/// What the descriptions are parsed as: `CommonMark` with the extensions a
/// client renderer commonly reads (tables, footnotes, strikethrough, task
/// lists), plus smart punctuation. Smart punctuation only makes the rewrite
/// refuse more: a quote it reads differently once a link's brackets go is a
/// difference the round trip in [`unlink_rustdoc`] sees.
const MARKDOWN: Options = Options::ENABLE_TABLES
    .union(Options::ENABLE_FOOTNOTES)
    .union(Options::ENABLE_STRIKETHROUGH)
    .union(Options::ENABLE_TASKLISTS)
    .union(Options::ENABLE_SMART_PUNCTUATION);

/// `text` with each rustdoc link replaced by its text, or `None` when it
/// holds none, or when the rewrite would change anything else.
///
/// The links are the ones pulldown-cmark reads, so a code span, a code block
/// or an escaped bracket never holds one. A link is a rustdoc link when its
/// destination reads as an item path ([`is_rustdoc_target`]): an inline link
/// (``[`X`](crate::X)``), a link to a definition (`[x][y]` or `[x]` with
/// `[y]: crate::X`), and a reference no definition resolves, which the
/// broken-link callback accepts as a link the way rustdoc finds intra-doc
/// links ([`resolve_rustdoc_reference`]). A web link, a relative URL and a
/// bare `[name]` stay. The link becomes its text as written:
/// ``[`X`](crate::X)`` becomes `` `X` ``, `[the point](crate::X)` becomes
/// `the point`, and a shortcut code link drops its disambiguator
/// (``[`fn@f`]`` becomes `` `f` ``), as rustdoc shows it. A definition of an
/// item path is removed.
///
/// Fail closed by construction: the rewritten text is parsed again, and it
/// must read as the original with those links' tags dropped, event for event,
/// or the text stays as written and the schema guard fails on it. A link whose
/// removal would turn a neighbour bold, merge two code spans or pair two
/// brackets is therefore left, whatever the construct. The same round trip
/// makes one pass final: the rewritten text parses to the links the first pass
/// left and no other, so a second pass finds none to rewrite (an input schema
/// is hardened twice).
pub(super) fn unlink_rustdoc(text: &str) -> Option<String> {
    let parser = parse(text).into_offset_iter();
    let mut edits: Vec<(Range<usize>, String)> = parser
        .reference_definitions()
        .iter()
        .filter(|(_, definition)| is_rustdoc_target(&definition.dest))
        .map(|(_, definition)| (with_its_line_ending(text, &definition.span), String::new()))
        .collect();
    let events: Vec<(Event<'_>, Range<usize>)> = parser.collect();
    let mut expected = Vec::with_capacity(events.len());
    let mut at = 0;
    while let Some((event, range)) = events.get(at) {
        match event {
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                ..
            }) if is_rustdoc_link(*link_type, dest_url) => {
                let end = at
                    + events[at..]
                        .iter()
                        .position(|(event, _)| matches!(event, Event::End(TagEnd::Link)))?;
                let (shown, inner) = link_text(text, *link_type, &events[at + 1..end]);
                edits.push((link_source(text, *link_type, range), shown));
                expected.extend(inner);
                at = end + 1;
            }
            _ => {
                expected.push(event.clone());
                at += 1;
            }
        }
    }
    let rewritten = apply(text, edits)?;
    reads_as(&rewritten, expected).then_some(rewritten)
}

/// Whether `rewritten` parses to `expected`, text runs joined, and defines no
/// item path: a duplicate definition, which Markdown ignores until the first
/// one goes, must not surface in its place.
fn reads_as(rewritten: &str, expected: Vec<Event<'_>>) -> bool {
    let parser = parse(rewritten);
    let defines_a_path = parser
        .reference_definitions()
        .iter()
        .any(|(_, definition)| is_rustdoc_target(&definition.dest));
    !defines_a_path && merged_text(expected) == merged_text(parser)
}

/// The source of a link pulldown-cmark reports at `range`: it ends a collapsed
/// link (`[x][]`) at its label, so the `[]` after it is added.
fn link_source(text: &str, link_type: LinkType, range: &Range<usize>) -> Range<usize> {
    const COLLAPSED: &str = "[]";
    let collapsed = matches!(link_type, LinkType::Collapsed | LinkType::CollapsedUnknown)
        && text[range.end..].starts_with(COLLAPSED);
    range.start..range.end + if collapsed { COLLAPSED.len() } else { 0 }
}

/// A definition's `span`, with the line ending after it: removing the
/// definition removes its line.
fn with_its_line_ending(text: &str, span: &Range<usize>) -> Range<usize> {
    let rest = &text[span.end..];
    let ending = ["\r\n", "\n", "\r"]
        .iter()
        .find(|ending| rest.starts_with(**ending))
        .map_or(0, |ending| ending.len());
    span.start..span.end + ending
}

fn parse(text: &str) -> Parser<'_, impl BrokenLinkCallback<'_>> {
    Parser::new_with_broken_link_callback(text, MARKDOWN, Some(resolve_rustdoc_reference))
}

/// The broken-link callback: a reference no definition resolves is a link
/// when rustdoc would resolve it as an item. A reference-style link
/// (`[text][crate::X]`) names its target, so a path is enough. A shortcut or
/// collapsed link (`[X]`, `[X][]`) is also prose (`map[key]`, `[sic]`), so it
/// must be code, hold `::`, or carry a disambiguator (``[`X`]``, `[a::B]`,
/// `[fn@f]`, `[f()]`). A bare `[Name]` stays: rustdoc may resolve it, but it
/// reads as bracketed prose.
fn resolve_rustdoc_reference(link: BrokenLink<'_>) -> Option<(CowStr<'_>, CowStr<'_>)> {
    let label = link.reference.trim();
    let is_link = is_rustdoc_target(label)
        && (link.link_type == LinkType::Reference
            || label.starts_with('`')
            || label.contains("::")
            || without_disambiguator(label).is_some()
            || CALL_SUFFIXES.iter().any(|suffix| label.ends_with(suffix)));
    is_link.then_some((link.reference, CowStr::Borrowed("")))
}

/// Whether a link of `link_type` to `destination` is a rustdoc link. A link
/// the callback resolved is one by construction.
fn is_rustdoc_link(link_type: LinkType, destination: &str) -> bool {
    match link_type {
        LinkType::Inline | LinkType::Reference | LinkType::Collapsed | LinkType::Shortcut => {
            is_rustdoc_target(destination)
        }
        LinkType::ReferenceUnknown | LinkType::CollapsedUnknown | LinkType::ShortcutUnknown => true,
        _ => false,
    }
}

/// Whether `target` reads as an item path rustdoc resolves: backticks, a
/// disambiguator and generics aside, a path of letters, digits, `_`, `::`,
/// and the `&`, `*` or `;` of a primitive (`&str`, `*const`). A URL, a
/// relative path, prose with a space and a lone `:` (`mailto:`, `http:`) are
/// not.
fn is_rustdoc_target(target: &str) -> bool {
    let target = target.trim();
    let target = target
        .strip_prefix('`')
        .and_then(|code| code.strip_suffix('`'))
        .map_or(target, str::trim);
    let path = without_disambiguator(target).unwrap_or(target);
    let path = CALL_SUFFIXES
        .iter()
        .find_map(|suffix| path.strip_suffix(suffix))
        .unwrap_or(path);
    without_generics(path).is_some_and(|path| {
        path.chars().any(|c| c.is_alphabetic() || c == '_')
            && path
                .chars()
                .all(|c| c.is_alphanumeric() || "_:&*;".contains(c))
            && !path.replace("::", "").contains(':')
    })
}

/// `path` less its `<…>` groups, or `None` when they do not balance.
fn without_generics(path: &str) -> Option<String> {
    let mut depth = 0_usize;
    let mut outside = String::with_capacity(path.len());
    for c in path.chars() {
        match c {
            '<' => depth += 1,
            '>' => depth = depth.checked_sub(1)?,
            _ if depth == 0 => outside.push(c),
            _ => {}
        }
    }
    (depth == 0).then_some(outside)
}

/// `target` less a disambiguator rustdoc accepts (`fn@f` gives `f`), or
/// `None` when it carries none.
fn without_disambiguator(target: &str) -> Option<&str> {
    let (kind, path) = target.split_once('@')?;
    DISAMBIGUATORS
        .contains(&kind.trim())
        .then(|| path.trim_start())
}

/// The source a link shows, given its `inner` events, and the events the
/// rewritten text must read there. A shortcut or collapsed link whose text is
/// one code span shows that code less its disambiguator, as rustdoc does.
fn link_text<'a>(
    text: &str,
    link_type: LinkType,
    inner: &[(Event<'a>, Range<usize>)],
) -> (String, Vec<Event<'a>>) {
    let start = inner.iter().map(|(_, range)| range.start).min();
    let end = inner.iter().map(|(_, range)| range.end).max();
    let source = start.zip(end).map_or("", |(start, end)| &text[start..end]);
    let names_its_target = matches!(
        link_type,
        LinkType::Shortcut
            | LinkType::ShortcutUnknown
            | LinkType::Collapsed
            | LinkType::CollapsedUnknown
    );
    if let [(Event::Code(code), _)] = inner {
        if let Some(path) = without_disambiguator(code).filter(|_| names_its_target) {
            let shown = source.replacen(&**code, path, 1);
            return (shown, vec![Event::Code(CowStr::from(path.to_owned()))]);
        }
    }
    let events = inner.iter().map(|(event, _)| event.clone()).collect();
    (source.to_owned(), events)
}

/// `text` with each edit applied, or `None` when there is none or two
/// overlap.
fn apply(text: &str, mut edits: Vec<(Range<usize>, String)>) -> Option<String> {
    if edits.is_empty() {
        return None;
    }
    edits.sort_by_key(|(range, _)| range.start);
    let mut rewritten = String::with_capacity(text.len());
    let mut cursor = 0;
    for (range, with) in edits {
        rewritten.push_str(text.get(cursor..range.start)?);
        rewritten.push_str(&with);
        cursor = range.end;
    }
    rewritten.push_str(&text[cursor..]);
    Some(rewritten)
}

/// `events` with each run of adjacent text joined: dropping a link's tags
/// leaves its text beside the text around it, which a parse of the rewritten
/// source may read as one run, or split elsewhere.
fn merged_text<'a>(events: impl IntoIterator<Item = Event<'a>>) -> Vec<Event<'a>> {
    let mut merged: Vec<Event<'a>> = Vec::new();
    for event in events {
        if let (Some(Event::Text(before)), Event::Text(after)) = (merged.last_mut(), &event) {
            *before = CowStr::from(format!("{before}{after}"));
            continue;
        }
        merged.push(event);
    }
    merged
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
/// [`strip_int_formats`](super::strip_int_formats), but keyed: only the
/// named properties widen.
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
