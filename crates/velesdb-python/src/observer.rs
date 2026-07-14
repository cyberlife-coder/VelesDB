//! `PyObserver` — bridges core [`DatabaseObserver`] lifecycle hooks to a single
//! Python callable.
//!
//! The Python SDK exposes the four *notify* callbacks (create/delete/upsert/
//! query) plus the read-path **veto** hook (`query_request`): the callback may
//! refuse a read by returning `False` or a string reason, so an SDK embedder can
//! enforce governance on every gated read (VelesQL `SELECT`/`MATCH`). The DDL/DML
//! veto hooks (`on_ddl_request` / `on_dml_mutation_request`) keep the trait's
//! default-allow behavior and remain unexposed.
//!
//! # GIL safety
//!
//! Core fires every callback *after* dropping the collection write guard, so
//! re-acquiring the GIL here cannot deadlock the core-lock against the GIL.
//! Notify callbacks swallow any [`PyErr`] (logged, never propagated) and never
//! panic — the [`DatabaseObserver`] contract forbids panicking. The
//! `query_request` veto also never panics: a raised callback error is logged and
//! treated as *allow* (fail-open on observer error, never break a read on a bug
//! in user policy code — an explicit `False`/reason is the only way to deny).
//!
//! # Strict (fail-closed) mode
//!
//! Fail-open is the default so a bug in user policy never breaks reads. Governance
//! / RBAC deployments can opt into **strict mode** (`Database(..., observer_strict=True)`):
//! then any *error* while consulting the policy — the callback raising, a
//! field-population failure, or an uninterpretable return value (e.g. a list) —
//! resolves to **deny** instead of allow, so a broken policy fails closed rather
//! than silently disabling governance. Explicit decisions (`None`/`True`/`False`/
//! str/`dict`) are unaffected by the mode.

use pyo3::prelude::*;
use pyo3::types::PyDict;
use velesdb_core::collection::CollectionType;
use velesdb_core::observer::{AccessDecision, AccessScope, QueryAccessContext, QueryOperationKind};
use velesdb_core::velesql::{Condition, Parser};
use velesdb_core::{DatabaseObserver, Error};

/// A [`DatabaseObserver`] that forwards lifecycle events to one Python callable.
///
/// The callback is invoked as `callback(event, **fields)`, where `event` is one
/// of `"collection_created"`, `"collection_deleted"`, `"upsert"`, or `"query"`,
/// and the keyword fields carry the event payload:
///
/// | event                | keyword fields              |
/// |----------------------|-----------------------------|
/// | `collection_created` | `name`, `kind`              |
/// | `collection_deleted` | `name`                      |
/// | `upsert`             | `collection`, `point_count` |
/// | `query`              | `collection`, `duration_us` |
///
/// `kind` is one of `"vector"`, `"metadata"`, `"graph"`, or `"unknown"`
/// (the last guards against a future [`CollectionType`] variant).
pub struct PyObserver {
    /// User-supplied Python callable. `Py<PyAny>` is `Send + Sync`, satisfying
    /// the `DatabaseObserver: Send + Sync` bound.
    cb: Py<PyAny>,
    /// Read-path failure policy. When `false` (default), an *error* while
    /// consulting the policy — the callback raising, a field-population
    /// failure, or an uninterpretable return value — resolves to **allow**
    /// (fail-open: a bug in user policy never breaks a read). When `true`
    /// (opt-in, for RBAC / governance deployments), those same error cases
    /// resolve to **deny** (fail-closed), so a broken policy denies rather
    /// than silently disabling governance. Explicit decisions
    /// (`None`/`True`/`False`/str/`dict`) behave identically in both modes.
    strict: bool,
}

impl PyObserver {
    /// Wraps a Python callable as a core observer.
    ///
    /// `strict` selects the read-path failure policy: `false` = fail-open
    /// (default, backward compatible), `true` = fail-closed on any error while
    /// consulting the policy. See [`PyObserver::strict`].
    pub fn new(cb: Py<PyAny>, strict: bool) -> Self {
        Self { cb, strict }
    }

    /// The [`AccessDecision`] to use when consulting the policy errors out:
    /// `Deny` under strict mode, `Allow` otherwise.
    fn on_error_decision(&self, what: &str) -> AccessDecision {
        if self.strict {
            AccessDecision::Deny(Error::Query(format!(
                "read denied by observer strict mode: {what}"
            )))
        } else {
            AccessDecision::Allow
        }
    }

    /// Invokes the callback as `callback(event, **fields)`, swallowing any
    /// `PyErr` so an observer side effect can never break a core operation.
    ///
    /// Takes the already-held GIL token `py` so callers acquire the GIL exactly
    /// once per event (build the `PyDict`, then dispatch under the same token).
    fn dispatch(&self, py: Python<'_>, event: &str, fields: &Bound<'_, PyDict>) {
        if let Err(err) = self.cb.bind(py).call((event,), Some(fields)) {
            eprintln!("[velesdb] observer callback '{event}' raised; ignoring: {err}");
        }
    }
}

