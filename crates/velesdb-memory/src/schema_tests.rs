//! Tests for [`strip_int_formats`](super::strip_int_formats) and for the
//! input-only [`scalarize_slot_types`](super::scalarize_slot_types) pass.

use schemars::{schema_for, JsonSchema};
use serde_json::json;

use super::strip_int_formats;

#[test]
fn removes_rust_int_formats_but_keeps_standard_ones() {
    let mut schema: schemars::Schema = serde_json::from_value(json!({
        "type": "object",
        "properties": {
            "id": { "type": "integer", "format": "uint64", "minimum": 0 },
            "ids": { "type": "array", "items": { "type": "integer", "format": "uint" } },
            "when": { "type": "string", "format": "date-time" }
        }
    }))
    .expect("valid schema");

    strip_int_formats(&mut schema);

    let value = serde_json::to_value(&schema).expect("serializable");
    assert!(value["properties"]["id"].get("format").is_none());
    assert!(value["properties"]["ids"]["items"].get("format").is_none());
    // The integer constraint survives; only the non-standard format is dropped.
    assert_eq!(value["properties"]["id"]["type"], "integer");
    assert_eq!(value["properties"]["id"]["minimum"], 0);
    // Standard formats are preserved.
    assert_eq!(value["properties"]["when"]["format"], "date-time");
}

#[derive(JsonSchema)]
#[schemars(transform = strip_int_formats)]
#[allow(dead_code)]
struct Sample {
    id: u64,
    hop: usize,
}

#[test]
fn derived_schema_has_no_int_format() {
    let schema = schema_for!(Sample);
    let text = serde_json::to_string(&schema).expect("serializable");
    assert!(
        !text.contains("\"format\""),
        "derived schema still carries an int format: {text}"
    );
}

// --- scalarize_slot_types (l'entree, et l'entree seulement) ------------------

/// Fait passer un schema d'ENTREE par la scalarisation seule (sans inlining
/// ni stringification d'id), pour observer chaque regle isolement.
#[cfg(feature = "mcp")]
fn scalarized(schema: serde_json::Value) -> serde_json::Value {
    let serde_json::Value::Object(map) = schema else {
        panic!("test: le schema d'entree est un objet JSON");
    };
    let mut wire = super::WireInputSchema::adopt(map);
    super::scalarize_slot_types(&mut wire);
    serde_json::Value::Object(wire.0)
}

/// (a) `anyOf: [T, null]` → `T`, et les mots-cles freres du slot ecrasent
/// ceux de la branche promue.
#[test]
#[cfg(feature = "mcp")]
fn collapses_a_nullable_union_into_its_typed_branch() {
    let out = scalarized(json!({
        "type": "object",
        "properties": {
            "media": {
                "description": "la description du slot gagne",
                "default": null,
                "anyOf": [
                    {
                        "type": "object",
                        "description": "celle de la branche perd",
                        "properties": {"data": {"type": "string"}},
                        "required": ["data"]
                    },
                    {"type": "null"}
                ]
            }
        }
    }));

    let media = &out["properties"]["media"];
    assert_eq!(media["type"], json!("object"));
    assert_eq!(media["description"], json!("la description du slot gagne"));
    assert_eq!(media["required"], json!(["data"]));
    assert!(media.get("anyOf").is_none(), "l'union a disparu: {media}");
    assert!(
        media.get("default").is_none(),
        "(d) un `default: null` sur un slot devenu non-nullable est retire: {media}"
    );
}

/// LE CAS NON-COLLAPSABLE : une branche sans `type` direct — un `$ref` que
/// le garde de cycle de l'inliner a laisse en place — ne doit PAS etre
/// promue. La promouvoir ferait du slot un « intypable » orphelin, c'est-a-
/// dire exactement le defaut que la passe repare.
#[test]
#[cfg(feature = "mcp")]
fn keeps_a_nullable_union_whose_branch_carries_no_direct_type() {
    let schema = json!({
        "type": "object",
        "properties": {
            "source": {
                "anyOf": [{"$ref": "#/$defs/SourceReference"}, {"type": "null"}]
            }
        }
    });

    let out = scalarized(schema.clone());

    assert_eq!(
        out["properties"]["source"], schema["properties"]["source"],
        "une branche non typee reste intacte"
    );
}

