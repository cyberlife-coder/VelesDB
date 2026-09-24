//! The guard's own proof: each form rustdoc links is flagged, and what it
//! does not link passes. The tables started as velesdb-server's (#2270),
//! whose hand-written scanner this crate replaces (#2330).

use super::{rustdoc_links, strings_with_rustdoc_links};

#[test]
fn flags_each_link_form() {
    for text in [
        "a JSON-encoded [`Point`].",
        "a JSON-encoded [ `Point` ].",
        "see [crate::Point].",
        "see [Vec<u8, A>::new].",
        "see [Vec<u8>].",
        "see [stream_traverse()].",
        "see [vec!].",
        "see [vec!{}].",
        "see [stream_traverse() ].",
        "see [vec! ].",
        "see [&str].",
        "see [*const].",
        "see [&mut].",
        "see [*mut].",
        "> see [\n> &str] here",
        "see [crate::Point](https://docs.rs/velesdb-core x).",
        "> see [\n> `Point`] here",
        "see [fn@stream_traverse].",
        "see [Point#fields].",
        "see [the point][Point].",
        "see [Point][].",
        "[p]: crate::Point",
        "> [p]: crate::Point",
        "- [p]: crate::Point",
        "[the\npoint]: crate::Point",
        "see [the point](crate::Point).",
        "see [the point](< crate::Point >).",
        // Forms velesdb-server's scanner missed: an autolink to a path, and a
        // bare item path, which rustdoc 1.90 resolves (`[u64]` to the
        // primitive) or warns about.
        "see <crate::Point>.",
        "see [SegmentInfo] and [optional].",
        "see [u64] and [Self].",
        // Forms the guard missed until #2330: rustdoc drops a label's
        // backticks, links `&` to the reference primitive, and reads
        // `mailto::X` as a path, not an address.
        "see [stream_traverse`()`].",
        "see [vec`!`].",
        "see [&].",
        "see [x](mailto::X).",
        // rustdoc trims the spaces around a disambiguator and a call or macro
        // suffix before it resolves the path, and links the bare never and
        // unit primitives.
        "see [foo ()] end",
        "see [vec !].",
        "see [vec !()].",
        "see [struct @Foo].",
        "see [fn @ f].",
        "see [!].",
        "see [!][].",
        "see [()].",
        // `[a, b][]` reads as a collapsed reference only once every reference
        // is accepted; a client renders the inline link after it.
        "see [a, b][](crate::Foo) end",
    ] {
        assert!(!rustdoc_links(text).is_empty(), "the guard misses {text:?}");
    }
}

/// A link to a URL is published as a link, whatever its text; code and
/// prose brackets hold no link.
#[test]
fn passes_web_links_code_and_prose_brackets() {
    for text in [
        "see [the guide](https://velesdb.com/docs).",
        "see [the spec](http://example.com/spec).",
        "write to [the team](mailto:team@velesdb.com).",
        "see [the guide](<https://velesdb.com/docs>).",
        "jump to [the top](#top).",
        "see [`Point`](https://docs.rs/velesdb-core).",
        "see [crate::Point](https://docs.rs/velesdb-core).",
        "see [issue #2261](https://x.dev).",
        "see [Try it!](https://x.dev).",
        "write to [ops@x.dev](mailto:ops@x.dev).",
        "write to <ops@x.dev>, a mail autolink.",
        "the syntax `[x](crate::y)` is code.",
        "a `&[Vec<f32>]` slice",
        "`MATCH (a)-[*1..5]->(b)` and `$.items[*]`",
        "`Collection::delete(&[u64])` in one call.",
        "```json\n{\"v\": [0.1, 0.2, 0.3]}\n```",
        "one of [`asc`, `desc`]",
        "weights in [0, 1]: higher wins",
        "see [#2261]",
        "write to [ops@x.dev]",
        "[write to ops@x]",
        "an empty [] pair",
        // Punctuation rustdoc never reads in a path.
        "see [f(x)] here",
        "see [(a)] and [a{}] and [x()y]",
    ] {
        assert_eq!(rustdoc_links(text), Vec::<String>::new(), "{text:?}");
    }
}

/// What failing closed costs: prose that reads as an item path fails, and
/// is written as code instead.
#[test]
fn flags_prose_that_reads_as_an_item_path() {
    for text in [
        "a bare [Point] reads like [sic].",
        "m[i][j] indexes",
        "MATCH (a:Person)-[:KNOWS]->(b)",
        // rustdoc ignores the spaced unit; the guard trims first and flags it.
        "see [ () ] here",
    ] {
        assert!(!rustdoc_links(text).is_empty(), "{text:?}");
    }
}

#[test]
fn reads_every_string_under_the_given_keys_at_escaped_pointers() {
    let doc = serde_json::json!({
        "info": { "title": "[`Title`]", "description": "plain" },
        "tags": [{ "name": "t", "description": "[`Tag`]" }],
        "paths": { "/x~y": { "get": {
            "summary": "[`Summary`]",
            "responses": { "200": { "description": "[`Response`]" } }
        } } },
        "components": { "schemas": { "S": { "properties": {
            "description": { "type": "string", "description": "[`Property`]" }
        } } } }
    });
    let mut linked = strings_with_rustdoc_links(&doc, &["description", "summary"]);
    linked.sort();
    assert_eq!(
        linked,
        [
            "/components/schemas/S/properties/description/description",
            "/paths/~1x~0y/get/responses/200/description",
            "/paths/~1x~0y/get/summary",
            "/tags/0/description",
        ]
    );
    let descriptions_only = strings_with_rustdoc_links(&doc, &["description"]);
    assert!(
        !descriptions_only.contains(&"/paths/~1x~0y/get/summary".to_owned()),
        "a key not asked for is not read: {descriptions_only:?}"
    );
}
