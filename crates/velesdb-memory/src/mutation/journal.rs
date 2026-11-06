use std::fs::File;
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;
#[cfg(all(test, unix))]
use std::path::PathBuf;

use parking_lot::Mutex;

use super::atomic_file::StoreDir;
use super::{DirtyKey, MutationObserver};
use crate::migration::capture;
use crate::MemoryError;

mod format;
mod identity;
mod io;

use identity::validate_epoch_id;
pub(crate) use identity::{CutoverIdentity, EpochIdentity};

use format::{
    encode_record, read_header, read_records, recover_torn_tail, scan_records, write_header,
    EncodedRecord, JournalHeader, FORMAT_VERSION,
};
pub(super) use format::{JournalRecord, RECORD_BYTES};
use io::{append_synced, write_compacted};

pub(super) const JOURNAL_FILE: &str = "online-migration-dirty.journal";
const STAGING_FILE: &str = "online-migration-dirty.journal.tmp";
const MAX_READ_BATCH: usize = 4_096;

struct JournalInner {
    file: Option<File>,
    header: JournalHeader,
    header_bytes: u64,
    last_sequence: u64,
    poisoned: bool,
}

struct LoadedJournal {
    file: File,
    header: JournalHeader,
    header_bytes: u64,
    last_sequence: u64,
}

pub(crate) struct DirtyJournal {
    /// The workspace, pinned by an open directory descriptor at `open`
    /// (#2408): every staging/publish/sync operation resolves a bare file
    /// name against it, so a directory swapped in afterwards cannot be
    /// followed. The journal file itself resolves by name exactly once, at
    /// `open` — afterwards `inner.file` is the only handle it is reached
    /// through (#2407).
    dir: StoreDir,
    max_bytes: u64,
    inner: Mutex<JournalInner>,
    #[cfg(test)]
    fault: std::sync::atomic::AtomicU8,
    /// Test-only seam: at the given `FaultPoint`, swap the journal path for a
    /// symlink to the given victim, once, then let the caller proceed as if
    /// nothing happened — the way a real attacker's swap would land mid-flight,
    /// without also short-circuiting the code path under test the way
    /// `fail_once_at` does. Unix-only: the only test that uses it plants a
    /// symlink, which windows has no equivalent seam for.
    #[cfg(all(test, unix))]
    swap_at: Mutex<Option<(u8, PathBuf)>>,
}

impl DirtyJournal {
    pub(crate) fn open(
        workspace: &Path,
        identity: &EpochIdentity,
        max_bytes: u64,
    ) -> Result<Self, MemoryError> {
        let dir = StoreDir::open(workspace, "journal")?;
        let file = open_or_create_journal(&dir, identity)?;
        let loaded = load_journal(file)?;
        verify_identity(&loaded.header, identity)?;
        validate_capacity(loaded.header_bytes, max_bytes)?;
        Ok(Self {
            dir,
            max_bytes,
            inner: Mutex::new(JournalInner {
                file: Some(loaded.file),
                header: loaded.header,
                header_bytes: loaded.header_bytes,
                last_sequence: loaded.last_sequence,
                poisoned: false,
            }),
            #[cfg(test)]
            fault: std::sync::atomic::AtomicU8::new(0),
            #[cfg(all(test, unix))]
            swap_at: Mutex::new(None),
        })
    }

    pub(crate) fn last_sequence(&self) -> u64 {
        self.inner.lock().last_sequence
    }

    pub(crate) fn compacted_through(&self) -> u64 {
        self.inner.lock().header.compacted_through
    }

    pub(crate) fn verify_cutover_identity(
        &self,
        expected: &CutoverIdentity<'_>,
    ) -> Result<(), MemoryError> {
        let inner = self.inner.lock();
        let identity = &inner.header.identity;
        if identity.source_path() != expected.source
            || identity.destination_path() != expected.destination
            || identity.source_provenance() != expected.source_provenance
            || identity.target_model() != expected.target_model
            || identity.target_dimension() != expected.target_dimension
            || identity.target_witness() != expected.target_witness
            || identity.epoch_id() != expected.epoch_id
        {
            return Err(capture("cutover identity disagrees with journal epoch"));
        }
        Ok(())
    }