/// (b) `"type": ["X", "null"]` → `"X"`, les contraintes numeriques survivant
/// intactes.
#[test]
#[cfg(feature = "mcp")]
fn collapses_a_nullable_type_list() {
    let out = scalarized(json!({
        "type": "object",
        "properties": {
            "priority": {"type": ["integer", "null"], "minimum": 0, "maximum": 255, "default": null}
        }
    }));

    let priority = &out["properties"]["priority"];
    assert_eq!(priority["type"], json!("integer"));
    assert_eq!(priority["minimum"], json!(0));
    assert_eq!(priority["maximum"], json!(255));
    assert!(priority.get("default").is_none(), "{priority}");
}

/// (c) Un `oneOf` de `const` devient un `enum` unique, les descriptions de
/// branches repliees dans celle du slot — sans quoi l'effondrement perdrait
/// ce que chaque valeur signifie (`recall_where.filters[].op`).
#[test]
#[cfg(feature = "mcp")]
fn collapses_a_const_union_into_an_enum_and_folds_the_branch_descriptions() {
    let out = scalarized(json!({
        "type": "object",
        "properties": {
            "op": {
                "description": "Comparison operator.",
                "oneOf": [
                    {"const": "eq", "description": "`=`", "type": "string"},
                    {"const": "lt", "description": "`<`", "type": "string"}
                ]
            }
        }
    }));

    let op = &out["properties"]["op"];
    assert_eq!(op["type"], json!("string"));
    assert_eq!(op["enum"], json!(["eq", "lt"]));
    assert!(op.get("oneOf").is_none(), "l'union a disparu: {op}");
    let description = op["description"].as_str().expect("une description");
    assert!(
        description.starts_with("Comparison operator."),
        "{description}"
    );
    assert!(description.contains("\"eq\": `=`"), "{description}");
    assert!(description.contains("\"lt\": `<`"), "{description}");
}

/// Les deux regles se composent : `Option<Enum>` arrive en
/// `anyOf: [{oneOf: [const…]}, null]`, dont la premiere branche ne porte
/// aucun `type` direct. C'est la descente enfants-d'abord qui la rend
/// promouvable.
#[test]
#[cfg(feature = "mcp")]
fn collapses_a_nullable_const_union_through_its_branch() {
    let out = scalarized(json!({
        "type": "object",
        "properties": {
            "format": {
                "anyOf": [
                    {"oneOf": [{"const": "plain", "type": "string"}, {"const": "jsonl", "type": "string"}]},
                    {"type": "null"}
                ]
            }
        }
    }));

    let format = &out["properties"]["format"];
    assert_eq!(format["type"], json!("string"));
    assert_eq!(format["enum"], json!(["plain", "jsonl"]));
}

/// La marche d'arbre couvre les memes chemins que l'inliner : `items`
/// simple, `items` tuple, et `additionalProperties`.
#[test]
#[cfg(feature = "mcp")]
fn walks_items_tuple_items_and_additional_properties() {
    let out = scalarized(json!({
        "type": "object",
        "properties": {
            "facts": {"type": "array", "items": {"type": "object", "properties": {
                "text": {"type": ["string", "null"]}
            }}},
            "pair": {"type": "array", "items": [
                {"type": ["integer", "null"]},
                {"type": ["boolean", "null"]}
            ]},
            "models": {"type": "object", "additionalProperties": {
                "anyOf": [{"type": "object", "properties": {}}, {"type": "null"}]
            }}
        }
    }));

    assert_eq!(
        out["properties"]["facts"]["items"]["properties"]["text"]["type"],
        json!("string")
    );
    assert_eq!(
        out["properties"]["pair"]["items"][0]["type"],
        json!("integer")
    );
    assert_eq!(
        out["properties"]["pair"]["items"][1]["type"],
        json!("boolean")
    );
    assert_eq!(
        out["properties"]["models"]["additionalProperties"]["type"],
        json!("object")
    );
}

