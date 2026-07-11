//! Backward-compatibility tests for the extended [`DatabaseObserver`] port.
//!
//! These tests prove Requirement 3.1 / 3.2 / 3.3: an observer that implements
//! ONLY the pre-existing hooks (the ones that existed before the read-path
//! `on_query_request` gate was added) still compiles and keeps its prior
//! behavior. The newly-added `on_query_request` hook is NOT overridden, so it
//! must fall back to the defaulted allow-all decision
//! ([`AccessDecision::Allow`]) — leaving the read path unchanged for existing
//! consumers.
//!
//! The very fact that [`LegacyObserver`] compiles without mentioning
//! `on_query_request` is the compile-time half of the guarantee; the runtime
//! assertions below cover the behavioral half.

use super::{AccessDecision, DatabaseObserver, QueryAccessContext, QueryOperationKind};
use crate::collection::CollectionType;
use parking_lot::Mutex;

/// A minimal observer that overrides ONLY methods that existed before the
/// read-path gate was introduced (`on_collection_created`, `on_upsert`,
/// `on_ddl_request`). It deliberately does NOT mention `on_query_request`,
/// relying on the trait default — this is what a pre-extension premium
/// observer looks like, and it must keep compiling (Requirement 3.3).
#[derive(Default)]
struct LegacyObserver {
    created: Mutex<Vec<String>>,
    upserts: Mutex<Vec<(String, usize)>>,
    ddl_should_reject: bool,
}

impl DatabaseObserver for LegacyObserver {
    fn on_collection_created(&self, name: &str, _kind: &CollectionType) {
        self.created.lock().push(name.to_string());
    }

    fn on_upsert(&self, collection: &str, point_count: usize) {
        self.upserts
            .lock()
            .push((collection.to_string(), point_count));
    }

    fn on_ddl_request(&self, operation: &str, collection_name: &str) -> crate::Result<()> {
        if self.ddl_should_reject {
            return Err(crate::Error::Query(format!(
                "rejected {operation} on {collection_name}"
            )));
        }
        Ok(())
    }
}

#[test]
fn legacy_observer_query_request_defaults_to_allow() {
    // The legacy observer never overrode on_query_request; the defaulted
    // allow-all decision must apply so the read path is unchanged (Req 3.1/3.2).
    let observer = LegacyObserver::default();
    let collection = "documents".to_string();
    let ctx = QueryAccessContext {
        collection: &collection,
        operation: QueryOperationKind::Select,
        principal: None,
        tenant_hint: None,
    };

    let decision = observer
        .on_query_request(&ctx)
        .expect("default on_query_request never errors");

    assert!(
        matches!(decision, AccessDecision::Allow),
        "an observer that does not override on_query_request must inherit the \
         allow-all default, got {decision:?}"
    );
}

#[test]
fn legacy_observer_query_request_allows_every_operation_kind() {
    // The defaulted decision is allow-all regardless of which read path runs.
    let observer = LegacyObserver::default();
    let collection = "vectors".to_string();

    for operation in [
        QueryOperationKind::VectorSearch,
        QueryOperationKind::TextSearch,
        QueryOperationKind::HybridSearch,
        QueryOperationKind::GraphTraversal,
        QueryOperationKind::Select,
    ] {
        let ctx = QueryAccessContext {
            collection: &collection,
            operation,
            principal: None,
            tenant_hint: None,
        };

        let decision = observer
            .on_query_request(&ctx)
            .expect("default on_query_request never errors");

        assert!(
            matches!(decision, AccessDecision::Allow),
            "operation {operation:?} must be allowed by the default hook"
        );
    }
}

#[test]
fn legacy_observer_preexisting_hooks_still_behave() {
    // The overridden pre-existing hooks keep their prior behavior unchanged.
    let observer = LegacyObserver::default();

    observer.on_collection_created("docs", &CollectionType::MetadataOnly);
    observer.on_upsert("docs", 7);

    assert_eq!(observer.created.lock().as_slice(), ["docs"]);
    assert_eq!(
        observer.upserts.lock().as_slice(),
        [("docs".to_string(), 7)]
    );

    // Default (non-rejecting) DDL gate allows.
    assert!(observer.on_ddl_request("CREATE", "docs").is_ok());

    // A rejecting legacy observer still denies through the Result channel,
    // exactly as before the read-path gate existed (Requirement 3.5 unchanged).
    let rejecting = LegacyObserver {
        ddl_should_reject: true,
        ..LegacyObserver::default()
    };
    assert!(rejecting.on_ddl_request("DROP", "docs").is_err());
}

#[test]
fn legacy_observer_is_usable_as_a_trait_object() {
    // Backward-compatible consumers store observers as `dyn DatabaseObserver`.
    // A legacy observer must still satisfy the object-safe port and answer the
    // read gate via the default.
    let observer: &dyn DatabaseObserver = &LegacyObserver::default();
    let collection = "graph".to_string();
    let ctx = QueryAccessContext {
        collection: &collection,
        operation: QueryOperationKind::GraphTraversal,
        principal: Some("user-1"),
        tenant_hint: Some("tenant-a"),
    };

    let decision = observer
        .on_query_request(&ctx)
        .expect("default on_query_request never errors");
    assert!(matches!(decision, AccessDecision::Allow));
}
