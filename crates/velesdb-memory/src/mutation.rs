//! Pre-mutation classification for the native online-migration coordinator.

use std::sync::Arc;

use parking_lot::RwLock;

use crate::MemoryError;

#[allow(dead_code)] // Internal until the control-surface slice exposes online migration.
mod catchup;
#[allow(dead_code)] // The catch-up slice consumes the journal before the control surface ships.
mod journal;

/// Idempotent source state that a migration must re-read after a mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum DirtyKey {
    Fact(u64),
    OutgoingEdges(u64),
}

/// Sink invoked before a native source mutation is allowed to run.
pub(crate) trait MutationObserver: Send + Sync {
    fn before_mutation(&self, key: DirtyKey) -> Result<(), MemoryError>;
}

/// Replaceable observer slot. `None` is the steady-state no-capture path.
#[derive(Default)]
pub(crate) struct MutationCapture {
    observer: RwLock<Option<Arc<dyn MutationObserver>>>,
}

impl MutationCapture {
    pub(crate) fn observe(&self, key: DirtyKey) -> Result<(), MemoryError> {
        let observer = self.observer.read();
        match observer.as_ref() {
            Some(observer) => observer.before_mutation(key),
            None => Ok(()),
        }
    }

    #[allow(dead_code)] // The journal activation path remains internal until slice 6.
    pub(crate) fn replace(
        &self,
        observer: Option<Arc<dyn MutationObserver>>,
    ) -> Result<(), MemoryError> {
        let mut installed = self.observer.write();
        if installed.is_some() && observer.is_some() {
            return Err(MemoryError::MigrationCapture(
                "a mutation observer is already active".to_owned(),
            ));
        }
        *installed = observer;
        Ok(())
    }

    pub(crate) fn is_active(&self) -> bool {
        self.observer.read().is_some()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::sync::{mpsc, Arc};
    use std::time::Duration;

    use parking_lot::{Condvar, Mutex};

    use super::{DirtyKey, MutationObserver};
    use crate::{EmbedError, Embedder, MemoryError, MemoryService};

    struct BlockingEmbedder {
        entered: mpsc::Sender<()>,
        released: Arc<(Mutex<bool>, Condvar)>,
    }

    impl Embedder for BlockingEmbedder {
        fn dimension(&self) -> usize {
            4
        }

        fn embed(&self, _text: &str) -> Result<Vec<f32>, EmbedError> {
            let _ = self.entered.send(());
            let (released, wake) = &*self.released;
            let mut released = released.lock();
            while !*released {
                wake.wait(&mut released);
            }
            Ok(vec![0.0; 4])
        }
    }

    struct NoopObserver;

    impl MutationObserver for NoopObserver {
        fn before_mutation(&self, _key: DirtyKey) -> Result<(), MemoryError> {
            Ok(())
        }
    }

    const MUTATING: &[&str] = &[
        "delete",
        "relate",
        "store",
        "store_with_metadata",
        "store_with_metadata_and_ttl",
        "store_with_ttl",
        "unrelate",
        "unrelate_from",
        "update_metadata",
    ];
    const READ_ONLY: &[&str] = &[
        "count",
        "edge_count",
        "get",
        "get_metadata",
        "get_metadata_batch",
        "incoming_relations",
        "incoming_relations_bounded",
        "list",
        "query_columnar",
        "query_excluding",
        "query_filtered",
        "relations",
        "relations_bounded",
    ];

    #[test]
    fn memory_store_registry_classifies_every_primitive() {
        let source = include_str!("storage.rs");
        let body = source
            .split_once("pub trait MemoryStore")
            .expect("MemoryStore trait")
            .1
            .split_once("\n}\n")
            .expect("MemoryStore trait body")
            .0;
        let actual: BTreeSet<&str> = body.lines().filter_map(method_name).collect();
        let classified: BTreeSet<&str> = MUTATING.iter().chain(READ_ONLY).copied().collect();

        assert_eq!(actual, classified);
        assert!(MUTATING.iter().all(|name| !READ_ONLY.contains(name)));
    }

    #[test]
    fn exclusive_activation_waits_for_an_in_flight_read_request() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (entered_tx, entered_rx) = mpsc::channel();
        let released = Arc::new((Mutex::new(false), Condvar::new()));
        let service = Arc::new(
            MemoryService::open(
                dir.path(),
                BlockingEmbedder {
                    entered: entered_tx,
                    released: Arc::clone(&released),
                },
            )
            .expect("open service"),
        );
        let reader_service = Arc::clone(&service);
        let reader = std::thread::spawn(move || reader_service.recall("query", 1, None));
        entered_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("read reached embedder");
        let activation_service = Arc::clone(&service);
        let (activated_tx, activated_rx) = mpsc::channel();
        let activation = std::thread::spawn(move || {
            activation_service
                .install_mutation_observer(Some(Arc::new(NoopObserver)))
                .expect("activate observer");
            let _ = activated_tx.send(());
        });

        assert!(activated_rx
            .recv_timeout(Duration::from_millis(50))
            .is_err());
        *released.0.lock() = true;
        released.1.notify_all();
        reader.join().expect("reader thread").expect("recall");
        activation.join().expect("activation thread");
    }

    #[test]
    fn a_second_capture_epoch_is_refused() {
        let capture = super::MutationCapture::default();
        capture
            .replace(Some(Arc::new(NoopObserver)))
            .expect("first epoch");
        let error = capture
            .replace(Some(Arc::new(NoopObserver)))
            .expect_err("second epoch");
        assert!(error.to_string().contains("already active"), "{error}");
        capture.replace(None).expect("remove observer");
        capture
            .replace(Some(Arc::new(NoopObserver)))
            .expect("replacement after removal");
    }

    fn method_name(line: &str) -> Option<&str> {
        line.strip_prefix("    fn ")?
            .split_once('(')
            .map(|(name, _)| name)
    }
}

#[cfg(all(test, feature = "persistence"))]
mod catchup_tests;

#[cfg(test)]
mod journal_tests;