/// Une liste de formes qui n'est pas `[X, null]` reste intacte :
/// `recall_where.filters[].value` est polymorphe par conception, et
/// l'effondrer arbitrairement mentirait sur ce que le serveur compare.
#[test]
#[cfg(feature = "mcp")]
fn leaves_a_genuinely_polymorphic_type_list_alone() {
    let out = scalarized(json!({
        "type": "object",
        "properties": {
            "value": {"type": ["number", "string", "boolean"]}
        }
    }));

    assert_eq!(
        out["properties"]["value"]["type"],
        json!(["number", "string", "boolean"])
    );
}

// --- La SORTIE, et la regle qui ne doit jamais l'atteindre -------------------

#[cfg(feature = "mcp")]
#[derive(schemars::JsonSchema)]
#[allow(dead_code)]
struct NestedOut {
    label: String,
}

#[cfg(feature = "mcp")]
#[derive(schemars::JsonSchema)]
#[allow(dead_code)]
struct OptionalOut {
    /// Ce que `load_working_context.working` et
    /// `retrieve_context_source.media` sont : une valeur legitimement absente.
    nested: Option<NestedOut>,
    /// Et la forme d'id que `widen_id_properties` produit en sortie.
    memory_id: Option<u64>,
}

/// Le durcissement de SORTIE doit garder ce qu'un `null` legitime exige.
///
/// C'est le garde de la regle que la separation par types documente sans
/// pouvoir la faire respecter a l'interieur de `schema.rs` : `scalarize_in_map`
/// y est typee sur la carte brute, donc ajouter `scalarize_in_map(&mut self.0)`
/// a [`super::WireOutputSchema::harden`] compile. Rien, jusqu'ici, ne
/// l'aurait vu : la regle de sortie de `tests/mcp_schema_bdd.rs`
/// (`announces_some_type`) est satisfaite aussi bien par `T` que par
/// `anyOf: [T, null]`.
///
/// Ce qui casserait alors n'est pas un schema, c'est une REPONSE : les SDK
/// MCP valident `structuredContent` contre l'`outputSchema` annonce (spec
/// 2025-06-18), donc un `working: null` — la reponse documentee d'une session
/// jamais sauvegardee — serait rejete chez le client.
#[test]
#[cfg(feature = "mcp")]
fn output_hardening_keeps_a_nullable_union_intact() {
    let published = super::wire_safe_output_schema::<OptionalOut>();
    let schema = serde_json::to_value(&*published).expect("le schema se serialise");

    let nested = &schema["properties"]["nested"];
    assert!(
        admits_null(nested),
        "la sortie doit continuer d'admettre `null` sur un champ optionnel, got {nested}"
    );
    let memory_id = &schema["properties"]["memory_id"];
    assert!(
        admits_null(memory_id),
        "idem pour un id optionnel, got {memory_id}"
    );
}

/// `null` est-il une valeur admise par ce slot — directement, dans une liste
/// de formes, ou par une branche d'union ?
#[cfg(feature = "mcp")]
fn admits_null(slot: &serde_json::Value) -> bool {
    match slot.get("type") {
        Some(serde_json::Value::String(kind)) => kind == "null",
        Some(serde_json::Value::Array(names)) => names.iter().any(|name| name == "null"),
        _ => ["anyOf", "oneOf"].iter().any(|keyword| {
            matches!(slot.get(*keyword), Some(serde_json::Value::Array(branches))
                if branches.iter().any(admits_null))
        }),
    }
}

/// The rustdoc-link rewrite applied to every published description (#2261).
#[cfg(feature = "mcp")]
mod unlink {
    use super::super::walks::{unlink_rustdoc, unlink_rustdoc_descriptions};
    use serde_json::{json, Value};

