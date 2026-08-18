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
