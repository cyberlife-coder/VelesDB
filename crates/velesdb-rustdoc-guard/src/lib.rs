//! The guard on rustdoc link syntax in published descriptions (#2261, #2330).
//!
//! A doc comment becomes a published description twice in this workspace:
//! velesdb-memory's MCP tool schemas and velesdb-server's `OpenAPI` document.
//! A rustdoc link in either reaches a client as raw syntax, so both check
//! what they publish with this one guard, as a dev-dependency. It is never
//! published: velesdb-core and the release see nothing of it.
//!
//! It reads each text with pulldown-cmark, as rustdoc does, and decides on
//! its own. Its broken-link callback accepts every reference, so it sees each
//! bracket Markdown could read as a link, and it flags
//! - a link, an autolink or an image to anything but an `http`, `https` or
//!   `mailto` URL or a fragment (`[x](crate::y)`, `[x](../y.html)`,
//!   `<crate::y>`, `![x](crate::y)`), whatever its text;
//! - a reference-style link no definition resolves (`[x][y]`);
//! - a shortcut or collapsed link no definition resolves whose label reads as
//!   an item path, which rustdoc 1.90 treats as an intra-doc link and warns
//!   about when it does not resolve (`` [`X`] ``, `[X]`, `[optional]`,
//!   `[a::B]`, `[fn@f]`, `[f()]`, `[X#method.id]`, `[X][]`);
//! - a reference definition to anything but such a URL (`[x]: crate::y`).
//!
//! Code spans, code blocks and escaped brackets hold no link, so it passes
//! them, and it passes prose brackets that name no item (`[0, 1]`, `[a b]`).
//! velesdb-memory's `one_pass_is_final` checks that it flags every text of a
//! pseudo-random mix that memory's rewrite rewrites.

use std::collections::BTreeSet;
use std::ops::Range;

use pulldown_cmark::{BrokenLink, CowStr, Event, LinkType, Options, Parser, Tag};
use serde_json::Value;

/// The Markdown extensions the guard parses with: those a client renderer
/// commonly reads.
const GUARD_MARKDOWN: Options = Options::ENABLE_TABLES
    .union(Options::ENABLE_FOOTNOTES)
    .union(Options::ENABLE_STRIKETHROUGH)
    .union(Options::ENABLE_TASKLISTS);

/// The destinations a published link may have.
const PUBLISHABLE_TARGETS: [&str; 4] = ["http://", "https://", "mailto:", "#"];

/// The rustdoc links `text` holds, each as its source, in text order.
///
/// It reads `text` twice. As rustdoc does, with every reference accepted, so
/// each bracket Markdown could read as a link is seen; and as a client
/// renders it, with none invented, so a link the first reading folds into a
/// reference (`[a, b][](crate::y)`, where `[a, b][]` hides the inline link
/// `[](crate::y)` a client renders) is seen too. What either flags is a link.
#[must_use]
pub fn rustdoc_links(text: &str) -> Vec<String> {
    let as_rustdoc =
        Parser::new_with_broken_link_callback(text, GUARD_MARKDOWN, Some(accept_every_reference))
            .into_offset_iter();
    let mut spans: BTreeSet<(usize, usize)> = as_rustdoc
        .reference_definitions()
        .iter()
        .filter(|(_, definition)| !is_publishable(&definition.dest))
        .map(|(_, definition)| (definition.span.start, definition.span.end))
        .collect();
    collect_links(as_rustdoc, &mut spans);
    // No reference is left unresolved here, so each link is judged by its
    // destination alone, as `is_rustdoc_link` judges a resolved one.
    collect_links(
        Parser::new_ext(text, GUARD_MARKDOWN).into_offset_iter(),
        &mut spans,
    );
    spans
        .into_iter()
        .map(|(start, end)| text[start..end].to_owned())
        .collect()
}

/// Adds to `spans` each rustdoc link of `events` ([`is_rustdoc_link`]), and
/// each image to anything but a URL: rustdoc resolves no intra-doc link in an
/// image, but an item path there is still no address a client can load.
fn collect_links<'a>(
    events: impl Iterator<Item = (Event<'a>, Range<usize>)>,
    spans: &mut BTreeSet<(usize, usize)>,
) {
    for (event, range) in events {
        let flagged = match event {
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                ..
            }) => is_rustdoc_link(link_type, &dest_url),
            Event::Start(Tag::Image { dest_url, .. }) => !is_publishable(&dest_url),
            _ => false,
        };
        if flagged {
            spans.insert((range.start, range.end));
        }
    }
}

#[expect(
    clippy::unnecessary_wraps,
    reason = "pulldown-cmark takes the callback's Option: accepting every reference is the point"
)]
fn accept_every_reference(link: BrokenLink<'_>) -> Option<(CowStr<'_>, CowStr<'_>)> {
    Some((link.reference, CowStr::Borrowed("")))
}

fn is_rustdoc_link(link_type: LinkType, destination: &str) -> bool {
    match link_type {
        LinkType::Inline
        | LinkType::Reference
        | LinkType::Collapsed
        | LinkType::Shortcut
        | LinkType::Autolink => !is_publishable(destination),
        LinkType::ReferenceUnknown => true,
        LinkType::ShortcutUnknown | LinkType::CollapsedUnknown => names_an_item(destination),
        _ => false,
    }
}