    use velesdb_rustdoc_guard::{rustdoc_links, strings_with_rustdoc_links};

    /// Asserts `text` is rewritten to `shown`, and that the guard flags the
    /// text and passes what it becomes.
    fn assert_rewritten(text: &str, shown: &str) {
        assert_eq!(unlink_rustdoc(text).as_deref(), Some(shown), "{text:?}");
        assert!(!rustdoc_links(text).is_empty(), "the guard misses {text:?}");
        assert_eq!(rustdoc_links(shown), Vec::<String>::new(), "{shown:?}");
    }

    /// Asserts `text` is left as written and the guard flags it: a text the
    /// rewrite refuses cannot be published unnoticed.
    fn assert_refused(text: &str) {
        assert_eq!(unlink_rustdoc(text), None, "{text:?}");
        assert!(!rustdoc_links(text).is_empty(), "the guard misses {text:?}");
    }

    /// rustdoc warns about a kind spaced from its `@` and shows the
    /// brackets; the rewrite drops them and the guard flags them all the same
    /// (#2330). The pseudo-random mix of `one_pass_is_final` cannot draw a
    /// known kind before a spaced `@`, so these are named here.
    #[test]
    fn a_spaced_disambiguator_is_rewritten_and_flagged() {
        assert_rewritten("see [struct @Foo].", "see Foo.");
        assert_rewritten("see [fn @ f].", "see f.");
    }

    #[test]
    fn a_code_link_shows_its_code_span() {
        for (text, shown) in [
            (
                "see [`A::b`] and [`c`](crate::d::c).",
                "see `A::b` and `c`.",
            ),
            (
                "the [`crate::Recollection`] it returns",
                "the `crate::Recollection` it returns",
            ),
            (
                "the [`Recollection`][] it returns",
                "the `Recollection` it returns",
            ),
            ("a [`call`](f()) here", "a `call` here"),
            ("see [`x`](m!()) and [`y`](m!)", "see `x` and `y`"),
            ("see [`x`]( crate::y )", "see `x`"),
            ("see [`x`](crate::y \"title\")", "see `x`"),
            (
                "the [`stable id`](super::fragment_id) of a fragment",
                "the `stable id` of a fragment",
            ),
            ("a [`builder`](fn@crate::build) call", "a `builder` call"),
            ("call [`build()`] first", "call `build()` first"),
            ("the [`vec!`] macro", "the `vec!` macro"),
            (
                "the [`HashMap<K, V>`] it holds",
                "the `HashMap<K, V>` it holds",
            ),
            ("see [`&str`] and [`*const`]", "see `&str` and `*const`"),
            ("see [``a`b``](crate::y) here", "see ``a`b`` here"),
            ("see [`a[`](crate::y) here", "see `a[` here"),
            ("in [0, 1) see [`x`](crate::y)", "in [0, 1) see `x`"),
            ("`vec![]` and [`X`]", "`vec![]` and `X`"),
            ("Title\n---\nsee [`A`]", "Title\n---\nsee `A`"),
            ("> quoted [`A::b`]\n> here", "> quoted `A::b`\n> here"),
            ("- item [`A::b`]\n- next", "- item `A::b`\n- next"),
        ] {
            assert_rewritten(text, shown);
        }
    }

    /// rustdoc 1.90 drops each of these kinds from the text it shows for a
    /// shortcut code link, and so does the rewrite. It knows no `tyalias@` or
    /// `typealias@`: a link with one is no rustdoc link, stays as written, and
    /// the guard flags it.
    #[test]
    fn every_disambiguator_rustdoc_accepts_is_dropped() {
        for kind in [
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
        ] {
            assert_rewritten(&format!("see [`{kind}@X`]"), "see `X`");
        }
        for kind in ["tyalias", "typealias"] {
            assert_refused(&format!("see [`{kind}@X`]"));
        }
    }

