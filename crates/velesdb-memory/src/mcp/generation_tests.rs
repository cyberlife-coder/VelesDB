use std::sync::{mpsc, Arc};
use std::time::Duration;

use parking_lot::{Condvar, Mutex};
use rmcp::handler::server::wrapper::Parameters;

use super::dto::RecallParams;
use super::McpServer;
use crate::{DynEmbedder, EmbedError, Embedder, HashEmbedder, MemoryService};

struct BlockingEmbedder {
    entered: mpsc::Sender<()>,
    release: Arc<(Mutex<bool>, Condvar)>,
}

impl Embedder for BlockingEmbedder {
    fn dimension(&self) -> usize {
        2
    }

    fn embed(&self, _text: &str) -> Result<Vec<f32>, EmbedError> {
        self.entered.send(()).expect("entered");
        let mut released = self.release.0.lock();
        while !*released {
            self.release.1.wait(&mut released);
        }
        Ok(vec![0.0; 2])
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mcp_request_holds_the_live_generation_until_completion() {
    let root = tempfile::tempdir().expect("root");
    let (entered_tx, entered_rx) = mpsc::channel();
    let release = Arc::new((Mutex::new(false), Condvar::new()));
    let source_embedder: DynEmbedder = Box::new(BlockingEmbedder {
        entered: entered_tx,
        release: Arc::clone(&release),
    });
    let source = MemoryService::open(root.path().join("source"), source_embedder).expect("source");
    let server = Arc::new(McpServer::new(source));
    let reader_server = Arc::clone(&server);
    let reader = tokio::spawn(async move {
        reader_server
            .recall(Parameters(RecallParams {
                query: "blocked".to_owned(),
                limit: Some(1),
                filter: None,
            }))
            .await
    });
    entered_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("request entered embedder");

    let target_embedder: DynEmbedder = Box::new(HashEmbedder::new(3));
    let target = MemoryService::open(root.path().join("target"), target_embedder).expect("target");
    let writer_server = Arc::clone(&server);
    let (replaced_tx, replaced_rx) = mpsc::channel();
    let writer = std::thread::spawn(move || {
        writer_server
            .service
            .replace_for_test(target, "target-model");
        replaced_tx.send(()).expect("replaced");
    });

    assert!(replaced_rx.recv_timeout(Duration::from_millis(50)).is_err());
    *release.0.lock() = true;
    release.1.notify_all();
    reader.await.expect("reader task").expect("recall");
    writer.join().expect("writer");
    server
        .service
        .with_generation(|generation| assert_eq!(generation.dimension(), 3))
        .expect("target generation");
}
