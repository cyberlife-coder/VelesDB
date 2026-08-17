use std::path::Path;
use std::sync::Arc;

use super::catchup::{CatchUpConfig, OnlineCatchUp};
use super::journal::{DirtyJournal, EpochIdentity};
use crate::storage::NativeStore;
use crate::{EmbedError, Embedder, MemoryService};

#[path = "catchup_tests/base_tests.rs"]
mod base;
#[path = "catchup_tests/faults_tests.rs"]
mod faults;
#[path = "catchup_tests/replay_tests.rs"]
mod replay;

pub(super) struct FixedEmbedder {
    vector: Vec<f32>,
}

impl Embedder for FixedEmbedder {
    fn dimension(&self) -> usize {
        self.vector.len()
    }

    fn embed(&self, _text: &str) -> Result<Vec<f32>, EmbedError> {
        Ok(self.vector.clone())
    }
}

pub(super) struct TestRig {
    pub(super) source: MemoryService<FixedEmbedder>,
    pub(super) destination: NativeStore,
    pub(super) target: FixedEmbedder,
    pub(super) journal: Arc<DirtyJournal>,
    pub(super) config: CatchUpConfig,
    _root: tempfile::TempDir,
}

impl TestRig {
    pub(super) fn new() -> Self {
        let root = tempfile::tempdir().expect("root");
        let source_path = root.path().join("source");
        let destination_path = root.path().join("destination");
        let source = MemoryService::open(
            &source_path,
            FixedEmbedder {
                vector: vec![1.0, 2.0],
            },
        )
        .expect("source");
        let destination = NativeStore::open(&destination_path, 3).expect("destination");
        let journal = journal(
            &root.path().join("journal"),
            &source_path,
            &destination_path,
        );
        Self {
            source,
            destination,
            target: FixedEmbedder {
                vector: vec![7.0, 8.0, 9.0],
            },
            journal,
            config: CatchUpConfig {
                fact_batch: 8,
                replay_batch: 8,
                edge_cap: 8,
            },
            _root: root,
        }
    }

    pub(super) fn start(&self) -> OnlineCatchUp<'_, FixedEmbedder> {
        OnlineCatchUp::start(
            &self.source,
            &self.destination,
            &self.target,
            Arc::clone(&self.journal),
            self.config,
        )
        .expect("start")
    }
}

pub(super) fn journal(workspace: &Path, source: &Path, destination: &Path) -> Arc<DirtyJournal> {
    std::fs::create_dir_all(workspace).expect("journal workspace");
    let identity = EpochIdentity::for_test(
        source.to_owned(),
        "sha256:source",
        "target-model",
        3,
        "sha256:0000000000000000000000000000000000000000000000000000000000000000",
        destination.to_owned(),
        "00112233445566778899aabbccddeeff",
    );
    Arc::new(DirtyJournal::open(workspace, &identity, 1024 * 1024).expect("journal"))
}