    pub(crate) fn workspace(&self) -> &Path {
        self.dir.path()
    }

    pub(crate) fn records_after(
        &self,
        sequence: u64,
        limit: usize,
    ) -> Result<Vec<JournalRecord>, MemoryError> {
        let mut inner = self.inner.lock();
        ensure_healthy(&inner)?;
        // The already-held handle, never a reopen of `JOURNAL_FILE` by name:
        // a link planted at that name after `open` intercepts nothing
        // (#2407), and a workspace swapped after `open` cannot be followed
        // through a descriptor that is already open (#2408). Appends always
        // re-seek to the end, so reading here cannot disturb the write
        // position.
        let header_bytes = inner.header_bytes;
        let file = inner
            .file
            .as_mut()
            .ok_or_else(|| capture("journal handle is closed"))?;
        file.seek(SeekFrom::Start(header_bytes))
            .map_err(|err| capture(format!("cannot seek journal: {err}")))?;
        read_records(file, sequence, limit.min(MAX_READ_BATCH))
    }

    pub(crate) fn compact_through(&self, watermark: u64) -> Result<(), MemoryError> {
        let mut inner = self.inner.lock();
        ensure_healthy(&inner)?;
        if watermark < inner.header.compacted_through || watermark > inner.last_sequence {
            return Err(capture(format!("invalid compaction watermark {watermark}")));
        }
        if watermark == inner.header.compacted_through {
            return Ok(());
        }
        let result = self.compact_locked(&mut inner, watermark);
        if result.is_err() {
            inner.poisoned = true;
        }
        result
    }

    fn compact_locked(&self, inner: &mut JournalInner, watermark: u64) -> Result<(), MemoryError> {
        let next_header = next_header(&inner.header, watermark)?;
        // The compaction reads the current journal through the descriptor
        // already held, never by name — same carry as `records_after`
        // (#2407).
        let input = inner
            .file
            .as_mut()
            .ok_or_else(|| capture("journal handle is closed"))?;
        let (compacted, header_bytes) =
            write_compacted(&self.dir, input, &next_header, watermark, |point| {
                self.maybe_fail(point)
            })?;
        self.publish_compaction(inner, compacted, next_header, header_bytes)
    }

    /// Publishes `compacted` over the journal path and keeps that same handle
    /// as the live one. Reopening the path by name afterwards would be a
    /// second lookup a hard link planted in the gap could win (#2407); a
    /// handle to the inode `promote` renamed cannot be redirected.
    fn publish_compaction(
        &self,
        inner: &mut JournalInner,
        compacted: File,
        next_header: JournalHeader,
        header_bytes: u64,
    ) -> Result<(), MemoryError> {
        self.maybe_fail(FaultPoint::BeforeCompactionReplace)?;
        inner.file.take();
        self.dir
            .promote(STAGING_FILE, JOURNAL_FILE)
            .map_err(|err| capture(format!("cannot replace journal generation: {err}")))?;
        self.maybe_fail(FaultPoint::AfterCompactionReplace)?;
        self.maybe_fail(FaultPoint::BeforeDirectorySync)?;
        self.dir
            .sync()
            .map_err(|err| capture(format!("cannot sync journal directory: {err}")))?;
        self.maybe_fail(FaultPoint::AfterDirectorySync)?;
        inner.file = Some(compacted);
        inner.header = next_header;
        inner.header_bytes = header_bytes;
        Ok(())
    }

    fn append(&self, key: DirtyKey) -> Result<(), MemoryError> {
        let mut inner = self.inner.lock();
        ensure_healthy(&inner)?;
        let (sequence, record) = prepare_append(&inner, self.max_bytes, key)?;
        let result = self.append_locked(&mut inner, &record);
        if let Err(error) = result {
            inner.poisoned = true;
            return Err(error);
        }
        inner.last_sequence = sequence;
        Ok(())
    }