    /// A link whose text is prose shows that prose, and a link to a definition
    /// loses the definition with it.
    #[test]
    fn a_prose_link_and_a_reference_style_link_show_their_text() {
        for (text, shown) in [
            ("see [the point](crate::Point) here", "see the point here"),
            ("see [the point][crate::Point] here", "see the point here"),
            ("see [crate::Point] here", "see crate::Point here"),
            ("see [fn@stream] here", "see stream here"),
            ("see [method@Foo::bar] here", "see Foo::bar here"),
            ("see [struct@ Foo][] here", "see Foo here"),
            ("see [stream()] here", "see stream() here"),
            ("see [c](crate::Foo#method.bar) here", "see c here"),
            (
                "see *[the point](crate::Point)* here",
                "see *the point* here",
            ),
            (
                "see [`Recollection`][rec].\n\n[rec]: crate::Recollection\n",
                "see `Recollection`.\n\n",
            ),
            (
                "see [the recollection][rec].\n\n[rec]: crate::Recollection\n",
                "see the recollection.\n\n",
            ),
            (
                "> see [z].\n>\n> [z]: crate::Z\n> more",
                "> see z.\n>\n> more",
            ),
        ] {
            assert_rewritten(text, shown);
        }
    }

    /// rustdoc 1.90 reads every bracketed item path as an intra-doc link and
    /// warns when it does not resolve (`[optional]`, `map[key]`: "unresolved
    /// link to `optional`"), so a doc comment CI builds with `-D warnings`
    /// holds one only when it resolves. The rewrite and the guard read a bare
    /// `[Name]` the same way.
    #[test]
    fn a_bare_item_path_is_a_rustdoc_link() {
        for (text, shown) in [
            (
                "see [SegmentInfo] and [struct@SegmentInfo].",
                "see SegmentInfo and SegmentInfo.",
            ),
            (
                "see [a::B] and [Recollection][]",
                "see a::B and Recollection",
            ),
            ("this is [optional] here", "this is optional here"),
            (
                "see [Recollection#method.id] and [fn@f#x][]",
                "see Recollection and f",
            ),
            (
                "see [`Recollection#method.id`] here",
                "see `Recollection` here",
            ),
            ("see [`fn@f#x`] here", "see `f` here"),
        ] {
            assert_rewritten(text, shown);
        }
    }

    /// A definition is removed with as much of its line as leaves the rest
    /// reading the same: its whole line in a quote that holds more, only the
    /// definition where the quote would otherwise go. rustdoc accepts each.
    #[test]
    fn a_definition_in_a_block_quote_goes_and_the_quote_stays() {
        for (text, shown) in [
            ("> [z]: crate::Z\n\nText [z]", "> \n\nText z"),
            ("Text [z]\n\n> [z]: crate::Z", "Text z\n\n> "),
            (">\t[z]: crate::Z\n> more", "> more"),
            (
                "> see [z].\n>\n> [z]: crate::Z\n> more",
                "> see z.\n>\n> more",
            ),
        ] {
            assert_rewritten(text, shown);
        }
    }

    /// rustdoc resolves no intra-doc link in an image or an autolink, so the
    /// rewrite leaves both, and the guard fails on an item path there.
    #[test]
    fn an_image_or_an_autolink_to_an_item_path_is_left_and_flagged() {
        for text in ["an image ![alt](crate::X) here", "see <crate::X> here"] {
            assert_refused(text);
        }
    }

