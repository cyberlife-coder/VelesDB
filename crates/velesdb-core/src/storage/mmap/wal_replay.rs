//! WAL replay for `MmapStorage` crash recovery.
//!
//! Issue #317: On crash, WAL data written since the last `flush()` would be
//! lost because the index file (`vectors.idx`) only reflects the last
//! flushed state. This module replays WAL entries to recover those writes.
//!
//! # WAL Format (CRC32-framed, Issue #317)
//!
//! ```text
//! Store:  [op=1: 1B] [id: 8B LE] [len: 4B LE] [data: len B] [crc32: 4B LE]
//! Delete: [op=2: 1B] [id: 8B LE] [crc32: 4B LE]
//! ```
//!
//! # Legacy WAL Format (pre-#317, no CRC)
//!
//! Detected by validating the CRC of the first entry. If validation fails,
//! the file is legacy format and replay is skipped (the persisted index is
//! authoritative for legacy data).
//!
//! # Corruption policy (#898)
//!
//! Replay distinguishes two failure shapes:
//!
//! - **Torn tail** — the last record is short or its declared length runs past
//!   EOF because the process crashed mid-append. This is expected after a
//!   crash, so replay stops cleanly and keeps every prior entry.
//! - **Mid-stream corruption** — a fully-framed record fails CRC. This
//!   indicates bit-rot or a malformed WAL; the entry is skipped, a metric and
//!   warning are recorded, and replay continues so later valid entries are
//!   still recovered. Unknown opcodes mid-stream stop replay (the framing can
//!   no longer be trusted).
//!
//! A bare `0x04` byte is the legacy compaction marker: versions prior to the
//! WAL-truncating compaction protocol appended it after a successful
//! compaction. It carries no payload and is skipped so post-compaction
//! entries written by those versions are still recovered.

use crate::storage::log_payload::crc32_hash;
use crate::storage::sharded_index::ShardedIndex;

use memmap2::MmapMut;
use std::fs::{File, OpenOptions};
use std::io::{self, BufReader, Read, Seek};
use std::path::Path;

/// Minimum store entry size: op(1) + id(8) + len(4) + crc(4) = 17.
const MIN_STORE_ENTRY: u64 = 17;
/// Delete entry size: op(1) + id(8) + crc(4) = 13.
const DELETE_ENTRY_SIZE: u64 = 13;

/// Outcome of attempting to replay a single WAL entry.
enum EntryOutcome {
    /// Entry applied (or intentionally skipped on mid-stream corruption); keep going.
    Applied,
    /// Clean stop: EOF or a torn tail record left by a crash.
    Stop,
}

impl EntryOutcome {
    /// Returns `true` while replay should continue.
    const fn should_continue(&self) -> bool {
        matches!(self, Self::Applied)
    }
}

/// Grows the mmap during replay (mirroring the live `ensure_capacity` path) so
/// recovered vectors that extend past the last flushed size are not dropped.
struct ReplayTarget<'a> {
    mmap: &'a mut MmapMut,
    data_file: &'a File,
}

impl ReplayTarget<'_> {
    /// Ensures the mapping covers at least `required_len` bytes, growing the
    /// backing file and remapping if necessary.
    fn ensure_capacity(&mut self, required_len: usize) -> io::Result<()> {
        if self.mmap.len() >= required_len {
            return Ok(());
        }
        self.mmap.flush()?;
        let required_u64 = u64::try_from(required_len)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "replay length overflow"))?;
        // Match the live growth floor (64 MB) to amortize remaps during replay.
        let new_len = required_u64.saturating_add(super::MmapStorage::MIN_GROWTH);
        self.data_file.set_len(new_len)?;
        // SAFETY: `set_len(new_len)` resized the backing file to fully cover the
        // mapping range before remapping; the old mapping is dropped on assign.
        // - Condition 1: file resized to `new_len` immediately above.
        // - Condition 2: `data_file` is the read+write handle owning the file.
        // SAFETY: remapping requires unsafe; resize guarantees mapping <= file size.
        *self.mmap = unsafe { MmapMut::map_mut(self.data_file)? };
        Ok(())
    }
}

