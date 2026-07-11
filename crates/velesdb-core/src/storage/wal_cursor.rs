//! Shippable WAL cursor: an additive, read-only API over the existing
//! append-only WAL framing.
//!
//! # Purpose
//!
//! Replication consumers (e.g. premium clustering / Raft) need to identify a
//! stable position in the write-ahead log and stream entries forward from it,
//! resuming after a restart without core silently discarding a position they
//! still need. This module defines the *seam* for that capability:
//!
//! - [`WalPosition`] — an opaque, comparable byte-offset into the log.
//! - [`WalRecord`] — one framed record plus the cursor immediately after it.
//! - [`WalConsumerId`] — an opaque replication-consumer identity.
//! - [`WalCursor`] — the read-only, forward-only trait consumers drive.
//!
//! # On-disk format
//!
//! This is an **additive read API** over the existing per-entry marker/CRC
//! framing in [`super::wal_entry`] and [`super::log_payload`]. It introduces
//! **no** on-disk format change: existing stored WALs remain readable, and no
//! new header or field is written. Consumer watermarks are in-memory
//! registration state, never a new on-disk artifact (Requirement 6.4).
//!
//! # Boundary
//!
//! Core exposes this trait; premium consumes it. Core never depends on premium
//! (Requirement 6.5).

use parking_lot::Mutex;
use rustc_hash::FxHashMap;
use std::sync::atomic::{AtomicU64, Ordering};

/// A stable, monotonic position within the WAL.
///
/// Opaque byte-offset into the append-only log. It is `Copy` and totally
/// ordered so a consumer can persist it and resume reading from exactly where
/// it left off (Requirement 6.2). Positions only ever increase in append
/// order; the `Ord` impl reflects that append order directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WalPosition(u64);

impl WalPosition {
    /// The beginning of the log — where a fresh consumer starts reading.
    pub const START: WalPosition = WalPosition(0);

    /// Constructs a position at the given byte offset.
    #[must_use]
    pub const fn new(offset: u64) -> Self {
        Self(offset)
    }

    /// The raw byte offset of this position into the append-only log.
    #[must_use]
    pub const fn offset(self) -> u64 {
        self.0
    }
}

/// One shippable WAL record.
///
/// Carries its own [`WalPosition`], the position immediately after it (the
/// consumer's `next` cursor), and the raw framed bytes exactly as stored on
/// disk — marker, id, and (for v2 records) length/payload/CRC. Consumers ship
/// `bytes` verbatim and resume from `next`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalRecord {
    /// Position of this record's first framed byte.
    pub position: WalPosition,
    /// Position of the next record (this record's `position` + its framed
    /// length). A consumer stores this as its resume cursor.
    pub next: WalPosition,
    /// Raw framed record bytes, exactly as written to the WAL.
    pub bytes: Vec<u8>,
}

/// Opaque replication-consumer identity.
///
/// Handed out by [`WalCursor::register_consumer`] and used to advance or drop
/// that consumer's retained low-watermark. Core assigns and interprets the
/// inner value; consumers treat it as opaque.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WalConsumerId(u64);

impl WalConsumerId {
    /// Constructs a consumer id from a raw value.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// The raw identity value.
    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }
}

/// Read-only, forward-only cursor over the WAL for replication consumers.
///
/// Implemented in core over the existing WAL framing; consumed by premium's
/// replication layer. Core never depends on premium (Requirement 6.5).
///
/// The read methods ([`read_from`](WalCursor::read_from),
/// [`tail_position`](WalCursor::tail_position)) are the surface defined by this
/// task; the low-watermark retention methods form the retention contract wired
/// into compaction/recovery separately.
pub trait WalCursor {
    /// Reads up to `max` records starting at `from`, in append order.
    ///
    /// Returned records are contiguous: the `next` of record *i* equals the
    /// `position` of record *i+1*, and the first record's `position` is the
    /// first framed record at or after `from`. Returns an empty vec at the
    /// live tail (nothing durable beyond `from`). A torn tail record — one
    /// truncated by a crash mid-append — is skipped and never yielded,
    /// matching the existing sequential-replay torn-tail policy in
    /// [`super::wal_entry`].
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying WAL file cannot be read.
    fn read_from(&self, from: WalPosition, max: usize) -> crate::Result<Vec<WalRecord>>;

    /// The current live append tail — the position a fresh consumer would
    /// reach after consuming everything durable so far.
    fn tail_position(&self) -> WalPosition;

