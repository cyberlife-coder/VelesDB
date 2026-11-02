use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};

use super::format::{
    decode_record, read_header, write_header, EncodedRecord, JournalHeader, DIGEST_BYTES,
    RECORD_BODY_BYTES,
};
use super::FaultPoint;
use crate::migration::capture;
use crate::mutation::atomic_file::StoreDir;
use crate::MemoryError;

pub(super) fn append_synced<F>(file: &mut File, record: &[u8], fault: F) -> Result<(), MemoryError>
where
    F: Fn(FaultPoint) -> Result<(), MemoryError>,
{
    write_append(file, record, &fault)?;
    sync_append(file, &fault)
}

fn write_append<F>(file: &mut File, record: &[u8], fault: &F) -> Result<(), MemoryError>
where
    F: Fn(FaultPoint) -> Result<(), MemoryError>,
{
    fault(FaultPoint::BeforeAppend)?;
    file.seek(SeekFrom::End(0))
        .map_err(|err| capture(format!("cannot seek journal: {err}")))?;
    file.write_all(record)
        .map_err(|err| capture(format!("cannot append journal: {err}")))?;
    fault(FaultPoint::AfterAppend)?;
    file.flush()
        .map_err(|err| capture(format!("cannot flush journal: {err}")))
}

fn sync_append<F>(file: &File, fault: &F) -> Result<(), MemoryError>
where
    F: Fn(FaultPoint) -> Result<(), MemoryError>,
{
    fault(FaultPoint::BeforeAppendSync)?;
    file.sync_all()
        .map_err(|err| capture(format!("cannot sync journal: {err}")))?;
    fault(FaultPoint::AfterAppendSync)
}

/// Writes the compacted journal to the staging name inside `dir` and returns
/// its handle. `input` is the *already-held* journal handle, not a reopen of
/// the journal name — the file this compaction reads is the inode the caller
/// holds, whatever a planted link at the name would resolve to (#2407).
pub(super) fn write_compacted<F>(
    dir: &StoreDir,
    input: &mut File,
    header: &JournalHeader,
    watermark: u64,
    fault: F,
) -> Result<(File, u64), MemoryError>
where
    F: Fn(FaultPoint) -> Result<(), MemoryError>,
{
    rewind_record_stream(input)?;
    let mut output = create_staging(dir)?;
    let header_bytes = write_header(&mut output, header)?;
    copy_records_after(input, &mut output, watermark)?;
    sync_compacted(&mut output, &fault)?;
    Ok((output, header_bytes))
}

fn sync_compacted<F>(output: &mut File, fault: &F) -> Result<(), MemoryError>
where
    F: Fn(FaultPoint) -> Result<(), MemoryError>,
{
    output
        .flush()
        .map_err(|err| capture(format!("cannot flush compacted journal: {err}")))?;
    fault(FaultPoint::BeforeCompactionSync)?;
    output
        .sync_all()
        .map_err(|err| capture(format!("cannot sync compacted journal: {err}")))?;
    fault(FaultPoint::AfterCompactionSync)
}

fn rewind_record_stream(input: &mut File) -> Result<(), MemoryError> {
    input
        .seek(SeekFrom::Start(0))
        .map_err(|err| capture(format!("cannot seek journal: {err}")))?;
    let (_, header_bytes) = read_header(input)?;
    input
        .seek(SeekFrom::Start(header_bytes))
        .map_err(|err| capture(format!("cannot seek journal: {err}")))?;
    Ok(())
}

fn create_staging(dir: &StoreDir) -> Result<File, MemoryError> {
    dir.create_new(super::STAGING_FILE, 0o666)
        .map_err(|err| capture(format!("cannot create compaction staging file: {err}")))
}

fn copy_records_after(
    input: &mut File,
    output: &mut File,
    watermark: u64,
) -> Result<(), MemoryError> {
    let mut bytes = [0_u8; RECORD_BODY_BYTES + DIGEST_BYTES];
    loop {
        match input.read_exact(&mut bytes) {
            Ok(()) => copy_if_pending(output, &bytes, watermark)?,
            Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(err) => return Err(capture(format!("cannot stream journal: {err}"))),
        }
    }
}

fn copy_if_pending(
    output: &mut File,
    bytes: &EncodedRecord,
    watermark: u64,
) -> Result<(), MemoryError> {
    if decode_record(bytes)?.sequence <= watermark {
        return Ok(());
    }
    output
        .write_all(bytes)
        .map_err(|err| capture(format!("cannot write compacted journal: {err}")))
}
