//! The REST read surfaces that read a collection's rows outside a search —
//! `/aggregate`, aggregates sent to `/query`, `GET .../points/{id}` and
//! `POST .../points/scroll` — honour the control-plane read gate: a denial
//! refuses the request with the observer's error, and a scope narrows what
//! comes back.
//!
//! The fixture is one collection, `items`, with six rows: three in category
//! `a`, two in `b`, one with none. The observer either scopes `items` to
//! `cat = 'a'` or denies it.

mod common;

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use common::create_test_app_with_observer;
use serde_json::{json, Value};
use tempfile::TempDir;
use tower::ServiceExt;
use velesdb_core::observer::{AccessDecision, AccessScope, QueryAccessContext};
use velesdb_core::velesql::Parser;
use velesdb_core::DatabaseObserver;

/// What the observer answers for `items`; until one is set it allows.
#[derive(Clone, Copy)]
#[repr(u8)]
enum Rule {
    Deny = 1,
    ScopeToA = 2,
}

#[derive(Default)]
struct PolicyObserver {
    rule: AtomicU8,
}

impl PolicyObserver {
    fn set(&self, rule: Rule) {
        self.rule.store(rule as u8, Ordering::SeqCst);
    }
}

impl DatabaseObserver for PolicyObserver {
    fn on_query_request(&self, ctx: &QueryAccessContext) -> velesdb_core::Result<AccessDecision> {
        if ctx.collection != "items" {
            return Ok(AccessDecision::Allow);
        }
        Ok(match self.rule.load(Ordering::SeqCst) {
            r if r == Rule::Deny as u8 => AccessDecision::Deny(velesdb_core::Error::Query(
                "read denied by governance policy".to_string(),
            )),
            r if r == Rule::ScopeToA as u8 => {
                #[allow(clippy::field_reassign_with_default)] // `AccessScope` is non_exhaustive
                let mut scope = AccessScope::default();
                scope.filter = Parser::parse("SELECT * FROM items WHERE cat = 'a'")
                    .expect("test: parse scope")
                    .select
                    .where_clause;
                AccessDecision::AllowWithScope(scope)
            }
            _ => AccessDecision::Allow,
        })
    }
}

async fn send(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("Content-Type", "application/json")
        .body(body.map_or_else(Body::empty, |b| Body::from(b.to_string())))
        .expect("test: build request");
    let response = app.clone().oneshot(request).await.expect("test: request");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("test: read body");
    let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, json)
}