/// Maps a [`CollectionType`] to a stable kind string. The `_` arm keeps this
/// total against the `#[non_exhaustive]` enum — it must never panic.
fn collection_kind(kind: &CollectionType) -> &'static str {
    match kind {
        CollectionType::Vector { .. } => "vector",
        CollectionType::MetadataOnly => "metadata",
        CollectionType::Graph { .. } => "graph",
        _ => "unknown",
    }
}

/// Maps a [`QueryOperationKind`] to a stable operation string for the callback.
/// The `_` arm keeps this total against the `#[non_exhaustive]` enum.
fn operation_str(op: QueryOperationKind) -> &'static str {
    match op {
        QueryOperationKind::VectorSearch => "vector_search",
        QueryOperationKind::TextSearch => "text_search",
        QueryOperationKind::HybridSearch => "hybrid_search",
        QueryOperationKind::GraphTraversal => "graph_traversal",
        QueryOperationKind::Select => "select",
        _ => "unknown",
    }
}

/// Interprets a `query_request` callback return value as an [`AccessDecision`]:
///
/// * `None` or `True` (or any other truthy object) → [`AccessDecision::Allow`]
///   — so an existing notify-only callback that ignores the event and returns
///   `None` keeps allowing every read (backward compatible);
/// * `False` → [`AccessDecision::Deny`] with a default reason;
/// * a `str` → `Deny` carrying that string as the human-readable reason;
/// * a `dict` → [`AccessDecision::AllowWithScope`] — the read is allowed but
///   *narrowed*. It MUST carry an enforceable `"filter"` (a `VelesQL` WHERE
///   predicate string, e.g. `"tenant = 'acme'"`, AND-composed into the query);
///   `"tenant"` is an optional audit hint OSS does not itself narrow by. A dict
///   with a missing/empty/tenant-only or unparseable `"filter"` **denies**
///   (fail closed) rather than allowing an unscoped read.
fn interpret_decision(ret: &Bound<'_, PyAny>, strict: bool) -> AccessDecision {
    if ret.is_none() {
        return AccessDecision::Allow;
    }
    // A string is a deny *reason*. Check before bool so `""`—an empty reason—
    // still denies rather than being coerced to a falsey bool.
    if let Ok(reason) = ret.extract::<String>() {
        return AccessDecision::Deny(Error::Query(format!("read denied by observer: {reason}")));
    }
    if let Ok(allow) = ret.extract::<bool>() {
        return if allow {
            AccessDecision::Allow
        } else {
            AccessDecision::Deny(Error::Query("read denied by observer policy".to_string()))
        };
    }
    // A dict expresses an allow-with-scope narrowing.
    if let Ok(dict) = ret.cast::<PyDict>() {
        return interpret_scope(dict);
    }
    // Any other return type (e.g. a list, an int, an object) is not a valid
    // decision. Fail-open by default (treat as no-opinion allow); fail-closed
    // under strict mode so a policy bug that returns the wrong type denies
    // rather than silently allowing.
    if strict {
        return AccessDecision::Deny(Error::Query(
            "read denied by observer strict mode: policy returned an \
             uninterpretable value (expected None/bool/str/dict)"
                .to_string(),
        ));
    }
    AccessDecision::Allow
}

/// Builds an [`AccessDecision::AllowWithScope`] from a callback-returned dict.
///
/// A scope dict **must** yield an enforceable `"filter"` predicate. Because OSS
/// enforces narrowing only through the `filter` (it forwards `tenant` for audit
/// but does not itself narrow by it), a dict with no parseable `filter` —
/// empty (`{}`), `tenant`-only (`{"tenant": …}`), or a `filter` that fails to
/// parse — is treated as **DENY** (fail closed). Otherwise a policy author who
/// writes `return {"tenant": t}` believing they scoped the read would instead
/// receive every tenant's rows. A proper `{"filter": "<predicate>"}` narrows;
/// bare `True`/`None` stays Allow and `False`/str stays Deny (handled upstream).
fn interpret_scope(dict: &Bound<'_, PyDict>) -> AccessDecision {
    let tenant = dict
        .get_item("tenant")
        .ok()
        .flatten()
        .and_then(|v| v.extract::<String>().ok());

    // An enforceable filter predicate is mandatory — its absence fails closed.
    let predicate = match dict.get_item("filter") {
        Ok(Some(v)) if !v.is_none() => match v.extract::<String>() {
            Ok(predicate) => predicate,
            Err(_) => {
                return AccessDecision::Deny(Error::Query(
                    "observer scope 'filter' must be a VelesQL predicate string".to_string(),
                ))
            }
        },
        _ => {
            return AccessDecision::Deny(Error::Query(
                "observer returned an allow-with-scope dict without an enforceable 'filter' \
                 predicate; OSS does not narrow by 'tenant' alone, so the read is denied \
                 (fail closed). Return a {\"filter\": \"<VelesQL predicate>\"} dict to scope, \
                 or None/True to allow."
                    .to_string(),
            ))
        }
    };

    let condition = match parse_scope_predicate(&predicate) {
        Ok(condition) => condition,
        Err(err) => {
            return AccessDecision::Deny(Error::Query(format!(
                "invalid observer scope filter: {err}"
            )))
        }
    };

    // `AccessScope` is `#[non_exhaustive]`, so struct-literal construction is
    // not available to this crate — build via `Default` then assign fields.
    #[allow(clippy::field_reassign_with_default)]
    let scope = {
        let mut scope = AccessScope::default();
        scope.tenant = tenant;
        scope.filter = Some(condition);
        scope
    };
    AccessDecision::AllowWithScope(scope)
}