    fn append_locked(&self, inner: &mut JournalInner, record: &[u8]) -> Result<(), MemoryError> {
        let file = inner
            .file
            .as_mut()
            .ok_or_else(|| capture("journal handle is closed"))?;
        append_synced(file, record, |point| self.maybe_fail(point))
    }

    #[cfg(test)]
    pub(super) fn header_bytes(&self) -> u64 {
        self.inner.lock().header_bytes
    }

    #[cfg(test)]
    pub(super) fn fail_once_at(&self, point: FaultPoint) {
        use std::sync::atomic::Ordering;
        self.fault.store(point as u8, Ordering::SeqCst);
    }

    /// Swaps the journal path for a symlink to `victim` the next time
    /// execution reaches `point`, then lets the caller proceed normally —
    /// unlike `fail_once_at`, which returns early instead of reaching the
    /// code the swap is meant to land in front of.
    #[cfg(all(test, unix))]
    pub(super) fn swap_once_at(&self, point: FaultPoint, victim: PathBuf) {
        *self.swap_at.lock() = Some((point as u8, victim));
    }

    #[cfg(all(test, unix))]
    fn apply_pending_swap(&self, point: FaultPoint) -> Result<(), MemoryError> {
        let mut swap_at = self.swap_at.lock();
        if swap_at.as_ref().is_some_and(|(at, _)| *at == point as u8) {
            let (_, victim) = swap_at.take().expect("checked above");
            let journal_path = self.dir.path().join(JOURNAL_FILE);
            std::fs::remove_file(&journal_path)
                .map_err(|err| capture(format!("test: cannot remove journal path: {err}")))?;
            std::os::unix::fs::symlink(&victim, &journal_path)
                .map_err(|err| capture(format!("test: cannot plant swap symlink: {err}")))?;
        }
        Ok(())
    }

    #[cfg(all(test, not(unix)))]
    #[allow(clippy::unnecessary_wraps, clippy::unused_self)] // Mirrors the unix seam exactly.
    fn apply_pending_swap(&self, _point: FaultPoint) -> Result<(), MemoryError> {
        Ok(())
    }

    #[cfg(test)]
    fn maybe_fail(&self, point: FaultPoint) -> Result<(), MemoryError> {
        use std::sync::atomic::Ordering;
        self.apply_pending_swap(point)?;
        if self
            .fault
            .compare_exchange(point as u8, 0, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            return Err(capture(format!("injected journal fault at {point:?}")));
        }
        Ok(())
    }

    #[cfg(not(test))]
    #[allow(clippy::unnecessary_wraps, clippy::unused_self)] // Mirrors the test fault seam exactly.
    fn maybe_fail(&self, _point: FaultPoint) -> Result<(), MemoryError> {
        Ok(())
    }
}

impl MutationObserver for DirtyJournal {
    fn before_mutation(&self, key: DirtyKey) -> Result<(), MemoryError> {
        self.append(key)
    }
}

#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub(super) enum FaultPoint {
    BeforeAppend = 1,
    AfterAppend = 2,
    BeforeAppendSync = 3,
    AfterAppendSync = 4,
    BeforeCompactionSync = 5,
    AfterCompactionSync = 6,
    BeforeCompactionReplace = 7,
    AfterCompactionReplace = 8,
    BeforeDirectorySync = 9,
    AfterDirectorySync = 10,
}

fn open_or_create_journal(dir: &StoreDir, identity: &EpochIdentity) -> Result<File, MemoryError> {
    recover_staging(dir)?;
    if dir.exists(JOURNAL_FILE)? {
        // `StoreDir::open_regular_file` checks the handle it just opened —
        // not a separate `stat` of the name — so no `validate_regular` step
        // precedes it: a second check would only restate the same race.
        return dir.open_regular_file(JOURNAL_FILE, "journal");
    }
    create_journal(dir, identity)
}