/// An app over `items` (six rows) whose reads go through `observer`.
async fn setup() -> (TempDir, Router, Arc<PolicyObserver>) {
    let dir = TempDir::new().expect("test: dir");
    let observer = Arc::new(PolicyObserver::default());
    let app = create_test_app_with_observer(&dir, observer.clone());
    let (status, _) = send(
        &app,
        "POST",
        "/collections",
        Some(json!({"name": "items", "dimension": 4, "metric": "cosine"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let points: Vec<Value> = [
        (1, Some("a")),
        (2, Some("a")),
        (3, Some("a")),
        (4, Some("b")),
        (5, Some("b")),
        (6, None),
    ]
    .iter()
    .map(|(id, cat)| {
        let payload = cat.map_or_else(|| json!({"amount": 1}), |c| json!({"cat": c, "amount": 1}));
        json!({"id": id, "vector": [1.0, 0.0, 0.0, 0.0], "payload": payload})
    })
    .collect();
    let (status, _) = send(
        &app,
        "POST",
        "/collections/items/points",
        Some(json!({ "points": points })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    (dir, app, observer)
}

fn query_body(sql: &str) -> Option<Value> {
    Some(json!({"query": sql, "params": {}}))
}

/// GIVEN `items` scoped to `cat = 'a'`
/// WHEN `COUNT(*)` is sent to `/aggregate` and to `/query`
/// THEN both count the 3 scoped rows, not the 6 stored.
#[tokio::test]
async fn aggregate_endpoints_count_only_the_scoped_rows() {
    let (_dir, app, observer) = setup().await;
    let (_, unscoped) = send(
        &app,
        "POST",
        "/aggregate",
        query_body("SELECT COUNT(*) AS n FROM items"),
    )
    .await;
    assert_eq!(unscoped["result"]["n"], 6, "{unscoped}");

    observer.set(Rule::ScopeToA);

    for endpoint in ["/aggregate", "/query"] {
        let (status, body) = send(
            &app,
            "POST",
            endpoint,
            query_body("SELECT COUNT(*) AS n FROM items"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{endpoint}: {body}");
        assert_eq!(body["result"]["n"], 3, "{endpoint}: {body}");
    }
}

/// GIVEN `items` scoped to `cat = 'a'`
/// WHEN a `GROUP BY cat` is sent to `/aggregate`
/// THEN only the `a` group comes back.
#[tokio::test]
async fn aggregate_group_by_returns_only_the_scoped_groups() {
    let (_dir, app, observer) = setup().await;
    observer.set(Rule::ScopeToA);

    let (status, body) = send(
        &app,
        "POST",
        "/aggregate",
        query_body("SELECT cat, COUNT(*) AS n FROM items GROUP BY cat"),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let groups = body["result"].as_array().expect("test: groups array");
    assert_eq!(groups.len(), 1, "{body}");
    assert_eq!(groups[0]["cat"], "a", "{body}");
}

/// GIVEN the observer denies `items`
/// WHEN an aggregate is sent to `/aggregate` or `/query`
/// THEN the request is refused and carries the observer's message.
#[tokio::test]
async fn aggregate_endpoints_refuse_a_denied_read() {
    let (_dir, app, observer) = setup().await;
    observer.set(Rule::Deny);

    for endpoint in ["/aggregate", "/query"] {
        let (status, body) = send(
            &app,
            "POST",
            endpoint,
            query_body("SELECT COUNT(*) AS n FROM items"),
        )
        .await;
        assert!(
            !status.is_success(),
            "{endpoint} must refuse, got {status}: {body}"
        );
        assert!(
            body.to_string()
                .contains("read denied by governance policy"),
            "{endpoint}: {body}"
        );
    }
}

/// GIVEN the observer denies `items`, then scopes it to `cat = 'a'`
/// WHEN a point is read by id
/// THEN the denial refuses it, and an out-of-scope point reads as not found
///      while an in-scope one is returned.
#[tokio::test]
async fn get_point_honours_the_read_gate() {
    let (_dir, app, observer) = setup().await;
    observer.set(Rule::Deny);
    let (status, body) = send(&app, "GET", "/collections/items/points/4", None).await;
    assert!(
        !status.is_success(),
        "denied get must refuse, got {status}: {body}"
    );

    observer.set(Rule::ScopeToA);
    let (status, _) = send(&app, "GET", "/collections/items/points/4", None).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "an out-of-scope point is hidden"
    );
    let (status, body) = send(&app, "GET", "/collections/items/points/1", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

/// GIVEN the observer denies `items`, then scopes it to `cat = 'a'`
/// WHEN the collection is scrolled
/// THEN the denial refuses it, and the scope returns only the 3 `a` rows.
#[tokio::test]
async fn scroll_honours_the_read_gate() {
    let (_dir, app, observer) = setup().await;
    observer.set(Rule::Deny);
    let (status, body) = send(
        &app,
        "POST",
        "/collections/items/points/scroll",
        Some(json!({"batch_size": 100})),
    )
    .await;
    assert!(
        !status.is_success(),
        "denied scroll must refuse, got {status}: {body}"
    );

    observer.set(Rule::ScopeToA);
    let (status, body) = send(
        &app,
        "POST",
        "/collections/items/points/scroll",
        Some(json!({"batch_size": 100})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["points"].as_array().map(Vec::len), Some(3), "{body}");
}