/// Replays CRC32-framed WAL entries into the sharded index and mmap.
///
/// Skips legacy (non-CRC) WAL files. The WAL is **not** truncated here: the
/// caller must first flush the recovered mmap and persist the index, then call
/// [`truncate_wal`]. Truncating before the mmap is durable would open a
/// data-loss window (#898).
///
/// Every id touched by an applied store or delete entry is appended to
/// `touched_ids` (duplicates included) BEFORE the caller truncates the WAL,
/// so the HNSW open-time reconciliation can detect stale index entries whose
/// only witness was the just-truncated WAL.
///
/// # Returns
///
/// Number of WAL entries successfully replayed.
#[allow(clippy::module_name_repetitions)]
pub(crate) fn replay_wal_to_index(
    wal_path: &Path,
    index: &ShardedIndex,
    dimension: usize,
    mmap: &mut MmapMut,
    data_file: &File,
    next_offset: &mut usize,
    touched_ids: &mut Vec<u64>,
) -> io::Result<usize> {
    let Some((mut reader, file_len)) = open_crc_wal(wal_path)? else {
        return Ok(0);
    };

    let vector_size = dimension * std::mem::size_of::<f32>();
    let mut target = ReplayTarget { mmap, data_file };
    drain_wal_entries(
        &mut reader,
        file_len,
        index,
        &mut target,
        next_offset,
        vector_size,
        touched_ids,
    )
}

/// Truncates the WAL after the recovered state has been made durable.
///
/// Must only be called once the mmap has been flushed and the index persisted,
/// otherwise a crash between truncation and flush loses the replayed writes.
pub(crate) fn truncate_wal(wal_path: &Path) -> io::Result<()> {
    let file = OpenOptions::new().write(true).open(wal_path)?;
    file.set_len(0)?;
    file.sync_all()
}

/// Opens the WAL file and validates it uses CRC32-framed format.
///
/// Returns `None` if the file is missing, empty, or uses the legacy format.
fn open_crc_wal(wal_path: &Path) -> io::Result<Option<(BufReader<File>, u64)>> {
    if !wal_path.exists() {
        return Ok(None);
    }

    let file = File::open(wal_path)?;
    let file_len = file.metadata()?.len();
    if file_len == 0 {
        return Ok(None);
    }

    if !is_crc_framed_wal(wal_path, file_len)? {
        return Ok(None);
    }

    let reader = BufReader::new(File::open(wal_path)?);
    Ok(Some((reader, file_len)))
}

/// Replays all valid entries from the WAL, returning the count.
///
/// Returns an error only for unrecoverable I/O failures; torn tail records (a
/// crash mid-append) stop replay cleanly with the entries seen so far.
#[allow(clippy::too_many_arguments)] // Mirrors the WAL entry frame: every field is required.
fn drain_wal_entries(
    reader: &mut BufReader<File>,
    file_len: u64,
    index: &ShardedIndex,
    target: &mut ReplayTarget<'_>,
    next_offset: &mut usize,
    vector_size: usize,
    touched_ids: &mut Vec<u64>,
) -> io::Result<usize> {
    let mut replayed = 0usize;
    while replay_one_entry(
        reader,
        file_len,
        index,
        target,
        next_offset,
        vector_size,
        touched_ids,
    )?
    .should_continue()
    {
        replayed += 1;
    }
    Ok(replayed)
}

// ---------------------------------------------------------------------------
// Format Detection
// ---------------------------------------------------------------------------

