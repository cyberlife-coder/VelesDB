//! Whether a graph element matches a pattern element: the one definition
//! every MATCH engine applies — core's walk and the WASM executor alike.
//!
//! - **Labels.** `(n:A:B)` requires every label: the node carries `A` and
//!   `B`.
//! - **Inline properties.** `(n {k: v})` and `-[r {k: v}]->` mean
//!   `n.k = v` / `r.k = v` for each pair, evaluated as WHERE evaluates `=`:
//!   in three-valued logic (a `null` or absent property, or a `NULL`
//!   literal, is UNKNOWN, so it does not match), numbers by exact value, and
//!   `k = id` reading the element's id. A `$name` is bound by
//!   [`super::bind_match_clause`] before any element is tested.

use std::borrow::Cow;
use std::collections::HashMap;
use std::hash::BuildHasher;

use super::Value;

/// A property of an element with id `id`, as every MATCH reader sees it:
/// `id` is the element's id, whatever its payload holds; any other key is
/// what `lookup` finds.
pub fn element_property<'a>(
    id: u64,
    property: &str,
    lookup: impl FnOnce(&str) -> Option<&'a serde_json::Value>,
) -> Option<Cow<'a, serde_json::Value>> {
    if property == "id" {
        return Some(Cow::Owned(serde_json::Value::from(id)));
    }
    lookup(property).map(Cow::Borrowed)
}

/// Whether an element whose properties `read` answers satisfies the inline
/// properties `{k: v, ..}`: every `k = v` is known TRUE (see the module
/// docs).
pub fn inline_properties_hold<'a, S: BuildHasher>(
    properties: &HashMap<String, Value, S>,
    read: impl Fn(&str) -> Option<Cow<'a, serde_json::Value>>,
) -> bool {
    properties.iter().all(|(key, literal)| {
        let operand = read(key);
        crate::filter::equality_truth(operand.as_deref(), &literal.to_json()) == Some(true)
    })
}

/// Whether a node carrying `labels` has every label of `required`.
pub fn has_every_label(required: &[String], labels: &[impl AsRef<str>]) -> bool {
    required
        .iter()
        .all(|want| labels.iter().any(|have| have.as_ref() == want))
}

#[cfg(test)]
#[path = "pattern_match_tests.rs"]
mod tests;
