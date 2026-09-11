//! Rewrites rustdoc's shortcut intra-doc-link syntax out of the OpenAPI
//! document `utoipa` generates (#2263).
//!
//! `utoipa` copies doc comments verbatim into `description` fields. A
//! shortcut link such as `` [`Point`] `` renders as a hyperlink in rustdoc's
//! own HTML, but nothing else understands that syntax: Swagger UI, a
//! generated SDK, or a person reading `docs/openapi.json` sees the literal
//! brackets. [`unlink`] walks the typed document and drops them everywhere a
//! doc comment could have put one.
//!
//! The walk is over `utoipa::openapi`'s own types, not a generic
//! `serde_json::Value` round trip: `OpenApi` derives `Serialize` and
//! `Deserialize` from the same struct shape, but several of its enums
//! (`RefOr`, `Schema`, `AdditionalProperties`) are `#[serde(untagged)]`, and
//! at least one combination of `#[serde(untagged)]` with a `#[serde(flatten)]`
//! field elsewhere in the tree makes deserializing a re-encoded `Value` fail
//! with "data did not match any variant of untagged enum RefOr" — confirmed
//! by round-tripping this crate's own document with no mutation at all.
//! Matching the real enum by hand sidesteps that failure mode entirely.

use std::collections::BTreeMap;

use utoipa::openapi::{
    path::{Operation, Parameter},
    schema::{AdditionalProperties, ArrayItems},
    Array, Components, Content, OpenApi, PathItem, RefOr, Response, Schema,
};

/// Applies the rewrite to every description reachable from `doc` and
/// returns it. Called once, from [`crate::ApiDoc::openapi`], so the served
/// `/api-docs/openapi.json` and the committed `docs/openapi.{json,yaml}`
/// snapshot are generated from the same already-rewritten value and can
/// never drift from each other.
pub(crate) fn unlink(mut doc: OpenApi) -> OpenApi {
    rewrite_opt(&mut doc.info.description);
    for tag in doc.tags.iter_mut().flatten() {
        rewrite_opt(&mut tag.description);
    }
    for item in doc.paths.paths.values_mut() {
        walk_path_item(item);
    }
    if let Some(components) = &mut doc.components {
        walk_components(components);
    }
    doc
}

fn walk_path_item(item: &mut PathItem) {
    rewrite_opt(&mut item.description);
    rewrite_opt(&mut item.summary);
    let operations = [
        &mut item.get,
        &mut item.put,
        &mut item.post,
        &mut item.delete,
        &mut item.options,
        &mut item.head,
        &mut item.patch,
        &mut item.trace,
    ];
    for operation in operations.into_iter().flatten() {
        walk_operation(operation);
    }
}

fn walk_operation(operation: &mut Operation) {
    rewrite_opt(&mut operation.description);
    rewrite_opt(&mut operation.summary);
    for parameter in operation.parameters.iter_mut().flatten() {
        walk_parameter(parameter);
    }
    if let Some(request_body) = &mut operation.request_body {
        rewrite_opt(&mut request_body.description);
        walk_content_map(&mut request_body.content);
    }
    for response in operation.responses.responses.values_mut() {
        walk_refor_response(response);
    }
}

fn walk_parameter(parameter: &mut Parameter) {
    rewrite_opt(&mut parameter.description);
    if let Some(schema) = &mut parameter.schema {
        walk_refor_schema(schema);
    }
}

fn walk_content_map(content: &mut BTreeMap<String, Content>) {
    for entry in content.values_mut() {
        if let Some(schema) = &mut entry.schema {
            walk_refor_schema(schema);
        }
    }
}

fn walk_refor_response(refor: &mut RefOr<Response>) {
    match refor {
        RefOr::Ref(reference) => rewrite_str(&mut reference.description),
        RefOr::T(response) => {
            rewrite_str(&mut response.description);
            for entry in response.content.values_mut() {
                if let Some(schema) = &mut entry.schema {
                    walk_refor_schema(schema);
                }
            }
        }
    }
}