/// Returns `true` if the WAL uses CRC32-framed entries.
fn is_crc_framed_wal(wal_path: &Path, file_len: u64) -> io::Result<bool> {
    let min_size = MIN_STORE_ENTRY.min(DELETE_ENTRY_SIZE);
    if file_len < min_size {
        return Ok(false);
    }

    let mut reader = BufReader::new(File::open(wal_path)?);
    let mut op = [0u8; 1];
    // Skip leading legacy compaction markers (bare 0x04 bytes) so a WAL whose
    // first real record sits behind a marker is still detected as CRC-framed.
    loop {
        if reader.read_exact(&mut op).is_err() {
            return Ok(false);
        }
        if op[0] != 4 {
            break;
        }
    }

    match op[0] {
        1 => validate_first_store_crc(&mut reader, file_len),
        2 => Ok(validate_first_delete_crc(&mut reader)),
        _ => Ok(false),
    }
}

/// Validates the CRC of the first store entry.
fn validate_first_store_crc(reader: &mut BufReader<File>, file_len: u64) -> io::Result<bool> {
    let mut id_bytes = [0u8; 8];
    let mut len_bytes = [0u8; 4];
    if reader.read_exact(&mut id_bytes).is_err() || reader.read_exact(&mut len_bytes).is_err() {
        return Ok(false);
    }

    // OOM guard (#897/#898): cap the declared length against the bytes that can
    // actually follow in the file before allocating.
    let Some(data_len) = checked_store_data_len(len_bytes, reader, file_len)? else {
        return Ok(false);
    };

    let mut data = vec![0u8; data_len];
    if reader.read_exact(&mut data).is_err() {
        return Ok(false);
    }

    let mut stored_crc = [0u8; 4];
    if reader.read_exact(&mut stored_crc).is_err() {
        return Ok(false);
    }

    Ok(store_crc_matches(id_bytes, len_bytes, &data, stored_crc))
}

/// Validates the CRC of the first delete entry.
fn validate_first_delete_crc(reader: &mut BufReader<File>) -> bool {
    let mut id_bytes = [0u8; 8];
    if reader.read_exact(&mut id_bytes).is_err() {
        return false;
    }

    let mut stored_crc = [0u8; 4];
    if reader.read_exact(&mut stored_crc).is_err() {
        return false;
    }

    delete_frame_crc(id_bytes) == u32::from_le_bytes(stored_crc)
}

// ---------------------------------------------------------------------------
// Entry Replay
// ---------------------------------------------------------------------------

/// Replays one WAL entry.
#[allow(clippy::too_many_arguments)] // Mirrors the WAL entry frame: every field is required.
fn replay_one_entry(
    reader: &mut BufReader<File>,
    file_len: u64,
    index: &ShardedIndex,
    target: &mut ReplayTarget<'_>,
    next_offset: &mut usize,
    vector_size: usize,
    touched_ids: &mut Vec<u64>,
) -> io::Result<EntryOutcome> {
    let pos = reader.stream_position()?;
    if pos >= file_len {
        return Ok(EntryOutcome::Stop);
    }

    let mut op = [0u8; 1];
    if reader.read_exact(&mut op).is_err() {
        return Ok(EntryOutcome::Stop);
    }

    match op[0] {
        1 => replay_store(
            reader,
            file_len,
            index,
            target,
            next_offset,
            vector_size,
            touched_ids,
        ),
        2 => replay_delete(reader, file_len, index, touched_ids),
        // Legacy compaction marker (no payload): written by pre-WAL-truncation
        // versions after a successful compaction. Skip it and keep replaying so
        // post-compaction entries are recovered; replaying the entries BEFORE
        // the marker is convergent against the index those versions persisted
        // (store records carry the full vector value, deletes replay in order).
        4 => Ok(EntryOutcome::Applied),
        // Unknown opcode: framing is no longer trustworthy, so stop cleanly. If
        // bytes still follow the opcode this is mid-stream corruption — record
        // it for visibility, mirroring the CRC path; a trailing partial byte is
        // just a torn tail and stays silent.
        _ => {
            if reader.stream_position()? < file_len {
                crate::metrics::global_guardrails_metrics().record_wal_replay_corrupt_entry();
            }
            Ok(EntryOutcome::Stop)
        }
    }
}

