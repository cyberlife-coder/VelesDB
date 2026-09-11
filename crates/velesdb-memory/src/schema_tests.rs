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

    #[test]
    fn a_code_link_keeps_its_code_span() {
        assert_eq!(
            unlink_rustdoc("see [`A::b`] and [`c`](crate::d::c).").as_deref(),
            Some("see `A::b` and `c`.")
        );
    }

    #[test]
    fn an_inline_link_shows_its_label() {
        for (text, shown) in [
            ("a [call](f()) here", "a call here"),
            ("see [x]( crate::y)", "see x"),
            ("in [0, 1) see [x](crate::y)", "in [0, 1) see x"),
        ] {
            assert_eq!(unlink_rustdoc(text).as_deref(), Some(shown), "{text}");
        }
        assert_eq!(
            unlink_rustdoc("the [`stable id`](super::fragment_id) of a fragment").as_deref(),
            Some("the `stable id` of a fragment")
        );
        assert_eq!(
            unlink_rustdoc("a [builder](fn@crate::build) call").as_deref(),
            Some("a builder call")
        );
    }

    #[test]
    fn a_path_like_shortcut_shows_its_path_without_the_disambiguator() {
        for (text, shown) in [
            (
                "the [crate::Recollection] it returns",
                "the crate::Recollection it returns",
            ),
            ("call [build()] first", "call build() first"),
            ("the [vec!] macro", "the vec! macro"),
            ("a [struct@Foo] value", "a Foo value"),
            ("see [`fn@build`]", "see `build`"),
            ("see [`fn@ build`]", "see `build`"),
            ("a [` Foo `] padded", "a `Foo` padded"),
            ("see [m!()]", "see m!()"),
            (
                "the [`HashMap<K, V>`] it holds",
                "the `HashMap<K, V>` it holds",
            ),
        ] {
            assert_eq!(unlink_rustdoc(text).as_deref(), Some(shown), "{text}");
        }
    }

    /// One pass is final. A text a second pass would change further, such as
    /// nested brackets whose outer pair Markdown shows as written, is left as
    /// written, and the guard fails on it.
    #[test]
    fn a_text_a_second_pass_would_change_stays_as_written() {
        let once = unlink_rustdoc("see [`A`]").expect("test: a link to rewrite");
        assert_eq!(unlink_rustdoc(&once), None, "{once}");
        for text in [
            "[[`X`]]",
            "[`a`](crate::a) and [[`b`]]",
            "the [[crate::X]] field",
        ] {
            assert_eq!(unlink_rustdoc(text), None, "{text}");
            assert!(holds_rustdoc_link(text), "the guard misses {text:?}");
        }
    }

    #[test]
    fn rewritten_code_spans_never_touch() {
        assert_eq!(unlink_rustdoc("[`a`]`b`").as_deref(), Some("`a` `b`"));
        assert_eq!(
            unlink_rustdoc("see [`a`](crate::a)[`b`](crate::b)").as_deref(),
            Some("see `a` `b`")
        );
    }

    #[test]
    fn brackets_that_are_not_rustdoc_links_stay() {
        for text in [
            "in [0, 1]",
            "[docs](https://example.com/a)",
            "[`a`][reference]",
            "a [ lone bracket",
            "the [Recollection] it returns",
            "fragments[i] and map[key] lookup",
            "v[idx] = x",
            "[sic] later",
            "[NOTE]: something",
            "[user@example]",
            "[x](@foo)",
            "range [`0, 1`] inclusive",
            "either [`a | b`]",
            "[`value@`]",
            "an [`a<b c`] unbalanced",
            "a [`Vec<T>>`] stray",
            "`decisions[fragment_index]` is unambiguous",
            "[text][crate::X]",
            "``a [`b`] c`` in a double-backtick span",
        ] {
            assert_eq!(unlink_rustdoc(text), None, "{text}");
        }
    }

    #[test]
    fn every_description_is_rewritten_and_nothing_else() {
        let mut schema = json!({
            "description": "a [`Top`]",
            "default": { "description": "[`kept`]" },
            "examples": [{ "description": "[`kept`]" }],
            "const": { "description": "[`kept`]" },
            "properties": {
                "default": { "description": "named [`N`]" },
                "description": { "type": "string", "description": "field [`F`](crate::F)" },
                "tags": {
                    "type": "array",
                    "items": { "description": "item [`I`]" },
                    "default": ["[`not a description`]"]
                }
            },
            "$defs": { "D": { "description": "def [`D`]" } },
            "anyOf": [{ "description": "any [`A`]" }],
            "oneOf": [{ "description": "one [`O`]" }],
            "additionalProperties": { "description": "extra [`E`]" },
            "patternProperties": { "^x-": { "description": "pattern [`P`]" } }
        });
        // The guard's own walk over the same tree: it reads every description
        // the rewrite reads, and none of the instance data it leaves.
        let mut before = Vec::new();
        collect_linked(&schema, "", &mut before);
        before.sort();
        assert_eq!(
            before,
            [
                "/$defs/D/description",
                "/additionalProperties/description",
                "/anyOf/0/description",
                "/description",
                "/oneOf/0/description",
                "/patternProperties/^x-/description",
                "/properties/default/description",
                "/properties/description/description",
                "/properties/tags/items/description",
            ]
        );
        unlink_rustdoc_descriptions(schema.as_object_mut().expect("test: an object"));
        let mut after = Vec::new();
        collect_linked(&schema, "", &mut after);
        assert!(after.is_empty(), "the rewrite left {after:?}");
        assert_eq!(
            schema,
            json!({
                "description": "a `Top`",
                "default": { "description": "[`kept`]" },
                "examples": [{ "description": "[`kept`]" }],
                "const": { "description": "[`kept`]" },
                "properties": {
                    "default": { "description": "named `N`" },
                    "description": { "type": "string", "description": "field `F`" },
                    "tags": {
                        "type": "array",
                        "items": { "description": "item `I`" },
                        "default": ["[`not a description`]"]
                    }
                },
                "$defs": { "D": { "description": "def `D`" } },
                "anyOf": [{ "description": "any `A`" }],
                "oneOf": [{ "description": "one `O`" }],
                "additionalProperties": { "description": "extra `E`" },
                "patternProperties": { "^x-": { "description": "pattern `P`" } }
            })
        );
    }

    /// The committed capture of what the server publishes — kept equal to the
    /// live schema by `mcp_tools_drift` — holds no rustdoc link syntax
    /// ([`holds_rustdoc_link`]).
    #[test]
    fn the_published_tool_schema_carries_no_rustdoc_link() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/reference/mcp-tools.json"
        );
        let text = std::fs::read_to_string(path).expect("test: read the snapshot");
        let snapshot: Value = serde_json::from_str(&text).expect("test: snapshot is JSON");
        let mut linked = Vec::new();
        collect_linked(&snapshot, "", &mut linked);
        assert!(
            linked.is_empty(),
            "{} published descriptions still carry rustdoc link syntax, e.g. {:?}",
            linked.len(),
            &linked[..linked.len().min(5)]
        );
    }

    /// Markdown lets a link span lines, but what a line ending allows there
    /// depends on the next line, and no published doc comment writes one: the
    /// rewrite leaves every such link as written, and the guard flags each,
    /// whatever its target or form.
    #[test]
    fn a_link_that_spans_a_line_stays_as_written() {
        for text in [
            "see [x](\ncrate::y)",
            "see [x](\r\ncrate::y)",
            "see [x](\rcrate::y)",
            "see [x](crate::y\n)",
            "see [x](<\ncrate::y>)",
            "see [x](\n\ncrate::y)",
            "a [b\nc](crate::y) d",
            "see [a\n\nb](crate::y)",
            "see [a\nb](Foo)",
            "see [x](\nfn@f)",
            "see [`HashMap<K,\nV>`] here",
        ] {
            assert_eq!(unlink_rustdoc(text), None, "{text:?}");
            assert!(holds_rustdoc_link(text), "the guard misses {text:?}");
        }
    }

    /// Inside a code span a `[` is code, so an inline link's label may hold one
    /// there. A shortcut code link rustdoc does not resolve, such as
    /// ``[`a[`]`` or ``[`a.b`]``, stays as written, as rustdoc shows it.
    #[test]
    fn a_code_link_may_hold_a_bracket() {
        assert_eq!(
            unlink_rustdoc("see [`a[`](crate::y) here").as_deref(),
            Some("see `a[` here")
        );
        assert_eq!(
            unlink_rustdoc("see [a `[` b](Foo) here").as_deref(),
            Some("see a `[` b here")
        );
        assert_eq!(unlink_rustdoc("see [`a[`] here"), None);
        assert_eq!(unlink_rustdoc("see [`a.b`] here"), None);
        assert_eq!(unlink_rustdoc("see [`()`] here"), None);
    }

    /// rustdoc resolves a link by the path before its `#` fragment, and a
    /// shortcut link shows only that part. The rewrite does not model it: it
    /// leaves such a link as written, and the guard fails on it before it can
    /// reach a client. So it does on a bracketed `@` or `#` it leaves.
    #[test]
    fn a_link_with_a_fragment_stays_and_the_guard_flags_it() {
        for text in [
            "see [`Vec#method.push`] here",
            "see [a::B#x] here",
            "see [x](Vec#method.push) here",
            "see [a#b] here",
            "see [fn@ f] here",
        ] {
            assert_eq!(unlink_rustdoc(text), None, "{text}");
            assert!(holds_rustdoc_link(text), "the guard misses {text:?}");
        }
    }

    /// A reference-style or collapsed link, and a reference definition, whose
    /// target is not a URL: the rewrite leaves them, and the guard fails on
    /// each.
    #[test]
    fn the_guard_flags_reference_style_links() {
        for text in [
            "see [crate::Recollection][] here",
            "see [`crate::Recollection`][] here",
            "see [`Recollection`][rec].\n\n[rec]: crate::Recollection",
            "see [the recollection][rec].\n\n[rec]: crate::Recollection",
        ] {
            assert_eq!(unlink_rustdoc(text), None, "{text:?}");
            assert!(holds_rustdoc_link(text), "the guard misses {text:?}");
        }
    }

    /// A shortcut code link padded inside its brackets, or split across a
    /// line: the rewrite leaves it, and the guard fails on it.
    #[test]
    fn the_guard_flags_a_padded_shortcut_code_link() {
        for text in [
            "see [`crate::Foo` ] here",
            "see [ `fn@f` ] here",
            "see [`crate::Foo`\n] here",
        ] {
            assert_eq!(unlink_rustdoc(text), None, "{text:?}");
            assert!(holds_rustdoc_link(text), "the guard misses {text:?}");
        }
    }

    /// The scan reads inline code spans that close on their own line and the
    /// links around them. It leaves as written a text holding anything else (a
    /// backslash, a tab or four spaces, a fence, a `<` outside code, a table, a
    /// code span across a line), in or out of a quote or a list item, and the
    /// guard fails on the link syntax left in it.
    #[test]
    fn a_text_the_scan_cannot_read_exactly_stays_as_written() {
        for text in [
            "~~~\nlet w = [crate::X];\n~~~",
            "```rust\nlet w = [crate::X];",
            "Example:\n```rust\nlet v = vec![build()];\n\nlet w = [crate::X];\n```",
            "    let w = [crate::X];",
            "\tlet w = [crate::X];",
            "  \tlet w = [crate::X];",
            "<div>\n[crate::X]\n</div>",
            "see \\[crate::X] here",
            "- use ` carefully\n- see [`R`](crate::R) and `x`",
            "- use ` carefully\n- see `[crate::X]` here",
            "Uses a ` here.\n# Errors\nSee [`R`](crate::R) and `x`.",
            "Use a trailing ` here.\r\n\r\nSee [`R`](crate::R) and `x`.",
            "a\r~~~\r[crate::X]\r~~~",
            "> ~~~\n> let last = items[usize::MAX];\n> ~~~",
            ">     let last = items[usize::MAX];",
            "1. ```\n   let last = items[usize::MAX];",
            "> <div>\n> [crate::X]\n> </div>",
            "see <a title=\"[crate::X]\">x</a>",
            "see <https://x.dev/a[crate::X]b>",
            "| `x | y` [crate::X] `z |\n|---|---|",
            "| `x | y` [crate::X] `z |\r|---|---|",
            "see [x](<crate::y>)",
            "see [x](< fn@f >)",
        ] {
            assert_eq!(unlink_rustdoc(text), None, "{text:?}");
            assert!(holds_rustdoc_link(text), "the guard misses {text:?}");
        }
    }

    /// A line of dashes under a heading, or a pipe in prose, is no table: the
    /// scan reads the text and rewrites its links.
    #[test]
    fn a_heading_underline_or_a_pipe_is_not_a_table() {
        for (text, shown) in [
            ("Title\n---\nsee [`A`]", "Title\n---\nsee `A`"),
            ("a | b and [`A`]", "a | b and `A`"),
        ] {
            assert_eq!(unlink_rustdoc(text).as_deref(), Some(shown), "{text:?}");
        }
    }

    /// A `[` right after `!` opens an image, and a `[label]` a colon follows
    /// may be a reference definition, in a quote or a list item too: the
    /// rewrite leaves both, and the guard fails on them. A use of the
    /// definition elsewhere is rewritten.
    #[test]
    fn an_image_and_a_definition_stay_as_written() {
        for text in [
            "an image ![x](crate::y) here",
            "[`Foo`]: crate::Foo",
            "  [crate::X]: https://docs.rs/x",
            "> [`Foo`]: crate::Foo",
            "- [crate::X]: https://docs.rs/x",
            "see [`Foo`]: the id",
        ] {
            assert_eq!(unlink_rustdoc(text), None, "{text:?}");
            assert!(holds_rustdoc_link(text), "the guard misses {text:?}");
        }
        let defined = "see `Foo`.\n\n[`Foo`]: crate::Foo";
        assert_eq!(
            unlink_rustdoc("see [`Foo`].\n\n[`Foo`]: crate::Foo").as_deref(),
            Some(defined)
        );
        assert!(holds_rustdoc_link(defined), "the guard misses {defined:?}");
    }

    /// A web link stays as written and passes the guard, padded or wrapped in
    /// `<…>` as Markdown allows, and so do brackets that are not link syntax.
    #[test]
    fn the_guard_leaves_web_links_and_plain_brackets() {
        for text in [
            "see [docs](https://x.dev)",
            "see [docs]( https://x.dev)",
            "see [docs](<https://x.dev>)",
            "see [docs](< https://x.dev>)",
            "see [crate::Point](https://docs.rs/x)",
            "see [the spec](http://x.dev/spec)",
            "write to [the team](mailto:team@x.dev)",
            "jump to [the top](#top)",
            "in [0, 1] and map[key]",
            "a bare [Recollection] reads like [sic]",
        ] {
            assert_eq!(unlink_rustdoc(text), None, "{text}");
            assert!(!holds_rustdoc_link(text), "the guard flags {text:?}");
        }
    }

    /// A label ends at its first `]` outside a code span, as Markdown reads
    /// it: the `]` in ``[`a]b`](crate::x)`` is code, and in
    /// ``[a `[` `b](Foo) c` `` the backticks pair past the `]`, so no link
    /// forms.
    #[test]
    fn a_label_ends_at_its_first_bracket_outside_code() {
        assert_eq!(
            unlink_rustdoc("odd [`a]b`](crate::x) link").as_deref(),
            Some("odd `a]b` link")
        );
        assert_eq!(unlink_rustdoc("see [a `[` `b](Foo) c` d"), None);
    }

    /// A backtick nothing closes is literal text in Markdown: the link
    /// after it is rewritten, and the guard reads it.
    #[test]
    fn a_stray_backtick_is_literal() {
        let text = "a stray ` then [x](crate::y)";
        assert_eq!(unlink_rustdoc(text).as_deref(), Some("a stray ` then x"));
        let left = "a stray ` then [x](crate::y \"t\")";
        assert_eq!(unlink_rustdoc(left), None);
        assert!(holds_rustdoc_link(left), "the guard misses {left:?}");
    }

    /// The rewrite copies a code span verbatim, link syntax and all. The guard
    /// reads the raw text, so it fails on link syntax even inside a code span:
    /// a published description never shows it.
    #[test]
    fn the_guard_flags_link_syntax_even_inside_a_code_span() {
        let text = "the syntax `[x](crate::y)` is code";
        assert_eq!(unlink_rustdoc(text), None);
        assert!(holds_rustdoc_link(text), "the guard misses {text:?}");
    }

    /// Whether `text` holds rustdoc link syntax: a `[` that opens on a code
    /// span (`` [`Point`] ``), a bracketed path (`[crate::Point]`, `[fn@f]`,
    /// `[a#b]`, `[Vec<T>]`, `[f()]`, `[m!{}]`, `[m!]`), a reference-style link
    /// (`[x][y]`, `[x][]`), a reference definition (any `]:`), or an inline link
    /// to anything but a URL or a fragment. Every link the rewrite recognizes
    /// is one of these.
    ///
    /// It reads the raw text, so no Markdown construct (a code span, a quote, a
    /// list item) can hide one of these forms from it. What that costs: a
    /// description cannot show one even as code (`` `[x](y)` ``,
    /// ``[`asc`, `desc`]``, `&[Vec<f32>]`), give a web link code text, or write
    /// a reference-style link or definition, even to a URL; and prose that
    /// looks like one fails too (`[0, 1]: …`, `m[i][j]`, `[#2261]`,
    /// `[ops@x.dev]`). A bare `[Point]` passes: it reads the same as `[sic]`.
    /// velesdb-server's guard over its OpenAPI document applies the same rules
    /// (#2263).
    fn holds_rustdoc_link(text: &str) -> bool {
        text.contains("][")
            || text.contains("]:")
            || text
                .match_indices("](")
                .any(|(at, _)| !is_url(target_start(&text[at + 2..])))
            || text.match_indices('[').any(|(at, _)| {
                let after = &text[at + 1..];
                after.trim_start().starts_with('`') || brackets_a_path(after)
            })
    }

    /// Whether the label `after` starts, up to its `]`, names a path: it holds
    /// `::`, `@`, `#` or `<`, or ends in `()`, `!{}` or `!`. A `(` after the
    /// `]` makes it the text of an inline link instead, which
    /// [`holds_rustdoc_link`] reads by its target.
    fn brackets_a_path(after: &str) -> bool {
        after.split_once(']').is_some_and(|(label, rest)| {
            !rest.starts_with('(')
                && (label.contains("::")
                    || label.contains(['@', '#', '<'])
                    || label.ends_with("()")
                    || label.ends_with("!{}")
                    || label.ends_with('!'))
        })
    }

    /// `raw` past any whitespace or `<`, where a link target starts.
    fn target_start(raw: &str) -> &str {
        let target = raw.trim_start();
        target.strip_prefix('<').map_or(target, str::trim_start)
    }

    /// Whether a link target is a URL or a fragment of the page. A `mailto:`
    /// followed by a second `:` is a path (`mailto::X`), not an address.
    fn is_url(target: &str) -> bool {
        ["http://", "https://", "#"]
            .iter()
            .any(|prefix| target.starts_with(prefix))
            || target
                .strip_prefix("mailto:")
                .is_some_and(|address| !address.starts_with(':'))
    }

    #[test]
    fn the_guard_flags_each_link_form() {
        for text in [
            "a JSON-encoded [`Point`].",
            "a JSON-encoded [ `Point` ].",
            "see [`Point`](https://docs.rs/velesdb-core).",
            "see [crate::Point].",
            "see [Vec<u8, A>::new].",
            "see [Vec<u8>].",
            "see [stream()].",
            "see [vec!].",
            "see [vec!{}].",
            "see [fn@stream].",
            "see [Point#fields].",
            "see [the point][Point].",
            "see [Point][].",
            "[p]: crate::Point",
            "> [p]: crate::Point",
            "- [p]: crate::Point",
            "[the\npoint]: crate::Point",
            "see [the point](crate::Point).",
            "see [the point](< crate::Point >).",
            "see [x](mailto::X).",
        ] {
            assert!(holds_rustdoc_link(text), "{text}");
        }
    }

    /// Prose that looks like link syntax fails the guard too: the documented
    /// cost of reading the raw text.
    #[test]
    fn the_guard_flags_the_prose_it_documents_as_a_cost() {
        for text in [
            "weights in [0, 1]: higher wins",
            "m[i][j] indexes",
            "see [#2261]",
            "write to [ops@x.dev]",
            "one of [`asc`, `desc`]",
            "a `&[Vec<f32>]` slice",
        ] {
            assert!(holds_rustdoc_link(text), "{text}");
        }
    }

    #[test]
    fn the_guard_flags_an_inline_link_the_rewrite_leaves() {
        for text in [
            "odd [x](crate::y \"t\") link",
            "odd [x](Foo \"t\") link",
            "odd [x](< crate::y > \"t\") link",
        ] {
            assert_eq!(unlink_rustdoc(text), None, "{text}");
            let mut linked = Vec::new();
            collect_linked(&json!({ "description": text }), "", &mut linked);
            assert_eq!(linked, ["/description"], "{text}");
        }
    }

    #[test]
    fn the_guard_reads_a_property_named_like_a_keyword() {
        let mut linked = Vec::new();
        collect_linked(
            &json!({ "properties": { "default": { "description": "[`x`]" } } }),
            "",
            &mut linked,
        );
        assert_eq!(linked, ["/properties/default/description"]);
    }

    #[test]
    fn the_guard_leaves_instance_data_as_the_rewrite_does() {
        let mut linked = Vec::new();
        collect_linked(
            &json!({
                "default": { "description": "[`kept`]" },
                "examples": [{ "description": "[`kept`]" }]
            }),
            "",
            &mut linked,
        );
        assert!(linked.is_empty(), "{linked:?}");
    }

    fn collect_linked(value: &Value, path: &str, out: &mut Vec<String>) {
        match value {
            Value::Object(map) => {
                for (key, child) in map {
                    collect_under_key(key, child, &format!("{path}/{key}"), out);
                }
            }
            Value::Array(items) => {
                for (i, item) in items.iter().enumerate() {
                    collect_linked(item, &format!("{path}/{i}"), out);
                }
            }
            _ => {}
        }
    }

    /// One key's value, read as the rewrite reads it.
    fn collect_under_key(key: &str, child: &Value, here: &str, out: &mut Vec<String>) {
        use super::super::walks::{INSTANCE_KEYWORDS, NAMED_SCHEMA_MAPS};
        match child {
            Value::String(text) if key == "description" => {
                if holds_rustdoc_link(text) {
                    out.push(here.to_owned());
                }
            }
            // Instance data is a value, not a doc comment: the rewrite
            // leaves it, and so does the guard.
            _ if INSTANCE_KEYWORDS.contains(&key) => {}
            // Keys here are names, not keywords, as in the rewrite.
            Value::Object(named) if NAMED_SCHEMA_MAPS.contains(&key) => {
                for (name, schema) in named {
                    collect_linked(schema, &format!("{here}/{name}"), out);
                }
            }
            _ => collect_linked(child, here, out),
        }
    }
}
