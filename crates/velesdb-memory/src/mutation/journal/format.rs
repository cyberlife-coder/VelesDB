use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};

use sha2::{Digest, Sha256};

use super::{capture, validate_epoch_id, EpochIdentity};
use crate::mutation::DirtyKey;
use crate::MemoryError;

const MAGIC: &[u8; 8] = b"VDBDMJ01";
pub(super) const FORMAT_VERSION: u32 = 1;
pub(super) const DIGEST_BYTES: usize = 32;
pub(super) const RECORD_BODY_BYTES: usize = 17;
pub(crate) const RECORD_BYTES: u64 = (RECORD_BODY_BYTES + DIGEST_BYTES) as u64;
const MAX_HEADER_BYTES: usize = 64 * 1024;
pub(super) type EncodedRecord = [u8; RECORD_BODY_BYTES + DIGEST_BYTES];

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct JournalHeader {
    pub(super) format_version: u32,
    pub(super) generation: u64,
    pub(super) compacted_through: u64,
    pub(super) identity: EpochIdentity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct JournalRecord {
    pub(crate) sequence: u64,
    pub(crate) key: DirtyKey,
}

impl JournalRecord {
    pub(crate) fn new(sequence: u64, key: DirtyKey) -> Self {
        Self { sequence, key }
    }
}

pub(super) fn write_header(file: &mut File, header: &JournalHeader) -> Result<u64, MemoryError> {
    let body = serde_json::to_vec(header)
        .map_err(|err| capture(format!("cannot encode journal header: {err}")))?;
    let length = u32::try_from(body.len()).map_err(|_| capture("journal header is too large"))?;
    file.write_all(MAGIC)
        .and_then(|()| file.write_all(&length.to_le_bytes()))
        .and_then(|()| file.write_all(&body))
        .and_then(|()| file.write_all(&digest(&body)))
        .map_err(|err| capture(format!("cannot write journal header: {err}")))?;
    Ok(12 + u64::from(length) + DIGEST_BYTES as u64)
}

pub(super) fn read_header(file: &mut File) -> Result<(JournalHeader, u64), MemoryError> {
    file.seek(SeekFrom::Start(0))
        .map_err(|err| capture(format!("cannot seek journal header: {err}")))?;
    let mut prefix = [0_u8; 12];
    file.read_exact(&mut prefix)
        .map_err(|err| capture(format!("incomplete journal header: {err}")))?;
    if &prefix[..8] != MAGIC {
        return Err(capture("unknown journal magic or version"));
    }
    let length = header_length(&prefix)?;
    let (body, checksum) = read_header_body(file, length)?;
    let header = decode_header(&body, &checksum)?;
    Ok((header, 12 + length as u64 + DIGEST_BYTES as u64))
}

fn read_header_body(
    file: &mut File,
    length: usize,
) -> Result<(Vec<u8>, [u8; DIGEST_BYTES]), MemoryError> {
    let mut body = vec![0_u8; length];
    let mut checksum = [0_u8; DIGEST_BYTES];
    file.read_exact(&mut body)
        .and_then(|()| file.read_exact(&mut checksum))
        .map_err(|err| capture(format!("incomplete journal header: {err}")))?;
    Ok((body, checksum))
}

fn decode_header(body: &[u8], checksum: &[u8; DIGEST_BYTES]) -> Result<JournalHeader, MemoryError> {
    if digest(body) != *checksum {
        return Err(capture("journal header checksum mismatch"));
    }
    let header: JournalHeader = serde_json::from_slice(body)
        .map_err(|err| capture(format!("invalid journal header: {err}")))?;
    validate_header(&header)?;
    Ok(header)
}

fn header_length(prefix: &[u8; 12]) -> Result<usize, MemoryError> {
    let length = u32::from_le_bytes(
        prefix[8..12]
            .try_into()
            .map_err(|_| capture("invalid header length"))?,
    ) as usize;
    if length > MAX_HEADER_BYTES {
        return Err(capture("journal header exceeds safety limit"));
    }
    Ok(length)
}

fn validate_header(header: &JournalHeader) -> Result<(), MemoryError> {
    if header.format_version != FORMAT_VERSION {
        return Err(capture(format!(
            "unsupported journal version {}",
            header.format_version
        )));
    }
    validate_epoch_id(&header.identity.epoch_id)
}

pub(super) fn scan_records(
    file: &mut File,
    header: &JournalHeader,
    start: u64,
) -> Result<(u64, u64), MemoryError> {
    let length = file
        .metadata()
        .map_err(|err| capture(format!("cannot size journal: {err}")))?
        .len();
    let payload = length.saturating_sub(start);
    let complete = payload / RECORD_BYTES;
    let tail = payload % RECORD_BYTES;
    file.seek(SeekFrom::Start(start))
        .map_err(|err| capture(format!("cannot seek records: {err}")))?;
    scan_complete_records(file, header.compacted_through, start, complete, tail)
}

fn scan_complete_records(
    file: &mut File,
    compacted: u64,
    start: u64,
    complete: u64,
    tail: u64,
) -> Result<(u64, u64), MemoryError> {
    let mut expected = compacted.saturating_add(1);
    let mut bytes = [0_u8; RECORD_BODY_BYTES + DIGEST_BYTES];
    for index in 0..complete {
        file.read_exact(&mut bytes)
            .map_err(|err| capture(format!("cannot read journal record: {err}")))?;
        match decode_record(&bytes) {
            Ok(record) if record.sequence == expected => expected = expected.saturating_add(1),
            Ok(_) | Err(_) if index + 1 == complete && tail == 0 => {
                return Ok((expected.saturating_sub(1), start + index * RECORD_BYTES));
            }
            Ok(_) | Err(_) => {
                return Err(capture(format!(
                    "interior corruption at journal record {}",
                    index + 1
                )))
            }
        }
    }
    Ok((expected.saturating_sub(1), start + complete * RECORD_BYTES))
}

pub(super) fn recover_torn_tail(file: &mut File, valid_len: u64) -> Result<(), MemoryError> {
    let length = file
        .metadata()
        .map_err(|err| capture(format!("cannot size journal: {err}")))?
        .len();
    if length != valid_len {
        file.set_len(valid_len)
            .map_err(|err| capture(format!("cannot truncate torn journal tail: {err}")))?;
        file.sync_all()
            .map_err(|err| capture(format!("cannot sync recovered journal: {err}")))?;
    }
    Ok(())
}

pub(super) fn read_records(
    file: &mut File,
    after: u64,
    limit: usize,
) -> Result<Vec<JournalRecord>, MemoryError> {
    let mut records = Vec::with_capacity(limit);
    let mut bytes = [0_u8; RECORD_BODY_BYTES + DIGEST_BYTES];
    while records.len() < limit {
        match file.read_exact(&mut bytes) {
            Ok(()) => {
                let record = decode_record(&bytes)?;
                if record.sequence > after {
                    records.push(record);
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(err) => return Err(capture(format!("cannot read journal records: {err}"))),
        }
    }
    Ok(records)
}

pub(super) fn encode_record(record: JournalRecord) -> EncodedRecord {
    let mut bytes = [0_u8; RECORD_BODY_BYTES + DIGEST_BYTES];
    let (kind, id) = match record.key {
        DirtyKey::Fact(id) => (1, id),
        DirtyKey::OutgoingEdges(id) => (2, id),
    };
    bytes[0] = kind;
    bytes[1..9].copy_from_slice(&record.sequence.to_le_bytes());
    bytes[9..17].copy_from_slice(&id.to_le_bytes());
    let checksum = digest(&bytes[..RECORD_BODY_BYTES]);
    bytes[RECORD_BODY_BYTES..].copy_from_slice(&checksum);
    bytes
}

pub(super) fn decode_record(bytes: &EncodedRecord) -> Result<JournalRecord, MemoryError> {
    if digest(&bytes[..RECORD_BODY_BYTES]) != bytes[RECORD_BODY_BYTES..] {
        return Err(capture("journal record checksum mismatch"));
    }
    let sequence = u64::from_le_bytes(
        bytes[1..9]
            .try_into()
            .map_err(|_| capture("invalid record sequence"))?,
    );
    let id = u64::from_le_bytes(
        bytes[9..17]
            .try_into()
            .map_err(|_| capture("invalid record id"))?,
    );
    let key = match bytes[0] {
        1 => DirtyKey::Fact(id),
        2 => DirtyKey::OutgoingEdges(id),
        kind => return Err(capture(format!("unknown dirty-key kind {kind}"))),
    };
    Ok(JournalRecord::new(sequence, key))
}

fn digest(bytes: &[u8]) -> [u8; DIGEST_BYTES] {
    Sha256::digest(bytes).into()
}

#[cfg(test)]
mod tests {
    use std::io::{Seek, SeekFrom};

    use super::{read_header, write_header, JournalHeader};
    use crate::mutation::journal::EpochIdentity;

    #[test]
    fn future_header_version_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let identity = EpochIdentity::for_test(
            dir.path().join("source"),
            "sha256:source",
            "target",
            384,
            "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            dir.path().join("destination"),
            "00112233445566778899aabbccddeeff",
        );
        let header = JournalHeader {
            format_version: 2,
            generation: 0,
            compacted_through: 0,
            identity,
        };
        let mut file = tempfile::tempfile().expect("file");
        write_header(&mut file, &header).expect("write");
        file.seek(SeekFrom::Start(0)).expect("seek");
        let error = read_header(&mut file).expect_err("future version");
        assert!(error.to_string().contains("unsupported journal version 2"));
    }
}
