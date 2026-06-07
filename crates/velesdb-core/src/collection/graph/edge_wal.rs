//! Graph edge WAL: append + replay for crash-durable edge mutations.
//!
//! Graph edges persist primarily via the whole-file `edge_store.bin`
//! snapshot written during `flush_secondary_indexes`. Between flushes,
//! `add_edge` / `remove_edge` / `remove_node_edges` mutations would be
//! lost on a crash. This WAL captures those mutations so they can be
//! replayed on the next collection open, on top of the loaded snapshot.
//!
//! The design mirrors the BM25 index WAL (`index/bm25_persistence_wal.rs`):
//! a separate per-collection `edges.wal` file, length-prefixed entries,
//! WAL-append-BEFORE-apply ordering, fsync per append, replay-on-open
//! AFTER loading the snapshot, and truncate-after-snapshot during flush.
//!
//! ## On-disk entry layout (length-prefixed, little-endian)
//!
//! ```text
//! Add:        [u32 body_len][u8 0x01][json(GraphEdge) bytes]
//! Remove:     [u32 body_len][u8 0x02][u64 edge_id]
//! RemoveNode: [u32 body_len][u8 0x03][u64 node_id]
//! ```
//!
//! `body_len` is the byte count *after* the 4-byte prefix — it lets the
//! replay loop skip unknown / corrupt entries without aborting the whole
//! recovery. A truncated final entry (common on crash) is logged at
//! `warn` level and skipped rather than surfacing as an error.
//!
//! ## Crash-safety ordering
//!
//! Callers MUST invoke `wal_append_*` BEFORE applying the corresponding
//! in-memory mutation. If the process crashes between the two, replay
//! reconstructs the mutation on next open. The WAL is fsynced on every
//! append. After a successful snapshot the WAL is truncated via
//! [`wal_truncate`] so the next open replays zero entries.

use std::io::BufWriter;
use std::path::{Path, PathBuf};

use crate::collection::graph::{ConcurrentEdgeStore, GraphEdge};
use crate::error::{Error, Result};
use crate::index::wal_framing;

/// Error-message context prefix for shared framing helpers.
const CTX: &str = "Edge WAL";

const WAL_OP_ADD: u8 = 0x01;
const WAL_OP_REMOVE: u8 = 0x02;
const WAL_OP_REMOVE_NODE: u8 = 0x03;

/// WAL filename under a collection directory.
const EDGE_WAL_FILENAME: &str = "edges.wal";

/// Body length for a Remove / RemoveNode entry: `op(1)` + `id(8)`.
const ID_ENTRY_BODY_LEN: usize = 1 + 8;

/// Returns the absolute path to the edge WAL file under `dir`.
#[must_use]
pub(crate) fn wal_path_for_edges(dir: &Path) -> PathBuf {
    dir.join(EDGE_WAL_FILENAME)
}

// ---------------------------------------------------------------------------
// Append operations
// ---------------------------------------------------------------------------

/// Appends an `add_edge(edge)` mutation to the edge WAL.
///
/// Callers MUST invoke this BEFORE applying the mutation in-memory
/// (WAL-before-apply crash-safety ordering).
///
/// # Errors
///
/// Returns [`Error::Index`] if the edge cannot be serialized or the WAL
/// file cannot be opened / written.
pub(crate) fn wal_append_add(wal_path: &Path, edge: &GraphEdge) -> Result<()> {
    let mut w = wal_framing::open_wal_writer(wal_path, CTX)?;
    write_add_entry(&mut w, edge)?;
    wal_framing::flush_wal(&mut w, CTX)
}

/// Appends multiple `add_edge` mutations to the edge WAL with a single
/// open + fsync. Used by the batch insert path.
///
/// # Errors
///
/// Returns [`Error::Index`] if any edge cannot be serialized or the WAL
/// file cannot be opened / written.
pub(crate) fn wal_append_add_batch(wal_path: &Path, edges: &[GraphEdge]) -> Result<()> {
    if edges.is_empty() {
        return Ok(());
    }
    let mut w = wal_framing::open_wal_writer(wal_path, CTX)?;
    for edge in edges {
        write_add_entry(&mut w, edge)?;
    }
    wal_framing::flush_wal(&mut w, CTX)
}