    /// What rustdoc does not link is published unchanged, and the guard passes
    /// it: web links, a fragment, a mail address, inline code holding brackets
    /// or link syntax, code blocks, escaped brackets, and prose brackets.
    #[test]
    fn what_rustdoc_does_not_link_survives_unchanged() {
        for text in [
            "see [docs](https://example.com/a) and [the spec](http://x.dev/spec)",
            "see [`Point`](https://docs.rs/velesdb-core)",
            "write to [the team](mailto:team@x.dev), jump to [the top](#top)",
            "see <https://x.dev/a[crate::X]b>",
            "the syntax `[x](crate::y)` is code, so is `[`",
            "`decisions[fragment_index]` and ``a [`b`] c``",
            "```rust\nlet w = [`crate::X`];\n```",
            "~~~\n[x](crate::y)\n~~~",
            "Example:\n\n    let w = [`crate::X`];",
            "see \\[`crate::X`\\] here",
            "in [0, 1] and [1, 2] on [YYYY-MM-DD], item [0]",
            "the list [`asc`, `desc`] and [a b]",
            "an image ![logo](https://x.dev/a.png) and <ops@x.dev>",
        ] {
            assert_eq!(unlink_rustdoc(text), None, "{text:?}");
            assert_eq!(rustdoc_links(text), Vec::<String>::new(), "{text:?}");
        }
    }

    /// The rewrite is fail closed: when the text without a link's brackets
    /// would not read as the text with them (a neighbour turns bold, two code
    /// spans merge, a new link forms), the text
    /// stays as written, and the guard fails on it.
    #[test]
    fn a_rewrite_that_would_change_its_neighbours_is_refused() {
        for text in [
            "returns **[Recollection](crate::Recollection)**s",
            "[`a`]`b`",
            "`a`[`b`]",
            "[[`a`]](crate::x)",
        ] {
            assert_refused(text);
        }
    }

    /// A link to what is neither an item path nor a URL the guard lets
    /// through (a relative URL, another scheme) is no rustdoc link, so the
    /// rewrite leaves it. The guard is broader and fails on it: a published
    /// description links only to a web page, a mail address or a fragment.
    #[test]
    fn a_link_to_neither_a_path_nor_a_web_url_is_left_and_flagged() {
        for text in [
            "see [docs](../x.html)",
            "see [x](@foo)",
            "see [y](http:crate)",
            "see [spec](ftp://x.dev)",
        ] {
            assert_refused(text);
        }
    }

    /// One pass is final: a rewritten text holds no link a second pass would
    /// rewrite, checked on a pseudo-random mix of brackets, backticks, colons,
    /// emphasis and links, which both rewrites and refuses.
    #[test]
    fn one_pass_is_final() {
        let tokens = [
            "[",
            "]",
            "`",
            "``",
            "(",
            ")",
            "crate::x",
            "a",
            " ",
            ":",
            "\n",
            "[`X`]",
            "[`Y`](crate::y)",
            "*",
            "**",
            "!",
            "0, 1",
            "[a",
            "b]",
            "]]",
            "[[",
            "[`Z`]]",
            "[see ",
            "](",
            "`b`",
            "_",
            "'",
            "]:",
            "\"",
            "# ",
            "fn@f",
            "\\",
            "    ",
            "- ",
            "> ",
            "[r]: crate::r\n",
            "[r]",
            "][",
            "[Recollection#method.id]",
            "#x",
            ">\t",
            "\n\n",
        ];
        let mut seed: usize = 0x2265_2025;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let (mut rewritten, mut left) = (0_usize, 0_usize);
        for _ in 0..20_000 {
            let len = 1 + next() % 14;
            let text: String = (0..len).map(|_| tokens[next() % tokens.len()]).collect();
            match unlink_rustdoc(&text) {
                Some(out) => {
                    rewritten += 1;
                    assert_eq!(unlink_rustdoc(&out), None, "{text:?} -> {out:?}");
                    // The guard reads at least what the rewrite reads.
                    assert!(
                        !rustdoc_links(&text).is_empty(),
                        "the guard misses {text:?}"
                    );
                }
                None => left += 1,
            }
        }
        assert!(
            rewritten > 0 && left > 0,
            "rewritten {rewritten}, left {left}"
        );
    }