fn load_journal(mut file: File) -> Result<LoadedJournal, MemoryError> {
    file.seek(SeekFrom::Start(0))
        .map_err(|err| capture(format!("cannot seek journal: {err}")))?;
    let (header, header_bytes) = read_header(&mut file)?;
    let (last_sequence, valid_len) = scan_records(&mut file, &header, header_bytes)?;
    recover_torn_tail(&mut file, valid_len)?;
    Ok(LoadedJournal {
        file,
        header,
        header_bytes,
        last_sequence,
    })
}

fn verify_identity(header: &JournalHeader, identity: &EpochIdentity) -> Result<(), MemoryError> {
    if header.identity != *identity {
        return Err(capture("journal epoch identity mismatch"));
    }
    Ok(())
}

fn validate_capacity(header_bytes: u64, max_bytes: u64) -> Result<(), MemoryError> {
    if max_bytes < header_bytes + RECORD_BYTES {
        return Err(capture("journal byte cap cannot hold one record"));
    }
    Ok(())
}

fn next_header(header: &JournalHeader, watermark: u64) -> Result<JournalHeader, MemoryError> {
    let generation = header
        .generation
        .checked_add(1)
        .ok_or_else(|| capture("journal generation overflow"))?;
    Ok(JournalHeader {
        format_version: FORMAT_VERSION,
        generation,
        compacted_through: watermark,
        identity: header.identity.clone(),
    })
}

fn next_sequence(last: u64) -> Result<u64, MemoryError> {
    last.checked_add(1)
        .ok_or_else(|| capture("journal sequence overflow"))
}

fn prepare_append(
    inner: &JournalInner,
    max_bytes: u64,
    key: DirtyKey,
) -> Result<(u64, EncodedRecord), MemoryError> {
    let sequence = next_sequence(inner.last_sequence)?;
    let file = inner
        .file
        .as_ref()
        .ok_or_else(|| capture("journal handle is closed"))?;
    ensure_append_fits(file, max_bytes)?;
    Ok((sequence, encode_record(JournalRecord::new(sequence, key))))
}

fn ensure_append_fits(file: &File, max_bytes: u64) -> Result<(), MemoryError> {
    let length = file
        .metadata()
        .map_err(|err| capture(format!("cannot size journal: {err}")))?
        .len();
    if length.saturating_add(RECORD_BYTES) > max_bytes {
        return Err(capture(format!("journal byte cap {max_bytes} reached")));
    }
    Ok(())
}

fn create_journal(dir: &StoreDir, identity: &EpochIdentity) -> Result<File, MemoryError> {
    let header = JournalHeader {
        format_version: FORMAT_VERSION,
        generation: 0,
        compacted_through: 0,
        identity: identity.clone(),
    };
    let mut file = dir
        .create_new(STAGING_FILE, 0o666)
        .map_err(|err| capture(format!("cannot create journal staging file: {err}")))?;
    write_header(&mut file, &header)?;
    file.flush()
        .map_err(|err| capture(format!("cannot flush journal header: {err}")))?;
    file.sync_all()
        .map_err(|err| capture(format!("cannot sync journal header: {err}")))?;
    // The staging handle IS the journal inode: `promote` renames it in
    // place, so the handle stays live and nothing reopens the name
    // afterwards (#2407).
    dir.promote(STAGING_FILE, JOURNAL_FILE)
        .map_err(|err| capture(format!("cannot publish journal: {err}")))?;
    dir.sync()
        .map_err(|err| capture(format!("cannot sync journal directory: {err}")))?;
    Ok(file)
}

fn recover_staging(dir: &StoreDir) -> Result<(), MemoryError> {
    if !dir.exists(STAGING_FILE)? {
        return Ok(());
    }
    if dir.exists(JOURNAL_FILE)? {
        dir.validate_regular(JOURNAL_FILE, "journal")?;
    }
    dir.validate_regular(STAGING_FILE, "journal")?;
    dir.remove(STAGING_FILE).map_err(|err| {
        capture(format!(
            "cannot remove interrupted compaction staging file: {err}"
        ))
    })
}

fn ensure_healthy(inner: &JournalInner) -> Result<(), MemoryError> {
    if inner.poisoned {
        return Err(capture(
            "journal is poisoned after a durability failure; reopen it",
        ));
    }
    Ok(())
}