fn walk_components(components: &mut Components) {
    for schema in components.schemas.values_mut() {
        walk_refor_schema(schema);
    }
    for response in components.responses.values_mut() {
        walk_refor_response(response);
    }
}

fn walk_refor_schema(refor: &mut RefOr<Schema>) {
    match refor {
        RefOr::Ref(reference) => rewrite_str(&mut reference.description),
        RefOr::T(schema) => walk_schema(schema),
    }
}

fn walk_schema(schema: &mut Schema) {
    match schema {
        Schema::Object(object) => {
            rewrite_opt(&mut object.description);
            for property in object.properties.values_mut() {
                walk_refor_schema(property);
            }
            if let Some(names) = &mut object.property_names {
                walk_schema(names);
            }
            if let Some(additional) = &mut object.additional_properties {
                if let AdditionalProperties::RefOr(refor) = &mut **additional {
                    walk_refor_schema(refor);
                }
            }
        }
        Schema::Array(array) => walk_array(array),
        Schema::OneOf(one_of) => walk_composite(&mut one_of.description, &mut one_of.items),
        Schema::AllOf(all_of) => walk_composite(&mut all_of.description, &mut all_of.items),
        Schema::AnyOf(any_of) => walk_composite(&mut any_of.description, &mut any_of.items),
        // `Schema` is `#[non_exhaustive]`: a variant this crate does not
        // know about carries no description this walk can reach yet.
        _ => {}
    }
}

fn walk_array(array: &mut Array) {
    rewrite_opt(&mut array.description);
    match &mut array.items {
        ArrayItems::RefOrSchema(refor) => walk_refor_schema(refor),
        ArrayItems::False => {}
    }
    for prefix_item in &mut array.prefix_items {
        walk_schema(prefix_item);
    }
}

/// `OneOf`, `AllOf`, and `AnyOf` share this exact shape: a description plus
/// a list of member schemas.
fn walk_composite(description: &mut Option<String>, items: &mut [RefOr<Schema>]) {
    rewrite_opt(description);
    for item in items {
        walk_refor_schema(item);
    }
}

/// Rewrites an optional description in place; a no-op absence stays absent.
fn rewrite_opt(text: &mut Option<String>) {
    if let Some(text) = text {
        rewrite_str(text);
    }
}

/// Rewrites a required description in place (`Response`'s and `Ref`'s
/// `description` fields are plain, non-optional `String`s).
fn rewrite_str(text: &mut String) {
    if let Some(rewritten) = unlink_rustdoc_shortcut(text) {
        *text = rewritten;
    }
}

/// Rewrites rustdoc's shortcut intra-doc-link syntax — `` [`path::Item`] ``
/// — to what rustdoc itself displays it as: the brackets dropped, the code
/// span kept.
///
/// Deliberately narrow: matches only a backtick-delimited span with no
/// whitespace, which is the shortcut form actually written in this crate's
/// doc comments — not the full rustdoc link grammar (disambiguators such as
/// `fn@`, explicit `[text](path)` links, reference-style links). A doc
/// comment needing one of those should spell the description in prose
/// instead of relying on this rewrite. Returns `None` when nothing changed.
fn unlink_rustdoc_shortcut(text: &str) -> Option<String> {
    if !text.contains("[`") {
        return None;
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    let mut changed = false;
    while let Some(start) = rest.find("[`") {
        let (before, after_marker) = rest.split_at(start);
        out.push_str(before);
        let after_open = &after_marker[2..]; // past "[`"
        let rewritten = after_open.find('`').and_then(|end| {
            let content = &after_open[..end];
            let after_content = &after_open[end + 1..];
            (!content.is_empty()
                && !content.contains(char::is_whitespace)
                && after_content.starts_with(']'))
            .then_some((content, &after_content[1..]))
        });
        match rewritten {
            Some((content, remainder)) => {
                out.push('`');
                out.push_str(content);
                out.push('`');
                rest = remainder;
                changed = true;
            }
            None => {
                out.push_str("[`");
                rest = after_open;
            }
        }
    }
    out.push_str(rest);
    changed.then_some(out)
}

#[cfg(test)]
#[path = "openapi_rustdoc_links_tests.rs"]
mod tests;