    /// Every `description` of a schema is rewritten, at any depth and under a
    /// property named like a keyword, and instance data is not.
    #[test]
    fn every_description_is_rewritten_and_nothing_else() {
        let mut schema = json!({
            "description": "a [`Top`]",
            "default": { "description": "[`kept`]" },
            "examples": [{ "description": "[`kept`]" }],
            "example": { "description": "[`kept`]" },
            "const": { "description": "[`kept`]" },
            "enum": [{ "description": "[`kept`]" }],
            "properties": {
                "default": { "description": "named [`N`]" },
                "description": { "type": "string", "description": "field [`F`](crate::F)" },
                "tags": {
                    "type": "array",
                    "items": { "description": "item [`I`]" },
                    "default": ["[`not a description`]"]
                }
            },
            "$defs": { "enum": { "description": "def [`D`]" } },
            "anyOf": [{ "description": "any [`A`]" }],
            "oneOf": [{ "description": "one [`O`]" }],
            "additionalProperties": { "description": "extra [`E`]" },
            "patternProperties": { "example": { "description": "pattern [`P`]" } },
            "definitions": { "default": { "description": "old [`G`]" } },
            "dependentSchemas": { "const": { "description": "dep [`S`]" } },
            "dependencies": { "enum": { "description": "deps [`Y`]" } }
        });
        unlink_rustdoc_descriptions(schema.as_object_mut().expect("test: an object"));
        assert_eq!(
            schema,
            json!({
                "description": "a `Top`",
                "default": { "description": "[`kept`]" },
                "examples": [{ "description": "[`kept`]" }],
                "example": { "description": "[`kept`]" },
                "const": { "description": "[`kept`]" },
                "enum": [{ "description": "[`kept`]" }],
                "properties": {
                    "default": { "description": "named `N`" },
                    "description": { "type": "string", "description": "field `F`" },
                    "tags": {
                        "type": "array",
                        "items": { "description": "item `I`" },
                        "default": ["[`not a description`]"]
                    }
                },
                "$defs": { "enum": { "description": "def `D`" } },
                "anyOf": [{ "description": "any `A`" }],
                "oneOf": [{ "description": "one `O`" }],
                "additionalProperties": { "description": "extra `E`" },
                "patternProperties": { "example": { "description": "pattern `P`" } },
                "definitions": { "default": { "description": "old `G`" } },
                "dependentSchemas": { "const": { "description": "dep `S`" } },
                "dependencies": { "enum": { "description": "deps `Y`" } }
            })
        );
    }

    /// The committed capture of what the server publishes, kept equal to the
    /// live schema by `mcp_tools_drift`, holds no rustdoc link in any
    /// description. `mcp_schema_bdd` checks the live schemas themselves.
    #[test]
    fn the_published_tool_schema_carries_no_rustdoc_link() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/reference/mcp-tools.json"
        );
        let text = std::fs::read_to_string(path).expect("test: read the snapshot");
        let snapshot: Value = serde_json::from_str(&text).expect("test: snapshot is JSON");
        let linked = strings_with_rustdoc_links(&snapshot, &["description"]);
        assert!(
            linked.is_empty(),
            "{} published descriptions still carry rustdoc link syntax, e.g. {:?}",
            linked.len(),
            &linked[..linked.len().min(5)]
        );
    }

    /// The guard flags each rustdoc link form on its own, whether or not the
    /// rewrite would rewrite it.
    #[test]
    fn the_guard_flags_each_link_form() {
        for text in [
            "a JSON-encoded [`Point`].",
            "a JSON-encoded [ `Point` ].",
            "see [crate::Point].",
            "see [fn@stream].",
            "see [stream()].",
            "see [vec!].",
            "see [the point][Point].",
            "see [Point][].",
            "[p]: crate::Point",
            "see [the point](crate::Point).",
            "see [the point](<crate::Point>).",
            "> see [\n> `Point`] here",
            "see [Recollection#method.id].",
            "returns **[Recollection#method.id]**s",
            "see [`Vec#method.push`] and [a::B#x][]",
        ] {
            assert!(!rustdoc_links(text).is_empty(), "the guard misses {text:?}");
        }
    }
}
