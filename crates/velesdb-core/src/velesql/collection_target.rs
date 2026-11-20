//! The `_collection` parameter: how a top-level `MATCH` — the one statement
//! that runs without a `FROM` — names the collection it reads.
//!
//! Every other read names its collection in a `FROM`, which the grammar's
//! `select_stmt` requires, and a write names its table in the statement; the
//! key is read for neither. Core's read gate and executor resolve a `MATCH`
//! target here, and every adapter that addresses one — the REST server's
//! `/query`, the Tauri plugin's `query`, the WASM executor — fills and reads
//! the key through this module, so the rule exists once.

use std::borrow::Cow;
use std::collections::HashMap;

use crate::error::{Error, Result};
use crate::velesql::Query;

type Params = HashMap<String, serde_json::Value>;

/// The parameter key a top-level `MATCH` names its collection by.
pub const COLLECTION_PARAM: &str = "_collection";

/// The collection `params` names under [`COLLECTION_PARAM`]: `None` when the
/// key is absent.
///
/// # Errors
///
/// [`Error::QueryParameter`] (`VELES-038`) when the key is present but not a
/// non-empty string.
pub fn collection_param(params: &Params) -> Result<Option<&str>> {
    match params.get(COLLECTION_PARAM) {
        None => Ok(None),
        Some(serde_json::Value::String(name)) if !name.is_empty() => Ok(Some(name)),
        Some(other) => Err(Error::invalid_parameter(
            COLLECTION_PARAM,
            format!("expected a collection name (a non-empty string), got {other}"),
        )),
    }
}

/// The collection `query` reads first: its `FROM`, else, for a top-level
/// `MATCH`, [`collection_param`]. `None` when it names none — a `MATCH`
/// executor then refuses it — or is not a read addressed this way.
///
/// # Errors
///
/// [`collection_param`]'s error, for a top-level `MATCH` only.
pub fn collection_target<'q>(query: &'q Query, params: &'q Params) -> Result<Option<&'q str>> {
    if !query.select.from.is_empty() {
        return Ok(Some(&query.select.from));
    }
    if !query.is_match_query() {
        return Ok(None);
    }
    collection_param(params)
}

/// `params` with a request's own `collection` as [`COLLECTION_PARAM`], when
/// `query` is a top-level `MATCH`, `collection` is non-empty and `params`
/// set no such key: an explicit key wins. Borrowed whenever nothing is
/// filled.
#[must_use]
pub fn with_collection_param<'p>(
    query: &Query,
    params: &'p Params,
    collection: Option<&str>,
) -> Cow<'p, Params> {
    let fill = collection.filter(|name| {
        !name.is_empty() && query.is_match_query() && !params.contains_key(COLLECTION_PARAM)
    });
    match fill {
        Some(name) => {
            let mut owned = params.clone();
            owned.insert(COLLECTION_PARAM.to_string(), serde_json::json!(name));
            Cow::Owned(owned)
        }
        None => Cow::Borrowed(params),
    }
}

#[cfg(test)]
#[path = "collection_target_tests.rs"]
mod tests;