/// Reads a store entry, returning `None` for a torn tail (short/truncated)
/// record and `Some((id, data, crc_ok))` for a fully-framed record.
fn read_store_entry(
    reader: &mut BufReader<File>,
    file_len: u64,
) -> io::Result<Option<(u64, Vec<u8>, bool)>> {
    let mut id_bytes = [0u8; 8];
    let mut len_bytes = [0u8; 4];
    if reader.read_exact(&mut id_bytes).is_err() || reader.read_exact(&mut len_bytes).is_err() {
        return Ok(None);
    }

    // OOM guard (#897/#898): reject a declared length larger than the remaining
    // file before allocating `data`.
    let Some(data_len) = checked_store_data_len(len_bytes, reader, file_len)? else {
        return Ok(None);
    };

    let mut data = vec![0u8; data_len];
    if reader.read_exact(&mut data).is_err() {
        return Ok(None);
    }

    let mut stored_crc = [0u8; 4];
    if reader.read_exact(&mut stored_crc).is_err() {
        return Ok(None);
    }

    let crc_ok = store_crc_matches(id_bytes, len_bytes, &data, stored_crc);
    Ok(Some((u64::from_le_bytes(id_bytes), data, crc_ok)))
}

/// Validates a store entry's declared payload length against the bytes that can
/// physically follow it in the file.
///
/// Returns `Ok(None)` when the record is a torn tail (declared length plus the
/// trailing CRC would run past EOF), so the caller can stop replay cleanly
/// instead of allocating an oversized buffer.
fn checked_store_data_len(
    len_bytes: [u8; 4],
    reader: &mut BufReader<File>,
    file_len: u64,
) -> io::Result<Option<usize>> {
    let data_len = u64::from(u32::from_le_bytes(len_bytes));
    let pos = reader.stream_position()?;
    // Remaining bytes after the length field; the record needs `data_len + 4`
    // (payload + CRC). If it would overrun the file it is a torn/corrupt tail.
    let remaining = file_len.saturating_sub(pos);
    if data_len.saturating_add(4) > remaining {
        return Ok(None);
    }
    let data_len = usize::try_from(data_len)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "data_len overflow"))?;
    Ok(Some(data_len))
}

/// Returns `true` when the reader sits at (or past) EOF, i.e. the record just
/// consumed was the last one in the file.
///
/// Used to tell a normal post-crash torn tail (CRC-failing record at EOF, no
/// bytes after it) apart from genuine mid-stream corruption (a CRC failure
/// followed by validly framed records) so torn tails do not raise bit-rot
/// alerts (#898 follow-up).
fn is_at_tail(reader: &mut BufReader<File>, file_len: u64) -> io::Result<bool> {
    Ok(reader.stream_position()? >= file_len)
}

/// Computes the CRC32 of a delete frame `[op=2, id...]` using a stack buffer —
/// avoids a heap allocation for the single-use 9-byte frame.
#[inline]
fn delete_frame_crc(id_bytes: [u8; 8]) -> u32 {
    let mut frame = [0u8; 9];
    frame[0] = 2u8;
    frame[1..].copy_from_slice(&id_bytes);
    crc32_hash(&frame)
}

/// Computes the CRC32 of a store frame and compares it with `stored_crc`.
fn store_crc_matches(
    id_bytes: [u8; 8],
    len_bytes: [u8; 4],
    data: &[u8],
    stored_crc: [u8; 4],
) -> bool {
    let mut frame = Vec::with_capacity(1 + 8 + 4 + data.len());
    frame.push(1u8);
    frame.extend_from_slice(&id_bytes);
    frame.extend_from_slice(&len_bytes);
    frame.extend_from_slice(data);
    crc32_hash(&frame) == u32::from_le_bytes(stored_crc)
}

