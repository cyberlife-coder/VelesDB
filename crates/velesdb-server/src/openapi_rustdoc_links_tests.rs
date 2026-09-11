use super::*;

// The four forms #2263 found leaking into docs/openapi.json.
#[test]
fn rewrites_the_known_leaking_forms() {
    assert_eq!(
        unlink_rustdoc_shortcut("every [`STATS_INTERVAL`] nodes"),
        Some("every `STATS_INTERVAL` nodes".to_string())
    );
    assert_eq!(
        unlink_rustdoc_shortcut("per [`super::helpers::http_status_for_error`]"),
        Some("per `super::helpers::http_status_for_error`".to_string())
    );
    assert_eq!(
        unlink_rustdoc_shortcut("a JSON-encoded [`Point`]."),
        Some("a JSON-encoded `Point`.".to_string())
    );
    assert_eq!(
        unlink_rustdoc_shortcut("mirroring [`NodeStatsResponse::estimated`]."),
        Some("mirroring `NodeStatsResponse::estimated`.".to_string())
    );
}

#[test]
fn rewrites_two_links_in_one_string() {
    assert_eq!(
        unlink_rustdoc_shortcut("[`A`] and [`B`]"),
        Some("`A` and `B`".to_string())
    );
}

#[test]
fn leaves_plain_text_untouched() {
    assert_eq!(unlink_rustdoc_shortcut("plain text, no links"), None);
}

#[test]
fn leaves_non_shortcut_brackets_untouched() {
    // No backtick-delimited span: not the syntax this rewrite targets.
    assert_eq!(unlink_rustdoc_shortcut("in [0, 1) range"), None);
    // Whitespace inside the span: not a valid rustdoc path.
    assert_eq!(unlink_rustdoc_shortcut("either [`a | b`]"), None);
    // No closing `]`: an unterminated span, left as written.
    assert_eq!(unlink_rustdoc_shortcut("an unbalanced [`a"), None);
}

#[test]
fn unlink_rewrites_every_reachable_description() {
    use utoipa::openapi::path::{HttpMethod, OperationBuilder, PathItemBuilder};
    use utoipa::openapi::{
        ContentBuilder, InfoBuilder, ObjectBuilder, OpenApiBuilder, PathsBuilder, ResponseBuilder,
        ResponsesBuilder,
    };

    let leaf_schema = ObjectBuilder::new()
        .description(Some("leaf [`Leaf`]".to_string()))
        .build();
    let response = ResponseBuilder::new()
        .description("response [`Resp`]")
        .content(
            "application/json",
            ContentBuilder::new().schema(Some(leaf_schema)).build(),
        )
        .build();
    let operation = OperationBuilder::new()
        .description(Some("operation [`Op`]".to_string()))
        .responses(ResponsesBuilder::new().response("200", response).build())
        .build();
    let path_item = PathItemBuilder::new()
        .operation(HttpMethod::Get, operation)
        .build();
    let openapi = OpenApiBuilder::new()
        .info(
            InfoBuilder::new()
                .description(Some("info [`Info`]".to_string()))
                .build(),
        )
        .paths(PathsBuilder::new().path("/x", path_item).build())
        .build();

    let rewritten = unlink(openapi);
    let json = rewritten.to_json().expect("test: serializes");
    assert!(
        !json.contains("[`"),
        "a rustdoc shortcut link survived the walk: {json}"
    );
    assert!(
        json.contains("`Leaf`"),
        "leaf schema description untouched: {json}"
    );
    assert!(
        json.contains("`Resp`"),
        "response description untouched: {json}"
    );
    assert!(
        json.contains("`Op`"),
        "operation description untouched: {json}"
    );
    assert!(
        json.contains("`Info`"),
        "info description untouched: {json}"
    );
}