/// Serializes `edge` and writes a length-prefixed ADD entry.
///
/// The body uses JSON (`serde_json`) rather than postcard: `GraphEdge`
/// properties are `serde_json::Value`, whose `Deserialize` relies on
/// `deserialize_any`, which postcard (a non-self-describing format) does
/// not support — postcard round-trips silently drop / corrupt property
/// values. JSON is self-describing and round-trips `Value` losslessly.
fn write_add_entry(w: &mut BufWriter<std::fs::File>, edge: &GraphEdge) -> Result<()> {
    let edge_bytes = serde_json::to_vec(edge)
        .map_err(|e| Error::Index(format!("Edge WAL: serialize edge: {e}")))?;
    let body_len = add_entry_body_len(edge_bytes.len())?;
    wal_framing::wal_write(w, &body_len.to_le_bytes(), CTX)?;
    wal_framing::wal_write(w, &[WAL_OP_ADD], CTX)?;
    wal_framing::wal_write(w, &edge_bytes, CTX)
}

/// Computes the `u32` body length for an ADD entry = `op(1)` + edge bytes.
/// Fails if the sum overflows `u32`.
fn add_entry_body_len(edge_len: usize) -> Result<u32> {
    let total = edge_len
        .checked_add(1)
        .ok_or_else(|| Error::Index("Edge WAL: add entry length overflow".to_string()))?;
    u32::try_from(total)
        .map_err(|_| Error::Index(format!("Edge WAL: add entry too large ({total} bytes)")))
}

/// Appends a `remove_edge(edge_id)` mutation to the edge WAL.
///
/// # Errors
///
/// Returns [`Error::Index`] if the WAL file cannot be opened / written.
pub(crate) fn wal_append_remove(wal_path: &Path, edge_id: u64) -> Result<()> {
    append_id_entry(wal_path, WAL_OP_REMOVE, edge_id)
}

/// Appends a `remove_node_edges(node_id)` mutation to the edge WAL.
///
/// # Errors
///
/// Returns [`Error::Index`] if the WAL file cannot be opened / written.
pub(crate) fn wal_append_remove_node(wal_path: &Path, node_id: u64) -> Result<()> {
    append_id_entry(wal_path, WAL_OP_REMOVE_NODE, node_id)
}

/// Shared writer for the two single-`u64` opcodes (Remove / RemoveNode).
fn append_id_entry(wal_path: &Path, op: u8, id: u64) -> Result<()> {
    let body_len = u32::try_from(ID_ENTRY_BODY_LEN)
        .map_err(|_| Error::Index("Edge WAL: id entry header too large".to_string()))?;
    let mut w = wal_framing::open_wal_writer(wal_path, CTX)?;
    wal_framing::wal_write(&mut w, &body_len.to_le_bytes(), CTX)?;
    wal_framing::wal_write(&mut w, &[op], CTX)?;
    wal_framing::wal_write(&mut w, &id.to_le_bytes(), CTX)?;
    wal_framing::flush_wal(&mut w, CTX)
}

/// Truncates the edge WAL file to zero length.
///
/// Called after a successful snapshot to guarantee that the next open
/// replays zero WAL entries. A missing WAL file is a no-op.
///
/// # Errors
///
/// Returns [`Error::Index`] if the WAL file exists but cannot be truncated.
pub(crate) fn wal_truncate(wal_path: &Path) -> Result<()> {
    wal_framing::wal_truncate(wal_path, CTX)
}

// ---------------------------------------------------------------------------
// Replay
// ---------------------------------------------------------------------------

/// A single decoded WAL mutation, surfaced to the caller so that
/// collection-level side effects (e.g. edge-property reindexing) can run
/// alongside the edge-store mutation.
pub(crate) enum ReplayOp {
    /// An edge was (re)added; carries the edge for property reindexing.
    Add(GraphEdge),
    /// An edge was removed by id.
    Remove(u64),
    /// All edges around a node were removed.
    RemoveNode(u64),
}