/// Whether an unresolved shortcut or collapsed label reads as an item path,
/// which rustdoc 1.90 treats as an intra-doc link. It takes rustdoc's
/// `preprocess_link` steps in rustdoc's order: a label holding a `/` is
/// never a path (`[a#b/c]`); otherwise the label's backticks are dropped, so a code span reads as its code, and the item is what comes
/// before a `#` fragment. A one-word kind before an `@` comes off (`fn@f`),
/// then a call or macro suffix (`()`, `!`, `!()`, `!{}`, `![]`) that leaves
/// something, each with the spaces around it. What is left must hold only
/// letters, digits and the marks rustdoc's `should_ignore_link` keeps
/// (`:_<>, !*&;`), inside its generics too (`[Result<(), u8>]` is prose).
/// Its generics and the empty `::` segments they leave come off, and it is a
/// path when it holds no space and a letter or an underscore, or is the bare
/// `!` or `&` of a primitive (`[!]`, `[!<u8>]`, `[::<u8>&]`). Any other label
/// is never linked: a space outside generics (`[0, 1]`), a digit alone
/// (`[0]`), other punctuation (`[YYYY-MM-DD]`, `[ops@x.dev]`, `[f(x)]`,
/// `[()]`, `[!{}]`).
///
/// Where rustdoc warns and shows the brackets as written, the guard fails
/// closed: it flags a word before an `@` that rustdoc does not take for a
/// kind (`[struct @Foo]`), a call suffix on a primitive (`[!()]`), generics
/// rustdoc finds malformed (`[Vec<<u8>]`), and a path that resolves to
/// nothing (`[sic]`). It passes the labels rustdoc warns about but cannot
/// read as an item (`[write to ops@x]`, `[*]`, `[0]`, `[<T>]`), which show
/// as plain brackets too.
fn names_an_item(label: &str) -> bool {
    // rustdoc reads a `/` anywhere as a relative link, never an item.
    if label.contains('/') {
        return false;
    }
    // rustdoc drops every backtick before it reads the path (`[f`()`]`).
    let label = label.trim().replace('`', "");
    // rustdoc resolves the item before a `#` fragment (`[X#method.id]`).
    let item = label
        .split_once('#')
        .map_or(label.as_str(), |(item, _)| item);
    let path = without_call_suffix(without_disambiguator(item).trim());
    if !path
        .chars()
        .all(|c| c.is_alphanumeric() || ":_<>, !*&;".contains(c))
    {
        return false;
    }
    let path = without_generics(path);
    !path.contains(' ')
        && (path.chars().any(|c| c.is_alphabetic() || c == '_') || path == "!" || path == "&")
}

/// The suffixes rustdoc strips from a path before resolving it, longest
/// first: a macro's `!()`, `!{}` or `![]`, a function's `()`, a macro's `!`.
const CALL_SUFFIXES: [&str; 5] = ["!()", "!{}", "![]", "()", "!"];

/// `path` less the first suffix whose removal leaves something, and the
/// spaces before it. Like rustdoc, it keeps `[!]` whole, the never primitive,
/// and reads `[!()]` as `!` with a function's suffix.
fn without_call_suffix(path: &str) -> &str {
    CALL_SUFFIXES
        .iter()
        .find_map(|suffix| path.strip_suffix(suffix).filter(|rest| !rest.is_empty()))
        .map_or(path, str::trim)
}

/// `path` less its `<…>` groups and the empty `::` segments they leave, as
/// rustdoc's `strip_generics_from_path` returns it (`::<u8>&` gives `&`). A
/// path without generics is kept whole. Like rustdoc, it counts the depth
/// with a sign, so a `>` before any `<` takes it below zero (`[><f]` is `f`),
/// and it strips generics rustdoc finds unbalanced all the same, where
/// rustdoc warns instead.
fn without_generics(path: &str) -> String {
    if !path.contains(['<', '>']) {
        return path.to_owned();
    }
    let mut depth = 0_isize;
    let outside: String = path
        .chars()
        .filter(|&c| {
            match c {
                '<' => depth += 1,
                '>' => depth -= 1,
                _ => return depth == 0,
            }
            false
        })
        .collect();
    outside
        .split("::")
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("::")
}

/// `item` less a disambiguator: a one-word kind before an `@`, with the
/// spaces around it (`fn@f`, `struct @ Foo`). rustdoc links a known kind
/// written flush against its `@`, trims the path after it, and warns about
/// any other kind, spaced ones included; the guard checks no kind, and flags
/// them all. A label whose text before the `@` is not one word
/// (`[write to ops@x]`) keeps its `@`, and reads as prose.
fn without_disambiguator(item: &str) -> &str {
    item.split_once('@')
        .filter(|(kind, _)| {
            let kind = kind.trim();
            !kind.is_empty() && kind.chars().all(|c| c.is_alphanumeric() || c == '_')
        })
        .map_or(item, |(_, path)| path)
}

/// Whether `destination` is a URL or a fragment. A `mailto:` followed by a
/// second `:` is a path rustdoc resolves (`mailto::X`), not an address.
fn is_publishable(destination: &str) -> bool {
    PUBLISHABLE_TARGETS
        .iter()
        .any(|prefix| destination.starts_with(prefix))
        && !destination.starts_with("mailto::")
}

/// The JSON pointer (RFC 6901) of every string under `value`, at any depth,
/// held by one of `keys` and holding a rustdoc link.
#[must_use]
pub fn strings_with_rustdoc_links(value: &Value, keys: &[&str]) -> Vec<String> {
    let mut found = Vec::new();
    collect(value, keys, "", &mut found);
    found
}

fn collect(value: &Value, keys: &[&str], pointer: &str, found: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let here = format!("{pointer}/{}", key.replace('~', "~0").replace('/', "~1"));
                match child {
                    Value::String(text) if keys.contains(&key.as_str()) => {
                        if !rustdoc_links(text).is_empty() {
                            found.push(here);
                        }
                    }
                    _ => collect(child, keys, &here, found),
                }
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                collect(item, keys, &format!("{pointer}/{index}"), found);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod lib_tests;
