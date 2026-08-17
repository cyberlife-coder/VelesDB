use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use parking_lot::Mutex;

use super::{DirtyKey, MutationObserver};
use crate::MemoryError;

mod format;
mod identity;
mod io;
mod platform;

use identity::validate_epoch_id;
pub(crate) use identity::{CutoverIdentity, EpochIdentity};

use format::{
    encode_record, read_header, read_records, recover_torn_tail, scan_records, write_header,
    EncodedRecord, JournalHeader, FORMAT_VERSION,
};
pub(super) use format::{JournalRecord, RECORD_BYTES};
use io::{append_synced, write_compacted};
use platform::{durability_barrier, promote};

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
    workspace: PathBuf,
    path: PathBuf,
    max_bytes: u64,
    inner: Mutex<JournalInner>,
    #[cfg(test)]
    fault: std::sync::atomic::AtomicU8,
}

impl DirtyJournal {
    pub(crate) fn open(
        workspace: &Path,
        identity: &EpochIdentity,
        max_bytes: u64,
    ) -> Result<Self, MemoryError> {
        let path = prepare_journal(workspace, identity)?;
        let loaded = load_journal(&path)?;
        verify_identity(&loaded.header, identity)?;
        validate_capacity(loaded.header_bytes, max_bytes)?;
        Ok(Self {
            workspace: workspace.to_owned(),
            path,
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
        &self.workspace
    }

    pub(crate) fn records_after(
        &self,
        sequence: u64,
        limit: usize,
    ) -> Result<Vec<JournalRecord>, MemoryError> {
        let inner = self.inner.lock();
        ensure_healthy(&inner)?;
        let mut file =
            File::open(&self.path).map_err(|err| capture(format!("cannot read journal: {err}")))?;
        file.seek(SeekFrom::Start(inner.header_bytes))
            .map_err(|err| capture(format!("cannot seek journal: {err}")))?;
        read_records(&mut file, sequence, limit.min(MAX_READ_BATCH))
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
        let staging = self.workspace.join(STAGING_FILE);
        let header_bytes =
            write_compacted(&self.path, &staging, &next_header, watermark, |point| {
                self.maybe_fail(point)
            })?;
        self.publish_compaction(inner, &staging, next_header, header_bytes)
    }

    fn publish_compaction(
        &self,
        inner: &mut JournalInner,
        staging: &Path,
        next_header: JournalHeader,
        header_bytes: u64,
    ) -> Result<(), MemoryError> {
        self.maybe_fail(FaultPoint::BeforeCompactionReplace)?;
        inner.file.take();
        promote(staging, &self.path)
            .map_err(|err| capture(format!("cannot replace journal generation: {err}")))?;
        self.maybe_fail(FaultPoint::AfterCompactionReplace)?;
        self.maybe_fail(FaultPoint::BeforeDirectorySync)?;
        durability_barrier(&self.workspace)
            .map_err(|err| capture(format!("cannot sync journal directory: {err}")))?;
        self.maybe_fail(FaultPoint::AfterDirectorySync)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&self.path)
            .map_err(|err| capture(format!("cannot reopen compacted journal: {err}")))?;
        inner.file = Some(file);
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

    #[cfg(test)]
    fn maybe_fail(&self, point: FaultPoint) -> Result<(), MemoryError> {
        use std::sync::atomic::Ordering;
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

fn prepare_journal(workspace: &Path, identity: &EpochIdentity) -> Result<PathBuf, MemoryError> {
    validate_workspace(workspace)?;
    let path = workspace.join(JOURNAL_FILE);
    recover_staging(workspace, &path)?;
    if !path_entry_exists(&path)? {
        create_journal(workspace, identity)?;
    }
    validate_regular_file(&path)?;
    Ok(path)
}

fn load_journal(path: &Path) -> Result<LoadedJournal, MemoryError> {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|err| capture(format!("cannot open {}: {err}", path.display())))?;
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

fn create_journal(workspace: &Path, identity: &EpochIdentity) -> Result<(), MemoryError> {
    let staging = workspace.join(STAGING_FILE);
    let final_path = workspace.join(JOURNAL_FILE);
    let header = JournalHeader {
        format_version: FORMAT_VERSION,
        generation: 0,
        compacted_through: 0,
        identity: identity.clone(),
    };
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staging)
        .map_err(|err| capture(format!("cannot create journal staging file: {err}")))?;
    write_header(&mut file, &header)?;
    file.flush()
        .map_err(|err| capture(format!("cannot flush journal header: {err}")))?;
    file.sync_all()
        .map_err(|err| capture(format!("cannot sync journal header: {err}")))?;
    drop(file);
    promote(&staging, &final_path)
        .map_err(|err| capture(format!("cannot publish journal: {err}")))?;
    durability_barrier(workspace)
        .map_err(|err| capture(format!("cannot sync journal directory: {err}")))
}

fn validate_workspace(workspace: &Path) -> Result<(), MemoryError> {
    let metadata = std::fs::symlink_metadata(workspace)
        .map_err(|err| capture(format!("cannot inspect journal workspace: {err}")))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(capture("journal workspace must be a real directory"));
    }
    Ok(())
}

fn validate_regular_file(path: &Path) -> Result<(), MemoryError> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|err| capture(format!("cannot inspect journal file: {err}")))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(capture("journal path must be a regular file"));
    }
    Ok(())
}

fn recover_staging(workspace: &Path, final_path: &Path) -> Result<(), MemoryError> {
    let staging = workspace.join(STAGING_FILE);
    if !path_entry_exists(&staging)? {
        return Ok(());
    }
    if path_entry_exists(final_path)? {
        validate_regular_file(final_path)?;
    }
    validate_regular_file(&staging)?;
    std::fs::remove_file(&staging).map_err(|err| {
        capture(format!(
            "cannot remove interrupted compaction staging file: {err}"
        ))
    })
}

fn path_entry_exists(path: &Path) -> Result<bool, MemoryError> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(capture(format!(
            "cannot inspect {}: {error}",
            path.display()
        ))),
    }
}

fn ensure_healthy(inner: &JournalInner) -> Result<(), MemoryError> {
    if inner.poisoned {
        return Err(capture(
            "journal is poisoned after a durability failure; reopen it",
        ));
    }
    Ok(())
}

fn capture(message: impl Into<String>) -> MemoryError {
    MemoryError::MigrationCapture(message.into())
}
