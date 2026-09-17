// The guard on rustdoc link syntax in published schema descriptions (#2261).
//
// Shared, not copied: `src/schema_tests.rs` includes this file (`include!`)
// to check the rewrite and the committed `docs/reference/mcp-tools.json`, and
// `tests/mcp_schema_bdd.rs` loads it (`#[path]`) to check the live schema of
// every tool the server lists. It sits in a subdirectory so cargo builds no
// test target of its own from it.
//
// It reads each description with pulldown-cmark, as the rewrite in
// `src/schema_walks.rs` does, but decides on its own, and more broadly: its
// broken-link callback accepts every reference, so it sees each bracket
// Markdown could read as a link, and it flags
// - a link to anything but an `http`, `https` or `mailto` URL or a fragment
//   (`[x](crate::y)`, `[x](../y.html)`), whatever its text;
// - a reference-style link or collapsed link no definition resolves (`[x][y]`,
//   `[x][]`);
// - a shortcut link whose label is code, holds `::` or `@`, or ends in `()` or
//   `!` (`` [`X`] ``, `[a::B]`, `[fn@f]`, `[f()]`);
// - a reference definition to anything but such a URL (`[x]: crate::y`).
// Code spans, code blocks and escaped brackets hold no link, so it passes
// them, and it passes a bare `[name]`, which reads as prose.

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
        if let Event::Start(Tag::Link {
            link_type,
            dest_url,
            ..
        }) = event
        {
            if is_rustdoc_link(link_type, &dest_url) {
                found.push(text[range].to_owned());
            }
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
        LinkType::Inline | LinkType::Reference | LinkType::Collapsed | LinkType::Shortcut => {
            !is_publishable(destination)
        }
        LinkType::ReferenceUnknown | LinkType::CollapsedUnknown => true,
        LinkType::ShortcutUnknown => {
            let label = destination.trim();
            is_one_code_span(label)
                || label.contains("::")
                || label.contains('@')
                || label.ends_with("()")
                || label.ends_with('!')
        }
        _ => false,
    }
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

fn is_publishable(destination: &str) -> bool {
    PUBLISHABLE_TARGETS
        .iter()
        .any(|prefix| destination.starts_with(prefix))
}

/// The JSON pointer of every `description` string under `value`, at any
/// depth, that holds a rustdoc link.
pub fn descriptions_with_rustdoc_links(value: &Value) -> Vec<String> {
    let mut found = Vec::new();
    collect(value, "", &mut found);
    found
}

fn collect(value: &Value, path: &str, found: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let here = format!("{path}/{key}");
                match child {
                    Value::String(text) if key == "description" => {
                        if !rustdoc_links(text).is_empty() {
                            found.push(here);
                        }
                    }
                    _ => collect(child, &here, found),
                }
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                collect(item, &format!("{path}/{index}"), found);
            }
        }
        _ => {}
    }
}