/// Replays the edge WAL against `store`, invoking `on_op` for each applied
/// mutation, and returns the number of entries applied.
///
/// Missing WAL file returns `Ok(0)` (full back-compat for pre-feature DBs).
/// A truncated final entry (partial crash during append) is logged at
/// `warn` and skipped. Unknown opcodes and undecodable bodies are logged
/// and skipped without aborting replay.
///
/// ADD replays are idempotent: an `Error::EdgeExists` from the snapshot
/// already containing the edge is ignored.
///
/// # Errors
///
/// Returns [`Error::Index`] if the WAL file exists but cannot be read.
pub(crate) fn wal_replay<F>(
    wal_path: &Path,
    store: &ConcurrentEdgeStore,
    mut on_op: F,
) -> Result<u64>
where
    F: FnMut(&ReplayOp),
{
    let data = match std::fs::read(wal_path) {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(Error::Index(format!("Edge WAL read: {e}"))),
    };

    let mut pos = 0usize;
    let mut count = 0u64;

    while pos < data.len() {
        let Some((body_start, body_len)) = read_entry_header(&data, pos) else {
            break;
        };
        if body_start + body_len > data.len() {
            tracing::warn!(
                "Edge WAL truncated at offset {body_start}: declared {body_len} bytes but only {} remain",
                data.len() - body_start
            );
            break;
        }
        let op = data[body_start];
        let body = &data[body_start + 1..body_start + body_len];
        if let Some(replay_op) = decode_entry(op, body, body_start) {
            apply_replay_op(store, &replay_op);
            on_op(&replay_op);
            count += 1;
        }
        pos = body_start + body_len;
    }

    Ok(count)
}

/// Reads the `u32` length prefix via the shared framing helper, rejecting
/// a zero-length body (a torn / corrupt prefix) so replay does not spin.
fn read_entry_header(data: &[u8], pos: usize) -> Option<(usize, usize)> {
    let (body_start, body_len) = wal_framing::read_entry_header(data, pos, CTX)?;
    if body_len == 0 {
        tracing::warn!("Edge WAL zero-length entry at offset {pos}");
        return None;
    }
    Some((body_start, body_len))
}

/// Decodes a single WAL entry body into a [`ReplayOp`], or `None` if the
/// opcode is unknown or the body cannot be decoded (logged + skipped).
fn decode_entry(op: u8, body: &[u8], body_start: usize) -> Option<ReplayOp> {
    match op {
        WAL_OP_ADD => decode_add(body, body_start),
        WAL_OP_REMOVE => decode_id(body, body_start).map(ReplayOp::Remove),
        WAL_OP_REMOVE_NODE => decode_id(body, body_start).map(ReplayOp::RemoveNode),
        unknown => {
            tracing::warn!("Edge WAL unknown op 0x{unknown:02x} at offset {body_start}");
            None
        }
    }
}

/// Decodes an ADD body (JSON-serialized `GraphEdge`).
fn decode_add(body: &[u8], body_start: usize) -> Option<ReplayOp> {
    match serde_json::from_slice::<GraphEdge>(body) {
        Ok(edge) => Some(ReplayOp::Add(edge)),
        Err(e) => {
            tracing::warn!("Edge WAL undecodable add entry at offset {body_start}: {e}");
            None
        }
    }
}

/// Decodes a single-`u64` body (Remove / RemoveNode payload).
fn decode_id(body: &[u8], body_start: usize) -> Option<u64> {
    let Ok(bytes) = <[u8; 8]>::try_from(body) else {
        tracing::warn!("Edge WAL malformed id entry at offset {body_start}");
        return None;
    };
    Some(u64::from_le_bytes(bytes))
}

/// Applies a decoded [`ReplayOp`] to the edge store.
///
/// ADD is idempotent: `Error::EdgeExists` (the snapshot already holds the
/// edge) is treated as a successful no-op.
fn apply_replay_op(store: &ConcurrentEdgeStore, op: &ReplayOp) {
    match op {
        ReplayOp::Add(edge) => match store.add_edge(edge.clone()) {
            Ok(()) | Err(Error::EdgeExists(_)) => {}
            Err(e) => tracing::warn!("Edge WAL replay add failed for edge {}: {e}", edge.id()),
        },
        ReplayOp::Remove(id) => {
            store.remove_edge(*id);
        }
        ReplayOp::RemoveNode(id) => store.remove_node_edges(*id),
    }
}