    /// Registers a replication consumer, returning a handle used to advance
    /// its low-watermark. A registered consumer holds retention (see
    /// [`advance_low_watermark`](WalCursor::advance_low_watermark)).
    fn register_consumer(&self) -> WalConsumerId;

    /// Drops a previously registered consumer, releasing any retention it held.
    fn deregister_consumer(&self, consumer: WalConsumerId);

    /// Advances a consumer's low-watermark to `up_to`: the oldest position it
    /// still needs. Recovery/compaction MUST NOT discard any position
    /// `>= min(all registered low-watermarks)` (Requirement 6.3). With no
    /// registered consumer, retention behaves exactly as before this API
    /// existed.
    fn advance_low_watermark(&self, consumer: WalConsumerId, up_to: WalPosition);
}

/// In-memory low-watermark registration state for replication consumers.
///
/// Backs the retention half of [`WalCursor`]: it hands out
/// [`WalConsumerId`]s, records each consumer's retained low-watermark (the
/// oldest [`WalPosition`] it still needs), and exposes the minimum across all
/// registered consumers so compaction/recovery truncation can decide what may
/// be reclaimed (Requirement 6.3).
///
/// # Retention semantics
///
/// - A freshly registered consumer starts at [`WalPosition::START`] — it holds
///   retention over the entire durable log until it explicitly advances. A
///   consumer that only wants new records advances to the current tail right
///   after registering.
/// - Watermarks only ever move forward; a stale or out-of-order
///   [`advance`](Self::advance) call is ignored so retention can never
///   silently shrink beneath a position a consumer already passed.
/// - [`min_watermark`](Self::min_watermark) returns `None` when no consumer is
///   registered, which callers treat as "no retention hold" — truncation then
///   behaves exactly as it did before this API existed.
///
/// This is purely in-memory registration state: it is never persisted and adds
/// no on-disk artifact (Requirement 6.4). Consumer registration therefore
/// resets across a restart, by design.
#[derive(Debug, Default)]
pub struct WalWatermarkRegistry {
    /// Monotonic source of consumer ids.
    next_id: AtomicU64,
    /// `consumer -> retained low-watermark`.
    watermarks: Mutex<FxHashMap<WalConsumerId, WalPosition>>,
}

impl WalWatermarkRegistry {
    /// Creates an empty registry with no registered consumers.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a consumer, returning its opaque id.
    ///
    /// The new consumer's watermark starts at [`WalPosition::START`], so it
    /// holds retention over the whole durable log until it advances.
    pub fn register(&self) -> WalConsumerId {
        let id = WalConsumerId::new(self.next_id.fetch_add(1, Ordering::Relaxed));
        self.watermarks.lock().insert(id, WalPosition::START);
        id
    }

    /// Drops a consumer, releasing any retention it held.
    pub fn deregister(&self, consumer: WalConsumerId) {
        self.watermarks.lock().remove(&consumer);
    }

    /// Advances a consumer's low-watermark to `up_to` (monotonic — a request
    /// that would move it backwards, or that targets an unknown consumer, is
    /// ignored).
    pub fn advance(&self, consumer: WalConsumerId, up_to: WalPosition) {
        let mut guard = self.watermarks.lock();
        if let Some(current) = guard.get_mut(&consumer) {
            if up_to > *current {
                *current = up_to;
            }
        }
    }

    /// The minimum retained low-watermark across all registered consumers.
    ///
    /// Returns `None` when no consumer is registered — callers interpret that
    /// as "no retention hold" and truncate exactly as before this API existed.
    #[must_use]
    pub fn min_watermark(&self) -> Option<WalPosition> {
        self.watermarks.lock().values().copied().min()
    }
}

/// Decides whether the WAL may be fully reclaimed (truncated to empty) given
/// the registry's minimum retained watermark and the current append `tail`.
///
/// Full truncation reclaims every record below `tail`, so it is only safe when
/// no registered consumer still needs a position below `tail`:
///
/// - no consumer (`None`) → reclaim as today (Requirement 6.3 baseline);
/// - a consumer whose watermark already reached (or passed) `tail` → every
///   durable record is strictly below its watermark, so all are reclaimable;
/// - otherwise a record at or beyond the minimum watermark must survive, so
///   truncation is held (nothing is reclaimed — a subset of "records below the
///   watermark", which the contract permits).
#[must_use]
pub(crate) fn watermark_allows_full_truncation(
    min_watermark: Option<WalPosition>,
    tail: u64,
) -> bool {
    match min_watermark {
        None => true,
        Some(watermark) => watermark.offset() >= tail,
    }
}