/// Replays a store entry: validates CRC, writes data to mmap, updates index.
#[allow(clippy::too_many_arguments)] // Mirrors the WAL entry frame: every field is required.
fn replay_store(
    reader: &mut BufReader<File>,
    file_len: u64,
    index: &ShardedIndex,
    target: &mut ReplayTarget<'_>,
    next_offset: &mut usize,
    vector_size: usize,
    touched_ids: &mut Vec<u64>,
) -> io::Result<EntryOutcome> {
    let Some((id, data, crc_ok)) = read_store_entry(reader, file_len)? else {
        // Torn tail: stop cleanly, keeping prior entries.
        return Ok(EntryOutcome::Stop);
    };

    if !crc_ok {
        // #898 follow-up: distinguish a torn tail from genuine mid-stream
        // corruption. A fully-framed-but-CRC-failing record that is the LAST
        // record (no bytes remain after it) is the normal post-crash torn tail
        // and must NOT raise a bit-rot alert: stop cleanly, no metric. Only a
        // CRC failure with valid framing AFTER it is true mid-stream corruption.
        if is_at_tail(reader, file_len)? {
            return Ok(EntryOutcome::Stop);
        }
        crate::metrics::global_guardrails_metrics().record_wal_replay_corrupt_entry();
        tracing::warn!(id, "WAL replay: skipping mid-stream corrupt store entry");
        return Ok(EntryOutcome::Applied);
    }

    if data.len() == vector_size {
        apply_store_to_mmap(id, &data, index, target, next_offset, vector_size)?;
        touched_ids.push(id);
    }

    Ok(EntryOutcome::Applied)
}

/// Writes vector data into the mmap region and updates the index.
///
/// Grows the mmap when the recovered vector extends past the current mapping
/// (#898), so replayed writes are never silently dropped, and only advances
/// `next_offset` after the bounds-checked write succeeds.
fn apply_store_to_mmap(
    id: u64,
    data: &[u8],
    index: &ShardedIndex,
    target: &mut ReplayTarget<'_>,
    next_offset: &mut usize,
    vector_size: usize,
) -> io::Result<()> {
    let offset = index.get(id).unwrap_or(*next_offset);

    // Bounds check BEFORE advancing `next_offset` (#898): grow the mapping so
    // the write fits, then commit the offset advance only on success.
    let end = offset
        .checked_add(vector_size)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "WAL replay offset overflow"))?;
    target.ensure_capacity(end)?;

    target.mmap[offset..end].copy_from_slice(data);
    index.insert(id, offset);
    if offset == *next_offset {
        *next_offset = end;
    }
    Ok(())
}

/// Replays a delete entry: validates CRC, removes id from index.
fn replay_delete(
    reader: &mut BufReader<File>,
    file_len: u64,
    index: &ShardedIndex,
    touched_ids: &mut Vec<u64>,
) -> io::Result<EntryOutcome> {
    // op(1) already consumed; a delete needs id(8) + crc(4) to follow.
    let pos = reader.stream_position()?;
    if file_len.saturating_sub(pos) < 8 + 4 {
        return Ok(EntryOutcome::Stop);
    }

    let mut id_bytes = [0u8; 8];
    if reader.read_exact(&mut id_bytes).is_err() {
        return Ok(EntryOutcome::Stop);
    }

    let mut stored_crc = [0u8; 4];
    if reader.read_exact(&mut stored_crc).is_err() {
        return Ok(EntryOutcome::Stop);
    }

    if delete_frame_crc(id_bytes) == u32::from_le_bytes(stored_crc) {
        let id = u64::from_le_bytes(id_bytes);
        index.remove(id);
        touched_ids.push(id);
        return Ok(EntryOutcome::Applied);
    }

    // #898 follow-up: a CRC-failing delete that is the LAST record is a torn
    // tail (stop cleanly, no metric); only a CRC failure with valid framing
    // after it is genuine mid-stream corruption.
    if is_at_tail(reader, file_len)? {
        return Ok(EntryOutcome::Stop);
    }
    crate::metrics::global_guardrails_metrics().record_wal_replay_corrupt_entry();
    tracing::warn!("WAL replay: skipping mid-stream corrupt delete entry");
    Ok(EntryOutcome::Applied)
}
