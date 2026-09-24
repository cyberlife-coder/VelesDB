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

/// The rustdoc links `text` holds, each as its source.
#[must_use]
pub fn rustdoc_links(text: &str) -> Vec<String> {
    let parser =
        Parser::new_with_broken_link_callback(text, GUARD_MARKDOWN, Some(accept_every_reference))
            .into_offset_iter();
    let mut found: Vec<String> = parser
        .reference_definitions()
        .iter()
        .filter(|(_, definition)| !is_publishable(&definition.dest))
        .map(|(_, definition)| text[definition.span.clone()].to_owned())
        .collect();
    for (event, range) in parser {
        let flagged = match event {
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                ..
            }) => is_rustdoc_link(link_type, &dest_url),
            // rustdoc resolves no intra-doc link in an image, but an item
            // path there is still no address a client can load.
            Event::Start(Tag::Image { dest_url, .. }) => !is_publishable(&dest_url),
            _ => false,
        };
        if flagged {
            found.push(text[range].to_owned());
        }
    }
    found
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
/// which rustdoc 1.90 treats as an intra-doc link: one code span, or, its
/// backticks dropped and before any `#` fragment, a word of letters, digits
/// and path marks (`::`, `@`, `()`, `!`, `<…>`, `&`, `*`) holding a letter or
/// an underscore, or the bare `&` of the reference primitive. A label with a
/// space outside `<…>` (`[0, 1]`), a digit alone (`[0]`) or other punctuation
/// (`[YYYY-MM-DD]`) is prose.
fn names_an_item(label: &str) -> bool {
    let label = label.trim();
    if is_one_code_span(label) {
        return true;
    }
    // rustdoc drops every backtick before it reads the path (`[f`()`]`).
    let label = label.replace('`', "");
    let label = label.trim();
    // rustdoc resolves the item before a `#` fragment (`[X#method.id]`).
    let label = label.split_once('#').map_or(label, |(item, _)| item);
    let mut depth = 0_usize;
    let mut after_disambiguator = false;
    let outside_generics: String = label
        .chars()
        .filter(|&c| {
            // rustdoc trims the path after a disambiguator (`[struct@ Foo]`).
            let skip = after_disambiguator && c.is_whitespace();
            after_disambiguator = c == '@' || skip;
            if skip {
                return false;
            }
            match c {
                '<' => depth += 1,
                '>' => depth = depth.saturating_sub(1),
                _ => return depth == 0,
            }
            false
        })
        .collect();
    let path_like = outside_generics
        .chars()
        .any(|c| c.is_alphabetic() || c == '_')
        && outside_generics
            .chars()
            .all(|c| c.is_alphanumeric() || "_:@!(){}&*;".contains(c));
    path_like || outside_generics == "&"
}

/// Whether `label` is code and nothing else (`` `X` ``, ``` ``a`b`` ```), not
/// a list of code (`` `asc`, `desc` ``), which rustdoc reads as no path.
fn is_one_code_span(label: &str) -> bool {
    let fence = label.len() - label.trim_start_matches('`').len();
    fence > 0
        && label.len() > 2 * fence
        && label.ends_with(&label[..fence])
        && !label[fence..label.len() - fence].contains(&label[..fence])
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