/// Parses a bare `VelesQL` WHERE predicate (e.g. `"tenant = 'acme'"`) into a
/// [`Condition`] by wrapping it in a throwaway `SELECT` and reusing the real
/// parser — no parallel filter grammar is introduced.
fn parse_scope_predicate(predicate: &str) -> Result<Condition, String> {
    let sql = format!("SELECT * FROM _scope WHERE {predicate}");
    let query = Parser::parse(&sql).map_err(|e| e.message)?;
    query
        .select
        .where_clause
        .ok_or_else(|| "scope predicate produced no WHERE condition".to_string())
}

impl DatabaseObserver for PyObserver {
    fn on_collection_created(&self, name: &str, kind: &CollectionType) {
        let kind = collection_kind(kind);
        Python::attach(|py| {
            let fields = PyDict::new(py);
            if fields.set_item("name", name).is_err() || fields.set_item("kind", kind).is_err() {
                return;
            }
            self.dispatch(py, "collection_created", &fields);
        });
    }

    fn on_collection_deleted(&self, name: &str) {
        Python::attach(|py| {
            let fields = PyDict::new(py);
            if fields.set_item("name", name).is_err() {
                return;
            }
            self.dispatch(py, "collection_deleted", &fields);
        });
    }

    fn on_upsert(&self, collection: &str, point_count: usize) {
        Python::attach(|py| {
            let fields = PyDict::new(py);
            if fields.set_item("collection", collection).is_err()
                || fields.set_item("point_count", point_count).is_err()
            {
                return;
            }
            self.dispatch(py, "upsert", &fields);
        });
    }

    fn on_query(&self, collection: &str, duration_us: u64) {
        Python::attach(|py| {
            let fields = PyDict::new(py);
            if fields.set_item("collection", collection).is_err()
                || fields.set_item("duration_us", duration_us).is_err()
            {
                return;
            }
            self.dispatch(py, "query", &fields);
        });
    }

    /// Read-path veto. Invokes the callback as
    /// `callback("query_request", collection=…, operation=…, principal=…, tenant=…)`
    /// and maps its return value through [`interpret_decision`]: `None`/`True`
    /// allow, `False`/str deny, `dict` allow-with-scope (a `"filter"` VelesQL
    /// predicate string and/or `"tenant"` hint narrowing the read). A callback
    /// that raises, or a field-population failure, allows the read (fail-open on
    /// error so a bug in user policy code never breaks a query — only an explicit
    /// refusal denies). Fires on every gated read: VelesQL `SELECT`/`MATCH` and,
    /// via the Python SDK's direct-search gate, `vector_search` / `text_search` /
    /// `hybrid_search` (closing the "notify-only" gap, CORE-5).
    fn on_query_request(&self, ctx: &QueryAccessContext) -> velesdb_core::Result<AccessDecision> {
        let decision = Python::attach(|py| {
            let fields = PyDict::new(py);
            if fields.set_item("collection", ctx.collection).is_err()
                || fields
                    .set_item("operation", operation_str(ctx.operation))
                    .is_err()
                || fields.set_item("principal", ctx.principal).is_err()
                || fields.set_item("tenant", ctx.tenant_hint).is_err()
            {
                return self.on_error_decision("could not populate query_request fields");
            }
            match self.cb.bind(py).call(("query_request",), Some(&fields)) {
                Ok(ret) => interpret_decision(&ret, self.strict),
                Err(err) => {
                    let verb = if self.strict { "denying" } else { "allowing" };
                    eprintln!("[velesdb] observer callback 'query_request' raised; {verb}: {err}");
                    self.on_error_decision("query_request callback raised")
                }
            }
        });
        Ok(decision)
    }
}
