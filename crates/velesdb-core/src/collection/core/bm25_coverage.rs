//! A graph's BM25 index on open: loaded where its coverage marker vouches for
//! the snapshot, rebuilt from the payload store where nothing does.
//!
//! A graph's index holds the text of every payload it stores, its nodes
//! without an embedding included. An older binary left those out — and it
//! still opens a data directory this one wrote, since the snapshot keeps the
//! format it reads. There it may store such nodes without indexing them and
//! without a BM25 WAL frame for them, rewriting the snapshot or not. What
//! the index file says is then not enough to trust it.
//!
//! So a graph's snapshot is trusted only through the marker this binary
//! writes beside every graph snapshot it saves
//! ([`Bm25Coverage`]: the fingerprint of that snapshot's bytes, and the
//! payload log position its writer read before serializing the index). A
//! payload stored before that position had its BM25 change in the snapshot
//! or in the WAL after it: `flush_full` holds `text_wal` exclusively from
//! the serialization through the WAL truncate, and every BM25 change holds
//! it shared from its WAL append through its in-memory apply
//! (`Collection::flush_bm25_index`). On open:
//!
//! - a marker for this exact snapshot → the snapshot is loaded, its WAL
//!   replayed, and every payload written to the log since the recorded
//!   position indexed again from what is stored — so a node an older binary
//!   stored after the snapshot is indexed, rewrites and all;
//! - no marker, another snapshot's (an older binary rewrote it), or a
//!   position past the log's end (the log lost its tail) → the index is
//!   rebuilt from the payload store: the WAL, the untrusted snapshot's delta,
//!   is emptied, then the rebuilt snapshot is written, then its marker, each
//!   durable before the next. A crash anywhere in between leaves a snapshot
//!   no marker vouches for, and the next open rebuilds again.
//!
//! Every other kind of collection indexed every payload in every version:
//! its snapshot loads as it is, with no marker.

use std::path::Path;
use std::sync::Arc;

use crate::collection::types::Collection;
use crate::error::Result;
use crate::index::bm25_persistence::{
    load_coverage, save_coverage, save_snapshot, Bm25Coverage, StoredSnapshot,
};
use crate::index::Bm25Index;
use crate::lock_audit::RwLock;
use crate::storage::{LogPayloadStorage, PayloadStorage};

impl Collection {
    /// A graph's BM25 index from its snapshot `stored`: loaded and caught up
    /// where the marker vouches for it, else rebuilt (module doc).
    pub(super) fn load_graph_bm25(
        path: &Path,
        payload_storage: &Arc<RwLock<LogPayloadStorage>>,
        stored: StoredSnapshot,
    ) -> Result<Arc<Bm25Index>> {
        let log_end = payload_storage.read().wal_position();
        let vouched = load_coverage(path).filter(|coverage| {
            coverage.snapshot == stored.fingerprint && coverage.payloads_from <= log_end
        });
        let Some(coverage) = vouched else {
            return Self::rebuild_graph_bm25(path, payload_storage);
        };
        let index = Self::replay_bm25_wal(path, Arc::new(stored.index))?;
        let caught_up = catch_up(&index, &payload_storage.read(), coverage.payloads_from);
        tracing::debug!(
            "BM25 of a graph caught up on {caught_up} payloads written since its snapshot"
        );
        Ok(index)
    }

    /// Rebuilds a graph's BM25 index from the payload store and persists it
    /// with its marker, the WAL emptied first (module doc).
    fn rebuild_graph_bm25(
        path: &Path,
        payload_storage: &Arc<RwLock<LogPayloadStorage>>,
    ) -> Result<Arc<Bm25Index>> {
        let payloads_from = payload_storage.read().wal_position();
        let index = Arc::new(Bm25Index::new());
        Self::rebuild_bm25_index(payload_storage, &index);
        let wal_path = crate::index::bm25_persistence_wal::wal_path_for_bm25(path);
        crate::index::bm25_persistence_wal::wal_truncate(&wal_path)?;
        Self::save_graph_bm25(path, &index, payloads_from)?;
        tracing::info!(
            "BM25 snapshot of a graph not vouched for by its coverage marker; \
             rebuilt from payload storage ({} docs)",
            index.len()
        );
        Ok(index)
    }

    /// Saves a graph's BM25 snapshot, then the marker vouching for it:
    /// `payloads_from` is the payload log position read before `index` was
    /// serialized, so every payload write the snapshot may miss is at or
    /// after it, or in the WAL — `flush_full` calls this holding `text_wal`
    /// exclusively through the WAL truncate, and an open before the
    /// collection serves any write.
    pub(super) fn save_graph_bm25(
        path: &Path,
        index: &Bm25Index,
        payloads_from: u64,
    ) -> Result<()> {
        let snapshot = save_snapshot(path, index)?;
        save_coverage(path, &Bm25Coverage::new(snapshot, payloads_from))
    }
}

/// Indexes again, from what is stored, every payload written to the log at
/// or after `from`; returns how many. A payload without text drops the
/// document its id had.
fn catch_up(index: &Bm25Index, payloads: &LogPayloadStorage, from: u64) -> usize {
    let ids = payloads.ids_written_since(from);
    for &id in &ids {
        index.remove_document(id);
        let text = payloads
            .retrieve(id)
            .ok()
            .flatten()
            .map(|payload| Collection::extract_text_from_payload(&payload))
            .unwrap_or_default();
        if !text.is_empty() {
            index.add_document(id, &text);
        }
    }
    ids.len()
}
